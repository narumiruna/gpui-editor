use std::ops::Range;

use super::{
    BracketDirection,
    BracketDirection::{Backward, Forward},
    BracketMatch,
};
use crate::language::line_projection::splits_crlf;

/// Probe logical positions around the caret in the order used by CodeMirror.
/// A marked hit, mismatch, or unmatched result stops the search. Only `None`
/// (including a distance cutoff) permits another probe.
pub(in crate::language) fn directional_bracket_near(
    source: &str,
    head: usize,
    after_cursor: bool,
    mut probe: impl FnMut(usize, BracketDirection) -> Option<BracketMatch>,
) -> Option<BracketMatch> {
    let before = source.get(..head)?;
    if splits_crlf(source, head) {
        return None;
    }
    let after = &source[head..];
    let previous = || {
        Some(
            head - if before.ends_with("\r\n") {
                2
            } else {
                before.chars().next_back()?.len_utf8()
            },
        )
    };
    let next = || {
        Some(
            head + if after.starts_with("\r\n") {
                2
            } else {
                after.chars().next()?.len_utf8()
            },
        )
    };
    probe(head, Backward)
        .or_else(|| probe(previous()?, Forward))
        .or_else(|| {
            after_cursor.then(|| probe(head, Forward).or_else(|| probe(next()?, Backward)))?
        })
}

/// A configured one-character node name can participate in a marked scan.
/// `Some(None)` is a delimiter with no mate, from an odd-length pair list.
pub(in crate::language) fn named_delimiter<'a>(
    name: &str,
    direction: BracketDirection,
    pairs: &'a str,
) -> Option<Option<&'a str>> {
    let mut chars = name.chars();
    let name = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let index = pairs.chars().position(|ch| ch == name)?;
    if index.is_multiple_of(2) != (direction == Forward) {
        return None;
    }
    Some(
        pairs
            .char_indices()
            .nth(index ^ 1)
            .map(|(at, ch)| &pairs[at..at + ch.len_utf8()]),
    )
}

/// Match siblings by visible grammar name before checking their depth role.
/// The mate need not have a delimiter property of its own.
pub(in crate::language) fn match_named_siblings<N: Copy>(
    start: Range<usize>,
    direction: BracketDirection,
    expected: Option<&str>,
    mut siblings: impl DoubleEndedIterator<Item = N>,
    describe: impl Fn(N) -> (Range<usize>, &'static str),
    is_delimiter: impl Fn(N, BracketDirection) -> bool,
) -> BracketMatch {
    let forward = direction == Forward;
    let opposite = if forward { Backward } else { Forward };
    let mut hit = BracketMatch {
        start,
        end: None,
        matched: false,
    };
    let mut depth = 0;
    while let Some(sibling) = if forward {
        siblings.next()
    } else {
        siblings.next_back()
    } {
        let (range, name) = describe(sibling);
        if (forward && (range.start < hit.start.end || range.end <= hit.start.end))
            || (!forward && (range.end > hit.start.start || range.start >= hit.start.start))
        {
            continue;
        }
        if depth == 0 && expected == Some(name) && !range.is_empty() {
            hit.end = Some(range);
            hit.matched = true;
            break;
        }
        if is_delimiter(sibling, direction) {
            depth += 1;
        } else if is_delimiter(sibling, opposite) {
            if depth == 0 {
                hit.end = (!range.is_empty()).then_some(range);
                break;
            }
            depth -= 1;
        }
    }
    hit
}
