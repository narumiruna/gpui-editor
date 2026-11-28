use std::{collections::VecDeque, iter::FusedIterator, ops::Range, str::CharIndices, sync::Arc};

use unicode_normalization::UnicodeNormalization;

use super::{CursorOptions, SearchError, SearchMatch, check_range, filter::MatchFilter};
use crate::{Text, TextCursor, language::line_projection::splits_crlf};

#[derive(Clone, Debug)]
pub(super) struct LiteralPattern {
    data: Arc<PatternData>,
    ignore_case: bool,
    pub(super) units: usize,
}

#[derive(Debug)]
struct PatternData {
    query: Vec<char>,
    fallback: Vec<usize>,
}

impl LiteralPattern {
    pub(super) fn new(query: &str, ignore_case: bool) -> Self {
        let units = query.encode_utf16().count();
        let normalized: String = query.nfkd().collect();
        let query: Vec<_> = if ignore_case {
            normalized.to_lowercase().chars().collect()
        } else {
            normalized.chars().collect()
        };
        let mut fallback = vec![0; query.len()];
        let mut matched = 0;
        for index in 1..query.len() {
            matched = advance(&query, &fallback, matched, query[index]);
            fallback[index] = matched;
        }
        Self {
            data: Arc::new(PatternData { query, fallback }),
            ignore_case,
            units,
        }
    }
}

/// An iterator over literal matches. The query uses NFKD normalization as
/// a string. The document is normalized one scalar at a time, as in 6.7.3.
/// Matches always cover whole source scalars even when `precise` is false.
pub struct SearchCursor<'a> {
    source: &'a str,
    base: usize,
    chars: CharIndices<'a>,
    pattern: LiteralPattern,
    filter: MatchFilter,
    fragments: Option<Fragments<'a>>,
    logical_lines: bool,
    matched: usize,
    starts: VecDeque<(usize, bool)>,
}

struct Fragments<'a> {
    cursor: TextCursor<'a>,
    current: Range<usize>,
}

impl Fragments<'_> {
    fn ending_at(&mut self, end: usize) -> Range<usize> {
        // The completing scalar belongs to this fragment even if it ends
        // exactly at the fragment's right edge.
        while self.current.end < end {
            let start = self.current.end;
            let chunk = self.cursor.next().expect("match within scan range");
            self.current = start..start + chunk.len();
        }
        self.current.clone()
    }
}

impl<'a> SearchCursor<'a> {
    /// Search the entire document with case-sensitive matching.
    pub fn new(text: &'a Text, query: &str) -> Self {
        Self::with_pattern(
            text,
            0..text.len(),
            LiteralPattern::new(query, false),
            false,
        )
    }

    pub fn in_range(
        text: &'a Text,
        query: &str,
        range: Range<usize>,
        options: CursorOptions,
    ) -> Result<Self, SearchError> {
        check_range(text, &range)?;
        Ok(Self::with_pattern(
            text,
            range,
            LiteralPattern::new(query, options.ignore_case),
            options.whole_word,
        ))
    }

    pub(super) fn with_pattern(
        text: &'a Text,
        range: Range<usize>,
        pattern: LiteralPattern,
        whole_word: bool,
    ) -> Self {
        Self::with_filter(text, range, pattern, MatchFilter::new(whole_word))
    }

    pub(super) fn with_filter(
        text: &'a Text,
        range: Range<usize>,
        pattern: LiteralPattern,
        filter: MatchFilter,
    ) -> Self {
        let base = range.start;
        let source = text.as_str();
        let fragments = filter.has_whole_word().then(|| Fragments {
            cursor: text
                .cursor_range(base, range.end)
                .expect("valid scan range"),
            current: base..base,
        });
        Self {
            source,
            base,
            chars: source[range].char_indices(),
            pattern,
            filter,
            fragments,
            logical_lines: false,
            matched: 0,
            starts: VecDeque::new(),
        }
    }

    pub(super) fn selected_text(
        text: &'a Text,
        mut range: Range<usize>,
        pattern: LiteralPattern,
        logical_lines: bool,
    ) -> Self {
        if logical_lines {
            range.start += usize::from(splits_crlf(text.as_str(), range.start));
            range.end -= usize::from(splits_crlf(text.as_str(), range.end));
            range.end = range.end.max(range.start);
        }
        let mut cursor = Self::with_pattern(text, range, pattern, false);
        cursor.logical_lines = logical_lines;
        cursor
    }

    /// Find the next match, including matches that overlap the preceding one.
    pub fn next_overlapping(&mut self) -> Option<SearchMatch<'a>> {
        if self.pattern.data.query.is_empty() {
            return None;
        }
        while let Some((offset, source_char)) = self.chars.next() {
            let start = self.base + offset;
            let mut end = start + source_char.len_utf8();
            let source_char = if source_char == '\r' && self.logical_lines {
                if self.chars.as_str().starts_with('\n') {
                    self.chars.next();
                    end += 1;
                }
                '\n'
            } else {
                source_char
            };
            if source_char.is_ascii() {
                let ch = if self.pattern.ignore_case {
                    source_char.to_ascii_lowercase()
                } else {
                    source_char
                };
                if let Some(found) = self.step(ch, start..end, true, true) {
                    return Some(found);
                }
                continue;
            }
            let mut buf = [0; 4];
            let norm = source_char.encode_utf8(&mut buf).nfkd();
            let found = if self.pattern.ignore_case {
                self.match_scalar(start..end, norm.flat_map(char::to_lowercase))
            } else {
                self.match_scalar(start..end, norm)
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn match_scalar(
        &mut self,
        range: Range<usize>,
        normalized: impl Iterator<Item = char>,
    ) -> Option<SearchMatch<'a>> {
        let mut chars = normalized.peekable();
        let mut at_start = true;
        while let Some(ch) = chars.next() {
            if let Some(found) = self.step(ch, range.clone(), at_start, chars.peek().is_none()) {
                // Upstream advances the source cursor before normalization.
                // On a match it does not revisit the rest of this scalar.
                return Some(found);
            }
            at_start = false;
        }
        None
    }

    fn step(
        &mut self,
        ch: char,
        range: Range<usize>,
        precise_start: bool,
        precise_end: bool,
    ) -> Option<SearchMatch<'a>> {
        let PatternData { query, fallback } = &*self.pattern.data;
        if self.starts.len() == query.len() {
            self.starts.pop_front();
        }
        self.starts.push_back((range.start, precise_start));
        self.matched = advance(query, fallback, self.matched, ch);
        if self.matched != query.len() {
            return None;
        }
        self.matched = fallback[self.matched - 1];
        let &(start, precise_start) = self.starts.front()?;
        let range = start..range.end;
        if let Some(fragments) = self.fragments.as_mut()
            && !self.filter.matches_literal_word(
                self.source,
                range.clone(),
                fragments.ending_at(range.end),
            )
        {
            return None;
        }
        let found = SearchMatch {
            text: &self.source[range.clone()],
            range,
            precise: precise_start && precise_end,
            captures: Vec::new(),
        };
        self.filter.accepts(&found).then_some(found)
    }
}

impl<'a> Iterator for SearchCursor<'a> {
    type Item = SearchMatch<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.matched = 0;
        self.next_overlapping()
    }
}

impl FusedIterator for SearchCursor<'_> {}

fn advance(query: &[char], fallback: &[usize], mut matched: usize, ch: char) -> usize {
    while matched > 0 && query[matched] != ch {
        matched = fallback[matched - 1];
    }
    matched + usize::from(query[matched] == ch)
}
