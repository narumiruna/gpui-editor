//! Headless highlight projections from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.
//!
//! The host supplies the viewport, any measured line blocks, and rendering.
//! These functions build snapshot sets; they do not keep the browser's
//! incremental matcher cache or compute wrapping, fold or widget geometry.

use std::{ops::Range as ByteRange, sync::LazyLock};

use super::{
    MarkDecoration, Viewport,
    decorations::{Decoration, DecorationSet, LineDecoration},
};
use crate::{
    EditError, EditorSelection, EditorState, Extension, Facet, Range, Text,
    language::line_projection::configured_editor_lines,
};

/// Independently installable stock highlight extensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewHighlight {
    ActiveLine,
    Whitespace,
    TrailingWhitespace,
}

type EnabledHighlights = Facet<ViewHighlight, Vec<ViewHighlight>>;
static ENABLED: LazyLock<EnabledHighlights> = LazyLock::new(Facet::collect);
static ACTIVE_LINE: LazyLock<Extension> = LazyLock::new(|| ENABLED.of(ViewHighlight::ActiveLine));
static WHITESPACE: LazyLock<Extension> = LazyLock::new(|| ENABLED.of(ViewHighlight::Whitespace));
static TRAILING_WHITESPACE: LazyLock<Extension> =
    LazyLock::new(|| ENABLED.of(ViewHighlight::TrailingWhitespace));

impl ViewHighlight {
    pub fn extension(self) -> Extension {
        match self {
            Self::ActiveLine => ACTIVE_LINE.clone(),
            Self::Whitespace => WHITESPACE.clone(),
            Self::TrailingWhitespace => TRAILING_WHITESPACE.clone(),
        }
    }

    pub fn enabled(self, state: &EditorState) -> bool {
        state
            .context()
            .facet(&ENABLED)
            .expect("typed highlight facet")
            .contains(&self)
    }

    /// Project this highlight when installed. For active lines this uses
    /// logical lines. Use [`highlight_active_line_for_blocks`] with the same
    /// enabled check to use measured line blocks instead.
    pub fn project(
        self,
        state: &EditorState,
        viewport: &Viewport,
    ) -> Result<DecorationSet, EditError> {
        if !self.enabled(state) {
            return Ok(DecorationSet::empty());
        }
        let context = state.context();
        let projected = configured_editor_lines(&context)?;
        let lines = projected.as_ref().as_ref().unwrap_or(context.text());
        match self {
            Self::ActiveLine => highlight_active_line(lines, context.selection()),
            Self::Whitespace => highlight_whitespace(lines, viewport),
            Self::TrailingWhitespace => highlight_trailing_whitespace(lines, viewport),
        }
    }
}

/// Mark the start of each logical line that contains a selection head.
/// This includes nonempty selections and heads outside the viewport. Mark a
/// line only once, even if it has more than one head.
///
/// Return [`EditError::InvalidSelection`] if an endpoint is outside the text
/// or is not a UTF-8 boundary.
pub fn highlight_active_line(
    text: &Text,
    selection: &EditorSelection,
) -> Result<DecorationSet, EditError> {
    highlight_active_line_for_blocks(text, selection, |head| {
        text.line_at(head)
            .map(|line| line.range.start..line.range.end.max(head))
    })
}

/// Mark each host-provided line block that contains a selection head. A block
/// may join source lines across a replacement or start after a block widget.
/// The host must supply a valid block even for a head outside the viewport.
/// A missing block, invalid byte range, or block that excludes its head is an
/// [`EditError::InvalidRange`]. Repeated or earlier block starts are skipped.
pub fn highlight_active_line_for_blocks(
    text: &Text,
    selection: &EditorSelection,
    mut line_block_at: impl FnMut(usize) -> Option<ByteRange<usize>>,
) -> Result<DecorationSet, EditError> {
    selection.check(|position| text.is_char_boundary(position))?;
    let line = Decoration::line(LineDecoration::new("cm-activeLine"));
    let mut ranges = Vec::new();
    let mut last = None;
    for range in selection.ranges() {
        let head = range.head();
        let block = line_block_at(head).ok_or(EditError::InvalidRange)?;
        if !(block.start..=block.end).contains(&head) || !text.is_valid_range(block.clone()) {
            return Err(EditError::InvalidRange);
        }
        let start = block.start;
        if last.is_none_or(|last| start > last) {
            ranges.push(Range::new(start, start, line.clone()));
            last = Some(start);
        }
    }
    Ok(highlights(text, ranges))
}

