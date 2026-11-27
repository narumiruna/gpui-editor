use std::{cmp::Ordering, collections::HashMap, ops::Range};

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::{
    Change, ChangeSet, EditError, EditorState, RangeEdit, SelectionRange, Text, Transaction,
};

use super::matcher::{CompletionMatcher, MatchMode};

/// The text and metadata for one completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Completion<'a> {
    pub label: &'a str,
    /// The label the caller displays, if different from the text to match.
    pub display_label: Option<&'a str>,
    pub sort_text: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub info: Option<&'a str>,
    /// Text to insert. If absent, insert `label`.
    pub insert: Option<&'a str>,
    /// Adjust the score for a filtered match. Use values from -99 through 99.
    pub boost: i32,
    pub section: Option<CompletionSection<'a>>,
}

impl<'a> Completion<'a> {
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            display_label: None,
            sort_text: None,
            detail: None,
            kind: None,
            info: None,
            insert: None,
            boost: 0,
            section: None,
        }
    }

    fn sort_key(self) -> &'a str {
        self.sort_text
            .filter(|text| !text.is_empty())
            .unwrap_or(self.label)
    }

    fn specificity(self) -> u8 {
        u8::from(self.insert.is_some_and(|text| !text.is_empty())) * 10
            + u8::from(self.info.is_some_and(|text| !text.is_empty())) * 5
            + u8::from(self.kind.is_some_and(|text| !text.is_empty()))
    }

    fn same_option(self, other: Self) -> bool {
        self.label == other.label
            && self.detail == other.detail
            && (self.kind == other.kind || self.kind.is_none() || other.kind.is_none())
            && self.insert == other.insert
            && self.boost == other.boost
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionSection<'a> {
    pub name: &'a str,
    pub rank: SectionRank,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SectionRank {
    /// Sort explicit ranks from lowest to highest.
    Fixed(i32),
    /// Compare two dynamic sections by their best matching scores.
    Dynamic,
    /// Use the section name when the ranks do not determine the order.
    #[default]
    ByName,
}

/// Candidates from one source and the document range used to filter them.
#[derive(Clone, Debug)]
pub struct CompletionResult<'a> {
    pub range: Range<usize>,
    pub options: &'a [Completion<'a>],
    /// When false, keep these options in source order above filtered options
    /// (subject to section scores) and do not compute highlights.
    pub filter: bool,
}

/// A completion after filtering and sorting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankedCompletion<'a> {
    pub completion: &'a Completion<'a>,
    /// Document range from the source.
    pub range: Range<usize>,
    /// Matching ranges in `label`, or in the display label when a source
    /// provides them.
    pub match_ranges: Vec<Range<usize>>,
    pub score: i64,
}

impl RankedCompletion<'_> {
    /// Build a transaction to apply this text completion to the current document.
    pub fn transaction(&self, state: &EditorState) -> Result<Transaction, EditError> {
        let completion = self.completion;
        insert_completion_text(
            state,
            self.range.clone(),
            completion.insert.unwrap_or(completion.label),
        )
    }
}

/// Filter and sort completion results. The default tie order follows common
/// English identifier collation and retains source order for equal keys. It
/// does not use the host locale. Use [`rank_completions_by`] for other orders.
pub fn rank_completions<'a>(
    text: &Text,
    results: &[CompletionResult<'a>],
    mode: MatchMode,
) -> Result<Vec<RankedCompletion<'a>>, EditError> {
    rank_completions_by(text, results, mode, compare_completions)
}

/// Filter and sort results, using `compare` to break score ties. Adjacent
/// duplicates are collapsed, with preference for more specific metadata.
pub fn rank_completions_by<'a>(
    text: &Text,
    results: &[CompletionResult<'a>],
    mode: MatchMode,
    compare: impl FnMut(&Completion<'a>, &Completion<'a>) -> Ordering,
) -> Result<Vec<RankedCompletion<'a>>, EditError> {
    rank(text, results, mode, compare, |_, _| Vec::new())
}

pub(super) fn rank<'a>(
    text: &Text,
    results: &[CompletionResult<'a>],
    mode: MatchMode,
    mut compare: impl FnMut(&Completion<'a>, &Completion<'a>) -> Ordering,
    mut highlights: impl FnMut(&Completion<'a>, Option<&[Range<usize>]>) -> Vec<Range<usize>>,
) -> Result<Vec<RankedCompletion<'a>>, EditError> {
    let mut options = Vec::new();
    let mut section_indices = HashMap::new();
    let mut sections = Vec::new();
    for result in results {
        let query = text
            .slice(result.range.clone())
            .ok_or(EditError::InvalidRange)?;
        let matcher = result.filter.then(|| CompletionMatcher::new(query, mode));
        let mut previous_word_match = false;
        for completion in result.options {
            let (score, match_ranges) = if let Some(matcher) = &matcher {
                let Some(found) =
                    matcher.match_in_source(completion.label, &mut previous_word_match)
                else {
                    continue;
                };
                let ranges = if completion.display_label.is_none_or(str::is_empty) {
                    found.ranges
                } else {
                    highlights(completion, Some(&found.ranges))
                };
                (found.score + i64::from(completion.boost), ranges)
            } else {
                (
                    1_000_000_000 - options.len() as i64,
                    highlights(completion, None),
                )
            };
            if let Some(section) = completion.section {
                let index = *section_indices.entry(section.name).or_insert_with(|| {
                    let index = sections.len();
                    sections.push(SectionScore {
                        section,
                        best: None,
                    });
                    index
                });
                if result.filter && section.rank == SectionRank::Dynamic {
                    let best = &mut sections[index].best;
                    // The upstream fallback applies to a missing or zero score.
                    *best =
                        Some(score.max(best.filter(|&value| value != 0).unwrap_or(-1_000_000_000)));
                }
            }
            options.push(RankedCompletion {
                completion,
                range: result.range.clone(),
                match_ranges,
                score,
            });
        }
    }
    if !sections.is_empty() {
        sort_sections(&mut sections);
        for (index, section) in sections.iter().enumerate() {
            section_indices.insert(section.section.name, index);
        }
        for option in &mut options {
            if let Some(section) = option.completion.section {
                option.score -= (section_indices[section.name] as i64 + 1) * 100_000;
            }
        }
    }
    options.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| compare(a.completion, b.completion))
    });
    let mut ranked = Vec::new();
    let mut previous: Option<&Completion<'_>> = None;
    for option in options {
        if let Some(prev) = previous.filter(|prev| prev.same_option(*option.completion)) {
            previous = Some(option.completion);
            if option.completion.specificity() > prev.specificity()
                && let Some(last) = ranked.last_mut()
            {
                *last = option;
            }
        } else {
            previous = Some(option.completion);
            ranked.push(option);
        }
    }
    Ok(ranked)
}

