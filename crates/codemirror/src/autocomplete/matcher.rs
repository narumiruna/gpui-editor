use std::{iter, ops::Range};

const GAP: i64 = -1100;
const NOT_START: i64 = -700;
const CASE_FOLD: i64 = -200;
const BY_WORD: i64 = -100;
const NOT_FULL: i64 = -100;
const SCAN_LIMIT: usize = 200;

/// How a completion label must match the input.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MatchMode {
    #[default]
    Fuzzy,
    Strict,
}

/// The score and matching byte ranges in a completion label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionMatch {
    /// Higher scores indicate better matches.
    pub score: i64,
    pub ranges: Vec<Range<usize>>,
}

/// A reusable completion pattern. Strict matching only accepts prefixes.
/// Fuzzy matching also accepts substrings, word initials, and some split matches.
#[derive(Debug)]
pub struct CompletionMatcher<'a> {
    pattern: &'a str,
    units: usize,
    prepared: Pattern,
}

#[derive(Debug)]
enum Pattern {
    Fuzzy(Vec<(char, char)>),
    Strict(String),
}

impl<'a> CompletionMatcher<'a> {
    pub fn new(pattern: &'a str, mode: MatchMode) -> Self {
        let prepared = match mode {
            MatchMode::Fuzzy => Pattern::Fuzzy(pattern.chars().map(|ch| (ch, fold(ch))).collect()),
            MatchMode::Strict => Pattern::Strict(pattern.to_lowercase()),
        };
        Self {
            pattern,
            units: pattern.encode_utf16().count(),
            prepared,
        }
    }

    /// Match one label independently. Scores use UTF-16 length units, but
    /// highlight ranges cover complete UTF-8 scalars.
    pub fn match_label(&self, label: &str) -> Option<CompletionMatch> {
        self.match_in_source(label, &mut false)
    }

    pub(super) fn match_in_source(
        &self,
        label: &str,
        previous_word_match: &mut bool,
    ) -> Option<CompletionMatch> {
        match &self.prepared {
            Pattern::Fuzzy(chars) => self.fuzzy(label, chars, previous_word_match),
            Pattern::Strict(folded) => self.strict(label, folded),
        }
    }

    fn strict(&self, label: &str, folded: &str) -> Option<CompletionMatch> {
        let mut units = 0;
        let mut end = 0;
        for ch in label.chars() {
            if units >= self.units {
                break;
            }
            units += ch.len_utf16();
            end += ch.len_utf8();
        }
        if units != self.units {
            return None;
        }
        let prefix = &label[..end];
        let score = if prefix == self.pattern {
            0
        } else if prefix.to_lowercase() == folded {
            CASE_FOLD
        } else {
            return None;
        };
        Some(one_range(score + prefix_penalty(end, label), 0..end))
    }

