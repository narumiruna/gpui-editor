//! Snapshot isolation marks from `@codemirror/language` 6.13.1.

use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    EditError, EditorState, Extension, Facet, RangeSet, Text,
    view::{MarkDecoration, Viewport, bidi::Direction},
};

/// Options and host direction for a snapshot of syntax isolation marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BidiIsolateOptions {
    /// Disable the optimization that restricts marks to RTL line ranges in an LTR view.
    pub always_isolate: bool,
    /// Base direction of the view.
    pub text_direction: Direction,
    /// Whether the view can use different base directions per line.
    pub per_line_text_direction: bool,
}

impl Default for BidiIsolateOptions {
    fn default() -> Self {
        Self {
            always_isolate: false,
            text_direction: Direction::Ltr,
            per_line_text_direction: false,
        }
    }
}

static ISOLATE: LazyLock<Facet<bool, Option<bool>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&bool]| (!inputs.is_empty()).then(|| inputs.iter().any(|&&flag| flag)))
});

/// Ask the host to enable syntax bidi isolation in the installed language.
/// Set `always_isolate` to mark eligible nodes even on LTR-only lines.
/// The host supplies the actual viewport, direction, and paint geometry.
pub fn bidi_isolates(always_isolate: bool) -> Extension {
    ISOLATE.of(always_isolate)
}

/// Return `None` if syntax isolation is not installed. When multiple inputs
/// request it, any request for unconditional marks takes precedence.
pub fn configured_bidi_isolates(state: &EditorState) -> Option<bool> {
    *state
        .context()
        .facet(&ISOLATE)
        .expect("typed bidi isolation option")
}

// The adapters supply sorted, nonoverlapping leaf ranges. If the language
// normalizes a line break, the projection must keep original byte offsets.
pub(super) fn isolate_marks(
    text: &Text,
    lines: &Text,
    isolate_ranges: &[Range<usize>],
    viewport: &Viewport,
    options: BidiIsolateOptions,
) -> Result<Arc<RangeSet<MarkDecoration>>, EditError> {
    isolate_marks_for(text, lines, viewport, options, |range| {
        let start = isolate_ranges.partition_point(|r| r.end < range.start);
        isolate_ranges[start..]
            .iter()
            .take_while(move |r| r.start <= range.end)
            .map(|r| (r.clone(), None))
    })
}

pub(super) fn isolate_marks_for<Nodes>(
    text: &Text,
    lines: &Text,
    viewport: &Viewport,
    options: BidiIsolateOptions,
    nodes_at: impl Fn(Range<usize>) -> Nodes,
) -> Result<Arc<RangeSet<MarkDecoration>>, EditError>
where
    Nodes: Iterator<Item = (Range<usize>, Option<Direction>)>,
{
    debug_assert_eq!(text.len(), lines.len());
    let visible = viewport.visible_ranges();
    if !text.is_valid_range(viewport.interval())
        || visible.iter().any(|r| !text.is_valid_range(r.clone()))
    {
        return Err(EditError::InvalidRange);
    }
    let clipped;
    let ranges = if options.always_isolate
        || options.text_direction == Direction::Rtl
        || options.per_line_text_direction
    {
        visible
    } else {
        clipped = clip_rtl_ranges(lines, visible);
        // A clipped fragment can pass the length limit even when the full
        // line cannot. The plugin first needs RTL on a line within the limit.
        if clipped.is_empty() || !lines.iter_lines().any(build_for_line) {
            return Ok(Arc::new(RangeSet::empty()));
        }
        &clipped
    };
    let mark = MarkDecoration::new("cm-iso").with_inclusive_edges(true, true);
    let mut marks = Vec::new();
    for visible in ranges {
        // Lezer also visits nodes that only touch a viewport endpoint.
        for (range, direction) in nodes_at(visible.clone()) {
            marks.push(mark.clone().with_bidi_direction(direction).range(range)?);
        }
    }
    // A nested isolate can touch more than one visible range.
    Ok(Arc::new(
        RangeSet::of(marks, true).expect("valid isolate ranges"),
    ))
}

fn clip_rtl_ranges(text: &Text, visible: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut result: Vec<Range<usize>> = Vec::new();
    let mut cursor = text.cursor();
    let mut start = 0;
    for range in visible {
        let mut from = range.start;
        if let Some(last) = result.last()
            && last.end > from
        {
            from = last.end;
            if from >= range.end {
                continue;
            }
        }
        let end = start + cursor.value().len();
        if end < from {
            cursor
                .next_with_skip((from - end) as isize)
                .expect("valid viewport");
            start = from;
        }
        loop {
            let end = start + cursor.value().len();
            if !cursor.line_break() && build_for_line(cursor.value()) {
                let to = end.min(range.end);
                if let Some(last) = result.last_mut()
                    && (last.end >= start || short_gap(text.slice(last.end..start).unwrap()))
                {
                    last.end = to;
                } else {
                    result.push(start..to);
                }
            }
            if end >= range.end || cursor.next().is_none() {
                break;
            }
            start = end;
        }
    }
    result
}

fn short_gap(gap: &str) -> bool {
    let mut chars = gap.chars().peekable();
    let mut units = 0;
    while let Some(ch) = chars.next() {
        // Upstream represents CRLF as one logical LF.
        if ch == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }
        units += ch.len_utf16();
        if units >= 10 {
            return false;
        }
    }
    true
}

fn build_for_line(line: &str) -> bool {
    line.encode_utf16().take(4097).count() <= 4096
        && line
            .chars()
            .any(|ch| matches!(ch as u32, 0x590..=0x5f4 | 0x600..=0x8ac | 0xfb50..=0xfdff))
}