pub(super) fn compare_completions(a: &Completion<'_>, b: &Completion<'_>) -> Ordering {
    compare_sort_keys(a.sort_key(), b.sort_key())
}

fn compare_sort_keys(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    if a.is_ascii() && b.is_ascii() {
        return a
            .chars()
            .map(primary_weight)
            .cmp(b.chars().map(primary_weight))
            .then_with(|| {
                a.bytes()
                    .map(|byte| byte.is_ascii_uppercase())
                    .cmp(b.bytes().map(|byte| byte.is_ascii_uppercase()))
            });
    }
    let primary = |ch: &char| !is_combining_mark(*ch);
    a.nfd()
        .filter(primary)
        .flat_map(char::to_lowercase)
        .map(primary_weight)
        .cmp(
            b.nfd()
                .filter(primary)
                .flat_map(char::to_lowercase)
                .map(primary_weight),
        )
        .then_with(|| {
            a.nfd()
                .flat_map(char::to_lowercase)
                .cmp(b.nfd().flat_map(char::to_lowercase))
        })
        .then_with(|| a.nfd().map(case_weight).cmp(b.nfd().map(case_weight)))
}

fn primary_weight(ch: char) -> u32 {
    const PUNCTUATION: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    if ch.is_ascii_alphanumeric() {
        ch.to_ascii_lowercase() as u32
    } else if let Some(index) = PUNCTUATION.bytes().position(|byte| char::from(byte) == ch) {
        index as u32
    } else {
        ch as u32 + 128
    }
}

fn case_weight(ch: char) -> (bool, char) {
    (ch.is_uppercase(), ch)
}

struct SectionScore<'a> {
    section: CompletionSection<'a>,
    best: Option<i64>,
}

impl SectionScore<'_> {
    fn compare(&self, other: &Self) -> Ordering {
        let a = self.section;
        let b = other.section;
        if a.rank == SectionRank::Dynamic
            && b.rank == SectionRank::Dynamic
            && let (Some(a), Some(b)) = (self.best, other.best)
            && a != b
        {
            return b.cmp(&a);
        }
        let number = |rank| match rank {
            SectionRank::Fixed(value) => value,
            _ => 1_000_000_000,
        };
        number(a.rank)
            .cmp(&number(b.rank))
            .then_with(|| a.name.encode_utf16().cmp(b.name.encode_utf16()))
    }
}

fn sort_sections(sections: &mut [SectionScore<'_>]) {
    let [first, second, ..] = sections else {
        return;
    };
    // Mixed dynamic and unranked sections can form comparison cycles. Use
    // the initial run and stable insertion used by JavaScript for short
    // lists rather than a Rust sort that requires a total order.
    let descending = first.compare(second).is_gt();
    let end = sections
        .windows(2)
        .take_while(|pair| pair[0].compare(&pair[1]).is_gt() == descending)
        .count()
        + 1;
    if descending {
        sections[..end].reverse();
    }
    for index in end..sections.len() {
        let (mut lo, mut hi) = (0, index);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if sections[index].compare(&sections[mid]).is_lt() {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        sections[lo..=index].rotate_right(1);
    }
}

/// Replace the source range, and replace the corresponding range at other
/// selections when it contains the same text. If the range ends at the start
/// of the main selection, also replace the selected text at each applicable range.
pub fn insert_completion_text(
    state: &EditorState,
    source: Range<usize>,
    insert: &str,
) -> Result<Transaction, EditError> {
    let main = state.selection().main().range();
    let pattern = state
        .text()
        .slice(source.clone())
        .ok_or(EditError::InvalidRange)?;
    let transaction = state.change_by_range(|selection| {
        let selected = selection.range();
        let rebase = |pos| {
            if pos >= main.start {
                selected.start.checked_add(pos - main.start)
            } else {
                selected.start.checked_sub(main.start - pos)
            }
        };
        let target = rebase(source.start).zip(rebase(source.end));
        let no_edit = RangeEdit {
            changes: ChangeSet::default(),
            selection,
            effects: Vec::new(),
        };
        let Some((start, end)) = target else {
            return Ok(no_edit);
        };
        if state.text().slice(start..end) != Some(pattern) {
            return Ok(no_edit);
        }
        let end = if source.end == main.start {
            selected.end
        } else {
            end
        };
        let cursor = start
            .checked_add(insert.len())
            .ok_or(EditError::InvalidRange)?;
        Ok(RangeEdit {
            changes: ChangeSet::single(Change::new(start..end, insert))?,
            selection: SelectionRange::new(cursor, cursor),
            effects: Vec::new(),
        })
    })?;
    Ok(transaction
        .with_user_event("input.complete")
        .with_scroll_into_view())
}