/// Mark each ASCII space and tab in the viewport's match ranges.
/// These ranges include up to 1000 UTF-16 units of context on each side of
/// visible ranges when they do not cover the viewport.
///
/// Return [`EditError::InvalidRange`] if the viewport has invalid endpoints
/// for this text.
pub fn highlight_whitespace(text: &Text, viewport: &Viewport) -> Result<DecorationSet, EditError> {
    let space = Decoration::mark(MarkDecoration::new("cm-highlightSpace"));
    let tab = Decoration::mark(MarkDecoration::new("cm-highlightTab"));
    match_lines(text, viewport, |start, chunk, ranges| {
        for (offset, byte) in chunk.bytes().enumerate() {
            let mark = match byte {
                b' ' => &space,
                b'\t' => &tab,
                _ => continue,
            };
            ranges.push(Range::new(start + offset, start + offset + 1, mark.clone()));
        }
    })
}

/// Mark the ECMAScript whitespace suffix of each matched logical line or
/// line fragment. LF and CRLF endings are not part of the match. The matching
/// context is the same as in [`highlight_whitespace`].
///
/// Return [`EditError::InvalidRange`] if the viewport has invalid endpoints
/// for this text.
pub fn highlight_trailing_whitespace(
    text: &Text,
    viewport: &Viewport,
) -> Result<DecorationSet, EditError> {
    let mark = Decoration::mark(MarkDecoration::new("cm-trailingSpace"));
    match_lines(text, viewport, |start, chunk, ranges| {
        let from = chunk.trim_end_matches(ecmascript_whitespace).len();
        if from < chunk.len() {
            ranges.push(Range::new(start + from, start + chunk.len(), mark.clone()));
        }
    })
}

fn highlights(text: &Text, ranges: Vec<Range<Decoration>>) -> DecorationSet {
    DecorationSet::new(text, ranges, false).expect("highlight ranges are valid and ordered")
}

fn match_lines(
    text: &Text,
    viewport: &Viewport,
    mut add_matches: impl FnMut(usize, &str, &mut Vec<Range<Decoration>>),
) -> Result<DecorationSet, EditError> {
    let mut decorations = Vec::new();
    for range in match_ranges(text, viewport)? {
        let mut start = range.start;
        for chunk in text.iter_range(range).unwrap() {
            let end = start + chunk.len();
            // A viewport may stop between the CR and LF of a line ending.
            if !(matches!(chunk, "\n" | "\r\n")
                || chunk == "\r" && text.range_eq(end..end + 1, "\n"))
            {
                add_matches(start, chunk, &mut decorations);
            }
            start = end;
        }
    }
    Ok(highlights(text, decorations))
}

fn match_ranges(text: &Text, viewport: &Viewport) -> Result<Vec<ByteRange<usize>>, EditError> {
    let interval = viewport.interval();
    let visible = viewport.visible_ranges();
    if !text.is_valid_range(interval.clone())
        || visible
            .iter()
            .any(|range| !text.is_valid_range(range.clone()))
    {
        return Err(EditError::InvalidRange);
    }
    if visible == std::slice::from_ref(&interval) {
        return Ok(visible.to_vec());
    }
    let mut result: Vec<ByteRange<MatchPos>> = Vec::new();
    for range in visible {
        let first = text.line_at(range.start).unwrap();
        let last = text.line_at(range.end).unwrap();
        let left = range.start.min(first.range.end);
        let before = &first.text[..left - first.range.start];
        let after = &last.text[range.end.min(last.range.end) - last.range.start..];
        let (back, start_split) = match_context(before.chars().rev());
        let (forward, end_split) = match_context(after.chars());
        let start = MatchPos {
            byte: left - back,
            split: start_split,
        };
        let end = MatchPos {
            byte: range.end + forward - split_bytes(end_split),
            split: end_split,
        };
        if let Some(previous) = result.last_mut().filter(|previous| previous.end >= start) {
            previous.end = end;
        } else {
            result.push(start..end);
        }
    }
    Ok(result
        .into_iter()
        .map(|range| range.start.byte..range.end.byte + split_bytes(range.end.split))
        .collect())
}

// Do not merge across a gap of one surrogate, even though we include the
// entire scalar when scanning. A surrogate cannot match whitespace, but can
// keep a preceding space from matching `$` at the end of a fragment.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct MatchPos {
    byte: usize,
    split: bool,
}

fn split_bytes(split: bool) -> usize {
    4 * usize::from(split)
}

fn match_context(chars: impl Iterator<Item = char>) -> (usize, bool) {
    let mut units = 1000;
    let mut bytes = 0;
    for ch in chars {
        if units == 0 {
            break;
        }
        bytes += ch.len_utf8();
        if ch.len_utf16() > units {
            return (bytes, true);
        }
        units -= ch.len_utf16();
    }
    (bytes, false)
}

fn ecmascript_whitespace(ch: char) -> bool {
    matches!(ch, '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
