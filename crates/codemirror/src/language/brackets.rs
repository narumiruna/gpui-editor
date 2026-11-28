//! Plain bracket matching based on `@codemirror/language` 6.13.1.
//! See `docs/licenses/codemirror-language.txt`.

use std::ops::Range;

use crate::Text;

mod extension;
mod syntax;

pub use extension::{
    BracketMatchRender, BracketMatchingOptions, bracket_match_decorations, bracket_matching,
    bracket_matching_options, configured_bracket_matches, configured_bracket_matches_in,
};
pub(crate) use extension::{state_bracket_match_decorations, state_bracket_matches};
pub(super) use syntax::{
    BracketMates, directional_bracket_near, match_named_siblings, named_delimiter, named_sibling,
    node_delimiter,
};

const DEFAULT_BRACKETS: &str = "()[]{}";
const EDITOR_BRACKETS: &[u8] = DEFAULT_BRACKETS.as_bytes();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BracketDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug)]
pub struct BracketConfig<'a> {
    /// Consecutive opening and closing characters. Use an empty string for the default.
    pub brackets: &'a str,
    /// Maximum distance at chunk boundaries. The matcher defines the units.
    /// Use zero for the default.
    pub max_scan_distance: usize,
    /// Also check just after the caret if neither before-caret probe returns a match.
    pub after_cursor: bool,
}