    fn fuzzy(
        &self,
        label: &str,
        chars: &[(char, char)],
        previous_word_match: &mut bool,
    ) -> Option<CompletionMatch> {
        if chars.is_empty() {
            return Some(CompletionMatch {
                score: NOT_FULL,
                ranges: Vec::new(),
            });
        }
        let word_units = label.encode_utf16().count();
        if word_units < self.units {
            return None;
        }
        if let [(exact, folded)] = chars {
            let first = label.chars().next()?;
            let score = if first == *exact {
                0
            } else if first == *folded {
                CASE_FOLD
            } else {
                return None;
            };
            let end = first.len_utf8();
            return Some(one_range(score + prefix_penalty(end, label), 0..end));
        }
        let direct = label.find(self.pattern);
        if direct == Some(0) {
            let end = self.pattern.len();
            return Some(one_range(prefix_penalty(end, label), 0..end));
        }
        let has_lower = label.bytes().any(|byte| byte.is_ascii_lowercase());
        let mut any = Vec::new();
        let mut by_word = Vec::new();
        let mut by_word_folded = false;
        let mut word_adjacent = true;
        let mut adjacent_count = 0;
        let mut adjacent = 0..0;
        let mut adjacent_end_units = 0;
        let mut prev_type = WordType::Other;
        let mut scanned = 0;
        for (at, ch) in label.char_indices() {
            if scanned >= SCAN_LIMIT {
                break;
            }
            scanned += ch.len_utf16();
            let end = at + ch.len_utf8();
            if direct.is_none() {
                if chars.get(any.len()).is_some_and(|&pair| hits(ch, pair)) {
                    any.push(at..end);
                }
                if let Some(&pair) = chars.get(adjacent_count) {
                    if hits(ch, pair) {
                        if adjacent_count == 0 {
                            adjacent.start = at;
                        }
                        adjacent.end = end;
                        // Upstream stores the last matched position plus one UTF-16
                        // unit, even for an astral character. Expand the highlight,
                        // but keep the original end for the score.
                        adjacent_end_units = scanned - ch.len_utf16() + 1;
                        adjacent_count += 1;
                    } else {
                        adjacent_count = 0;
                    }
                }
            }
            let kind = word_type(ch);
            if by_word.len() < chars.len()
                && (at == 0
                    || (kind == WordType::Upper && has_lower)
                    || (prev_type == WordType::Other && kind != WordType::Other))
            {
                let pair = chars[by_word.len()];
                if hits(ch, pair) {
                    by_word.push(at..end);
                    by_word_folded |= ch != pair.0;
                    if by_word.len() == chars.len() {
                        break;
                    }
                } else if *previous_word_match || !by_word.is_empty() {
                    word_adjacent = false;
                }
            }
            prev_type = kind;
        }
        if direct.is_none() && any.len() < chars.len() {
            return None;
        }
        // Upstream's word scan only runs after a match is possible. It tests
        // the reused buffer's length, not the number of current matches.
        *previous_word_match |= !by_word.is_empty();
        let length = i64::try_from(word_units).ok()?;
        let by_word_score = BY_WORD + if by_word_folded { CASE_FOLD } else { 0 };
        if by_word.len() == chars.len() && by_word[0].start == 0 && word_adjacent {
            Some(coalesce(by_word_score - length, by_word))
        } else if adjacent_count == chars.len() && adjacent.start == 0 {
            let penalty = if adjacent_end_units == word_units {
                0
            } else {
                NOT_FULL
            };
            Some(one_range(CASE_FOLD - length + penalty, adjacent))
        } else if let Some(at) = direct {
            Some(one_range(NOT_START - length, at..at + self.pattern.len()))
        } else if adjacent_count == chars.len() {
            Some(one_range(CASE_FOLD + NOT_START - length, adjacent))
        } else if by_word.len() == chars.len() {
            let gap = if word_adjacent { 0 } else { GAP };
            Some(coalesce(by_word_score + NOT_START + gap - length, by_word))
        } else if chars.len() > 2 {
            let start = if any[0].start == 0 { 0 } else { NOT_START };
            Some(coalesce(start + CASE_FOLD + GAP - length, any))
        } else {
            None
        }
    }
}

fn prefix_penalty(end: usize, label: &str) -> i64 {
    if end == label.len() { 0 } else { NOT_FULL }
}

fn one_range(score: i64, range: Range<usize>) -> CompletionMatch {
    CompletionMatch {
        score,
        ranges: iter::once(range).collect(),
    }
}

fn coalesce(score: i64, ranges: Vec<Range<usize>>) -> CompletionMatch {
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && last.end == range.start
        {
            last.end = range.end;
        } else {
            merged.push(range);
        }
    }
    CompletionMatch {
        score,
        ranges: merged,
    }
}

fn hits(ch: char, (exact, folded): (char, char)) -> bool {
    ch == exact || ch == folded
}

fn fold(ch: char) -> char {
    let mut upper = ch.to_uppercase();
    let first = upper.next().unwrap_or(ch);
    if first == ch && upper.next().is_none() {
        ch.to_lowercase().next().unwrap_or(ch)
    } else {
        first
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WordType {
    Other,
    Upper,
    Lower,
}

fn word_type(ch: char) -> WordType {
    if ch.is_ascii_uppercase() || (ch >= '\u{ff}' && !ch.to_lowercase().eq(iter::once(ch))) {
        WordType::Upper
    } else if ch.is_ascii_lowercase()
        || ch.is_ascii_digit()
        || (ch >= '\u{ff}' && !ch.to_uppercase().eq(iter::once(ch)))
    {
        WordType::Lower
    } else {
        WordType::Other
    }
}