impl Default for BracketConfig<'_> {
    fn default() -> Self {
        Self {
            brackets: DEFAULT_BRACKETS,
            max_scan_distance: 10_000,
            after_cursor: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BracketMatch {
    pub start: Range<usize>,
    pub end: Option<Range<usize>>,
    pub matched: bool,
}

/// Look for a bracket next to a caret. Prefer the bracket just before it.
/// `same_syntax` restricts comparisons to the starting bracket's syntax class.
/// Hosts without syntax information can pass `|_, _| true`.
pub fn bracket_near(
    text: &Text,
    caret: usize,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Option<BracketMatch> {
    bracket_near_in(text, 0..text.len(), caret, config, same_syntax)
}

/// Like [`bracket_near`], but do not inspect text outside `scope`.
pub fn bracket_near_in(
    text: &Text,
    scope: Range<usize>,
    caret: usize,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Option<BracketMatch> {
    bracket_near_with_units(text, scope, caret, config, same_syntax, str::len)
}

fn bracket_near_with_units(
    text: &Text,
    scope: Range<usize>,
    caret: usize,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize, usize) -> bool,
    scan_units: impl Fn(&str) -> usize,
) -> Option<BracketMatch> {
    use BracketDirection::{Backward, Forward};
    let (before, after) = (
        text.slice(scope.start..caret)?,
        text.slice(caret..scope.end)?,
    );
    let prev = before.chars().next_back().map(|ch| caret - ch.len_utf8());
    let next = after.chars().next().map(|ch| caret + ch.len_utf8());
    let scan = |at, dir| {
        match_in(
            text,
            scope.clone(),
            at,
            dir,
            config,
            |start, at| start == at || same_syntax(start, at),
            &scan_units,
        )
    };
    scan(caret, Backward)
        .or_else(|| scan(prev?, Forward))
        .or_else(|| {
            config
                .after_cursor
                .then(|| scan(caret, Forward).or_else(|| scan(next?, Backward)))?
        })
}

/// Scan text for a pair. The starting bracket is at `pos` for forward scanning,
/// or immediately before it for backward scanning. `same_syntax` receives the
/// byte starts of the starting and candidate brackets, in that order.
///
/// Returns `None` if there is no starting bracket, the position is invalid, or
/// the search stops at the distance limit. Reaching the end without a partner
/// yields an unmatched result. This function does not query syntax tree node props.
pub fn match_bracket(
    text: &Text,
    pos: usize,
    direction: BracketDirection,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Option<BracketMatch> {
    match_in(
        text,
        0..text.len(),
        pos,
        direction,
        config,
        |start, at| start == at || same_syntax(start, at),
        str::len,
    )
}

pub(super) fn bracket_pairs(config: BracketConfig<'_>) -> &str {
    if config.brackets.is_empty() {
        DEFAULT_BRACKETS
    } else {
        config.brackets
    }
}

/// Scan a directional plain fallback over a byte-preserving logical line view.
/// Measure chunk boundaries in UTF-16 units. Apply the candidate syntax test
/// even to the starting character: its type can differ from the type resolved
/// at the directional starting position.
pub(super) fn match_configured_editor_text(
    lines: &Text,
    pos: usize,
    direction: BracketDirection,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize) -> bool,
) -> Option<BracketMatch> {
    match_configured_editor_text_in(lines, 0..lines.len(), pos, direction, config, same_syntax)
}

/// Like `match_configured_editor_text`, but stop at the scope bounds.
pub(super) fn match_configured_editor_text_in(
    lines: &Text,
    scope: Range<usize>,
    pos: usize,
    direction: BracketDirection,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize) -> bool,
) -> Option<BracketMatch> {
    if !(scope.start..=scope.end).contains(&pos) {
        return None;
    }
    match_in(
        lines,
        scope,
        pos,
        direction,
        config,
        |_, at| same_syntax(at),
        |chunk| {
            if chunk == "\r\n" {
                1
            } else {
                chunk.encode_utf16().count()
            }
        },
    )
}

fn match_in(
    text: &Text,
    scope: Range<usize>,
    pos: usize,
    direction: BracketDirection,
    config: BracketConfig<'_>,
    same_syntax: impl Fn(usize, usize) -> bool,
    scan_units: impl Fn(&str) -> usize,
) -> Option<BracketMatch> {
    let forward = direction == BracketDirection::Forward;
    let range = if forward {
        pos..scope.end
    } else {
        scope.start..pos
    };
    let pairs = bracket_pairs(config);
    let bracket_at = |ch| pairs.chars().position(|candidate| candidate == ch);
    let slice = text.slice(range.clone())?;
    let first = if forward {
        slice.chars().next()?
    } else {
        slice.chars().next_back()?
    };
    let starting_index = bracket_at(first)?;
    let opening_side = |index: usize| index.is_multiple_of(2) == forward;
    if !opening_side(starting_index) {
        return None;
    }
    let start = if forward {
        pos..pos + first.len_utf8()
    } else {
        pos - first.len_utf8()..pos
    };
    let limit = if config.max_scan_distance == 0 {
        10_000
    } else {
        config.max_scan_distance
    };
    let mut chunks = text.iter_range(range)?;
    let mut bytes = 0;
    let mut distance = 0;
    let mut depth = 0_i64;
    while let Some(chunk) = if forward {
        chunks.next()
    } else {
        chunks.next_back()
    } {
        if distance > limit {
            return None;
        }
        let base = if forward {
            pos + bytes
        } else {
            pos - bytes - chunk.len()
        };
        let mut chars = chunk.char_indices();
        while let Some((offset, ch)) = if forward {
            chars.next()
        } else {
            chars.next_back()
        } {
            let Some(index) = bracket_at(ch) else {
                continue;
            };
            let at = base + offset;
            if !same_syntax(start.start, at) {
                continue;
            }
            if opening_side(index) {
                depth += 1;
            } else if depth == 1 {
                return Some(BracketMatch {
                    start,
                    end: Some(at..at + ch.len_utf8()),
                    matched: index / 2 == starting_index / 2,
                });
            } else {
                depth -= 1;
            }
        }
        bytes += chunk.len();
        distance += scan_units(chunk);
    }
    Some(BracketMatch {
        start,
        end: None,
        matched: false,
    })
}

pub(super) fn editor_bracket_index(byte: u8) -> Option<usize> {
    EDITOR_BRACKETS.iter().position(|&bracket| bracket == byte)
}

/// Use syntax siblings for marked delimiters. An unmatched marked delimiter
/// does not trigger a plain-text scan.
pub(super) fn match_editor_siblings(
    start: Range<usize>,
    index: usize,
    mut siblings: impl DoubleEndedIterator<Item = (Range<usize>, usize)>,
) -> BracketMatch {
    let forward = index.is_multiple_of(2);
    let mut hit = BracketMatch {
        start,
        end: None,
        matched: false,
    };
    let mut depth = 0;
    while let Some((range, other)) = if forward {
        siblings.next()
    } else {
        siblings.next_back()
    } {
        if (forward && (range.start < hit.start.end || range.end <= hit.start.end))
            || (!forward && (range.end > hit.start.start || range.start >= hit.start.start))
        {
            continue;
        }
        if other.is_multiple_of(2) == forward {
            depth += 1;
        } else if depth == 0 {
            hit.matched = index / 2 == other / 2 && !range.is_empty();
            hit.end = (!range.is_empty()).then_some(range);
            break;
        } else {
            depth -= 1;
        }
    }
    hit
}

/// Plain fallback for syntax-aware editor matches. Positions are original
/// byte offsets. Measure the limit in normalized UTF-16 line chunks.
pub(super) fn match_editor_text(
    lines: &Text,
    start: Range<usize>,
    index: usize,
    same_syntax: impl Fn(usize) -> bool,
) -> Option<BracketMatch> {
    let forward = index.is_multiple_of(2);
    let (mut base, end) = if forward {
        (start.start, lines.len())
    } else {
        (start.end, 0)
    };
    let mut distance = 0;
    let mut depth = 0_i64;
    for chunk in lines.cursor_range(base, end)? {
        if distance > 10_000 {
            return None;
        }
        if !forward {
            base -= chunk.len();
        }
        let mut bytes = chunk.bytes().enumerate();
        while let Some((offset, byte)) = if forward {
            bytes.next()
        } else {
            bytes.next_back()
        } {
            let Some(other) = editor_bracket_index(byte) else {
                continue;
            };
            let at = base + offset;
            if !same_syntax(at) {
                continue;
            }
            if other.is_multiple_of(2) == forward {
                depth += 1;
            } else if depth == 1 {
                return Some(BracketMatch {
                    start,
                    end: Some(at..at + 1),
                    matched: index / 2 == other / 2,
                });
            } else {
                depth -= 1;
            }
        }
        if forward {
            base += chunk.len();
        }
        distance += if chunk == "\r\n" {
            1
        } else {
            chunk.encode_utf16().count()
        };
    }
    Some(BracketMatch {
        start,
        end: None,
        matched: false,
    })
}
