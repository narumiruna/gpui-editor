//! Headless match decorations from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.

use std::{
    ops::Range as ByteRange,
    sync::{Arc, LazyLock},
};

use regex::{Captures, Regex};

use super::{
    MarkDecoration, ViewUpdate, Viewport,
    decorations::{Decoration, DecorationSet, WidgetSpec},
};
use crate::{
    ConfigurationError, EditError, EditorState, Extension, Facet, Range, Text,
    language::line_projection::configured_editor_lines,
};

mod special_chars;
pub use special_chars::{
    SpecialChar, SpecialCharAppearance, SpecialCharConfig, SpecialCharRender,
    highlight_special_chars, special_char_appearance, special_char_decorations,
    special_char_extension,
};

const DEFAULT_CONTEXT: usize = 1000;
const UPDATE_LIMIT: usize = 1000;

/// One nonempty match in a logical line or a visible fragment of a line.
/// Byte positions refer to the full document unless noted otherwise.
pub struct MatchContext<'a> {
    pub text: &'a Text,
    /// Available for state-based creation and view updates.
    pub state: Option<&'a EditorState>,
    pub viewport: &'a Viewport,
    pub range: ByteRange<usize>,
    /// Captures and offsets relative to the fragment scanned by the engine.
    pub captures: Captures<'a>,
    fragment_start: usize,
}

impl MatchContext<'_> {
    /// The capture's range in the document. An unmatched group has no range.
    pub fn group_range(&self, group: usize) -> Option<ByteRange<usize>> {
        self.captures
            .get(group)
            .map(|m| self.fragment_start + m.start()..self.fragment_start + m.end())
    }
}

type MakeDecorations<W> = dyn Fn(MatchContext<'_>) -> Vec<Range<Decoration<W>>> + Send + Sync;

enum MatchStyle<W> {
    Fixed(Decoration<W>),
    Callback(Arc<MakeDecorations<W>>),
}

#[derive(Clone, Copy)]
struct ScanArea<'a> {
    text: &'a Text,
    state: Option<&'a EditorState>,
    viewport: &'a Viewport,
}

/// Maintain native decorations for non-overlapping regular expression matches.
///
/// The expression uses Rust's `regex` syntax and never matches across logical
/// lines. Positions are UTF-8 byte offsets. The host supplies the viewport
/// and draws the decorations.
pub struct MatchDecorator<W = ()> {
    regexp: Regex,
    style: MatchStyle<W>,
    boundary: Option<Regex>,
    max_length: usize,
}

impl<W> Clone for MatchDecorator<W> {
    fn clone(&self) -> Self {
        let style = match &self.style {
            MatchStyle::Fixed(value) => MatchStyle::Fixed(value.clone()),
            MatchStyle::Callback(value) => MatchStyle::Callback(value.clone()),
        };
        Self {
            regexp: self.regexp.clone(),
            style,
            boundary: self.boundary.clone(),
            max_length: self.max_length,
        }
    }
}

impl<W> std::fmt::Debug for MatchDecorator<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchDecorator")
            .field("regexp", &self.regexp)
            .field("boundary", &self.boundary)
            .field("max_length", &self.max_length)
            .finish_non_exhaustive()
    }
}

impl MatchDecorator {
    pub fn new(regexp: Regex, mark: MarkDecoration) -> Self {
        Self::fixed(regexp, Decoration::mark(mark))
    }
}

impl<W: PartialEq> MatchDecorator<W> {
    /// Apply a shared decoration to the whole of each match. Use a range
    /// decoration such as a mark or replacement, not a point widget.
    pub fn fixed(regexp: Regex, decoration: Decoration<W>) -> Self {
        Self::with_style(regexp, MatchStyle::Fixed(decoration))
    }

    /// Apply a decoration to the whole match, or skip it when the callback
    /// returns `None`.
    pub fn with_decoration(
        regexp: Regex,
        decorate: impl Fn(MatchContext<'_>) -> Option<Decoration<W>> + Send + Sync + 'static,
    ) -> Self {
        Self::with_decorate(regexp, move |context| {
            let range = context.range.clone();
            decorate(context)
                .into_iter()
                .map(|decoration| Range::new(range.start, range.end, decoration))
                .collect()
        })
    }

    /// Produce any number of decorations inside each match, including point
    /// widgets at its endpoints. Callbacks must be read-only. Rebuild the set
    /// if captured host settings change without an edit or viewport update.
    pub fn with_decorate(
        regexp: Regex,
        decorate: impl Fn(MatchContext<'_>) -> Vec<Range<Decoration<W>>> + Send + Sync + 'static,
    ) -> Self {
        Self::with_style(regexp, MatchStyle::Callback(Arc::new(decorate)))
    }

    fn with_style(regexp: Regex, style: MatchStyle<W>) -> Self {
        Self {
            regexp,
            style,
            boundary: None,
            max_length: DEFAULT_CONTEXT,
        }
    }

    /// Limit rescans at characters that cannot be part of a match. The
    /// expression is tested on one Unicode scalar at a time.
    pub fn with_boundary(mut self, boundary: Regex) -> Self {
        self.boundary = Some(boundary);
        self
    }

    /// Set the search context at each edge of a visible range, in UTF-16 units.
    /// The default is 1000. Context stays on the same logical line and does not
    /// split a Unicode scalar. A single range that covers the viewport needs
    /// no expansion.
    pub fn with_max_length(mut self, max_length: usize) -> Self {
        self.max_length = max_length;
        self
    }

    /// Find matches in the visible ranges and their context.
    pub fn create_deco(
        &self,
        text: &Text,
        viewport: &Viewport,
    ) -> Result<DecorationSet<W>, EditError> {
        self.create(ScanArea {
            text,
            state: None,
            viewport,
        })
    }

    /// Project matches with the state available to callbacks. Rebuild when
    /// a non-document input used by a callback changes.
    pub fn create_state_deco(
        &self,
        state: &EditorState,
        viewport: &Viewport,
    ) -> Result<DecorationSet<W>, EditError> {
        let context = state.context();
        let projected = configured_editor_lines(&context)?;
        self.create(ScanArea {
            text: projected.as_ref().as_ref().unwrap_or(context.text()),
            state: Some(state),
            viewport,
        })
    }

    fn create(&self, area: ScanArea<'_>) -> Result<DecorationSet<W>, EditError> {
        let mut ranges = Vec::new();
        for range in match_ranges(area.text, area.viewport, self.max_length)? {
            self.scan(area, range, &mut ranges)?;
        }
        make_set(
            area.text,
            ranges,
            matches!(self.style, MatchStyle::Callback(_)),
        )
    }

    /// Update a set from this decorator for the previous view state.
    ///
    /// Rebuild on viewport movement or when the span of edits that touch the
    /// viewport exceeds 1000 UTF-16 units. Otherwise map old decorations and
    /// rescan the affected visible line fragments. Unlike the upstream helper,
    /// this maps them even when an edit before the viewport only shifts them.
    pub fn update_deco(
        &self,
        update: &ViewUpdate<'_>,
        previous: &DecorationSet<W>,
    ) -> Result<DecorationSet<W>, EditError> {
        let text = update.state.text();
        let old_text = update.start_state.text();
        let changes = update.changes;
        if changes.length() != old_text.len() || changes.new_length() != text.len() {
            return Err(EditError::MismatchedLength);
        }
        check_viewport(old_text, update.start_viewport)?;
        check_viewport(text, update.viewport)?;
        if update.start_viewport.map(changes) != *update.viewport {
            return self.create_state_deco(update.state, update.viewport);
        }
        let projection = configured_editor_lines(&update.state.context())?;
        let before = configured_editor_lines(&update.start_state.context())?;
        if before.is_some() != projection.is_some() {
            return self.create_state_deco(update.state, update.viewport);
        }
        if changes.is_empty() {
            return Ok(previous.clone());
        }
        let lines = projection.as_ref().as_ref().unwrap_or(text);

        let viewport = update.viewport.interval();
        let extent = changes
            .changed_ranges()
            .map(|(_, new)| new)
            .filter(|new| new.start <= viewport.end && new.end >= viewport.start)
            .reduce(|span, new| span.start..new.end);
        if let Some(range) = &extent {
            if !text.is_valid_range(range.clone()) {
                return Err(EditError::InvalidRange);
            }
            if exceeds_update_limit(text, range.clone()) {
                return self.create_state_deco(update.state, update.viewport);
            }
        }

        if changes
            .changed_ranges()
            .any(|(_, range)| !text.is_valid_range(range))
        {
            return Err(EditError::InvalidRange);
        }
        let mapped = previous.map_desc(changes)?;
        let mut ranges: Vec<_> = mapped
            .iter(0)
            .filter(|range| {
                range.from < range.to || range.value.try_range(range.from..range.to).is_ok()
            })
            .cloned()
            .collect();
        if let Some(extent) = extent {
            let area = ScanArea {
                text: lines,
                state: Some(update.state),
                viewport: update.viewport,
            };
            for visible in update.viewport.visible_ranges() {
                let from = extent.start.max(visible.start);
                let to = extent.end.min(visible.end);
                if from <= to {
                    self.rescan(area, visible, from..to, &mut ranges)?;
                }
            }
        }
        make_set(text, ranges, true)
    }

    fn scan(
        &self,
        area: ScanArea<'_>,
        range: ByteRange<usize>,
        ranges: &mut Vec<Range<Decoration<W>>>,
    ) -> Result<(), EditError> {
        for_line_fragment(area.text, range, |base, fragment| {
            self.scan_fragment(area, base, fragment, 0..fragment.len(), ranges)
        })
    }

    fn scan_fragment(
        &self,
        area: ScanArea<'_>,
        base: usize,
        fragment: &str,
        scan: ByteRange<usize>,
        ranges: &mut Vec<Range<Decoration<W>>>,
    ) -> Result<(), EditError> {
        let mut offset = scan.start;
        while offset < scan.end {
            let captures = match &self.style {
                MatchStyle::Fixed(_) => None,
                MatchStyle::Callback(_) => {
                    let Some(captures) = self.regexp.captures_at(fragment, offset) else {
                        break;
                    };
                    Some(captures)
                }
            };
            let found = match &captures {
                Some(captures) => captures.get(0),
                None => self.regexp.find_at(fragment, offset),
            };
            let Some(found) = found.filter(|found| found.start() < scan.end) else {
                break;
            };
            offset = found.end();
            if found.is_empty() {
                offset += fragment[offset..]
                    .chars()
                    .next()
                    .expect("match before scan end")
                    .len_utf8();
                continue;
            }
            let from = base + found.start();
            let to = base + found.end();
            match &self.style {
                MatchStyle::Fixed(decoration) => {
                    ranges.push(Range::new(from, to, decoration.clone()))
                }
                MatchStyle::Callback(callback) => {
                    let items = callback(MatchContext {
                        text: area.state.map_or(area.text, EditorState::text),
                        state: area.state,
                        viewport: area.viewport,
                        range: from..to,
                        captures: captures.expect("callback captures"),
                        fragment_start: base,
                    });
                    if items.iter().any(|item| item.from < from || item.to > to) {
                        return Err(EditError::InvalidRange);
                    }
                    ranges.extend(items);
                }
            }
        }
        Ok(())
    }

    fn rescan(
        &self,
        area: ScanArea<'_>,
        visible: &ByteRange<usize>,
        changed: ByteRange<usize>,
        ranges: &mut Vec<Range<Decoration<W>>>,
    ) -> Result<(), EditError> {
        let first = area
            .text
            .line_at(changed.start)
            .expect("valid changed range");
        let last = area.text.line_at(changed.end).expect("valid changed range");
        let mut start = visible.start.max(first.range.start);
        let mut end = visible.end.min(last.range.end);
        if let Some(boundary) = &self.boundary {
            let from = changed.start.min(first.range.end) - first.range.start;
            let to = changed.end.min(last.range.end) - last.range.start;
            if let Some((offset, ch)) = first.text[..from]
                .char_indices()
                .rev()
                .find(|(_, ch)| boundary.is_match(ch.encode_utf8(&mut [0; 4])))
            {
                start = first.range.start + offset + ch.len_utf8();
            }
            if let Some((offset, _)) = last.text[to..]
                .char_indices()
                .find(|(_, ch)| boundary.is_match(ch.encode_utf8(&mut [0; 4])))
            {
                end = last.range.start + to + offset;
            }
        }
        if start > end {
            return Ok(());
        }
        ranges.retain(|range| range.from < start || range.to > end);
        if first.number == last.number {
            // Keep the full line for anchors. The limit bounds match starts.
            self.scan_fragment(
                area,
                first.range.start,
                first.text,
                start - first.range.start..end - first.range.start,
                ranges,
            )
        } else {
            self.scan(area, start..end, ranges)
        }
    }
}

/// Put a nonempty plain-text label at offset zero when the document is empty.
/// The host draws the `cm-placeholder` class and sets accessibility attributes.
/// The placeholder is not document text.
pub fn placeholder(text: &Text, label: &str) -> DecorationSet<String> {
    if !text.is_empty() || label.is_empty() {
        return DecorationSet::empty();
    }
    let widget = Decoration::widget(
        label.to_owned(),
        WidgetSpec {
            side: 1,
            ..WidgetSpec::default()
        },
    );
    make_set(text, vec![Range::new(0, 0, widget)], false).expect("valid placeholder")
}

type PlaceholderLabel = Arc<str>;
static PLACEHOLDERS: LazyLock<Facet<PlaceholderLabel, Vec<PlaceholderLabel>>> =
    LazyLock::new(Facet::collect);

/// Install a plain-text placeholder. An empty label contributes an empty
/// accessibility value but no widget. Reusing an extension adds one widget.
pub fn placeholder_extension(label: impl Into<Arc<str>>) -> Extension {
    PLACEHOLDERS.of(label.into())
}

/// Plain text for a host's accessibility attribute and visual placeholder
/// widgets. The widgets are not selectable and are hidden from accessibility.
#[derive(Debug)]
pub struct PlaceholderProjection {
    pub aria_placeholder: Option<Arc<str>>,
    /// Separate widget sets in precedence order, as in the pinned view.
    pub widgets: Vec<DecorationSet<String>>,
}

/// Read the first placeholder label even when the document is nonempty.
/// Project all nonempty labels only while the document has no text.
pub fn placeholder_projection(
    state: &EditorState,
) -> Result<PlaceholderProjection, ConfigurationError> {
    let labels = state.context().facet(&PLACEHOLDERS)?;
    let widgets = if state.text().is_empty() {
        labels
            .iter()
            .filter(|label| !label.is_empty())
            .map(|label| placeholder(state.text(), label))
            .collect()
    } else {
        Vec::new()
    };
    Ok(PlaceholderProjection {
        aria_placeholder: labels.first().cloned(),
        widgets,
    })
}

fn make_set<W: PartialEq>(
    text: &Text,
    ranges: Vec<Range<Decoration<W>>>,
    sort: bool,
) -> Result<DecorationSet<W>, EditError> {
    DecorationSet::new(text, ranges, sort).map_err(|_| EditError::InvalidRange)
}

fn check_viewport(text: &Text, viewport: &Viewport) -> Result<(), EditError> {
    if !text.is_valid_range(viewport.interval())
        || viewport
            .visible_ranges()
            .iter()
            .any(|range| !text.is_valid_range(range.clone()))
    {
        return Err(EditError::InvalidRange);
    }
    Ok(())
}

fn match_ranges(
    text: &Text,
    viewport: &Viewport,
    max_length: usize,
) -> Result<Vec<ByteRange<usize>>, EditError> {
    check_viewport(text, viewport)?;
    let visible = viewport.visible_ranges();
    if visible == std::slice::from_ref(&viewport.interval()) || max_length == 0 {
        return Ok(visible.to_vec());
    }
    let mut ranges: Vec<ByteRange<usize>> = Vec::new();
    for range in visible {
        let first = text.line_at(range.start).unwrap();
        let last = text.line_at(range.end).unwrap();
        let left = range.start.min(first.range.end);
        let right = range.end.min(last.range.end);
        let from = left
            - context_bytes(
                first.text[..left - first.range.start].chars().rev(),
                max_length,
            );
        let to = range
            .end
            .max(right + context_bytes(last.text[right - last.range.start..].chars(), max_length));
        if let Some(previous) = ranges.last_mut().filter(|previous| previous.end >= from) {
            previous.end = to;
        } else {
            ranges.push(from..to);
        }
    }
    Ok(ranges)
}

fn context_bytes(chars: impl Iterator<Item = char>, mut units: usize) -> usize {
    let mut bytes = 0;
    for ch in chars {
        let Some(remaining) = units.checked_sub(ch.len_utf16()) else {
            break;
        };
        units = remaining;
        bytes += ch.len_utf8();
    }
    bytes
}

fn for_line_fragment(
    text: &Text,
    range: ByteRange<usize>,
    mut visit: impl FnMut(usize, &str) -> Result<(), EditError>,
) -> Result<(), EditError> {
    let mut pos = range.start;
    let mut cursor = text.cursor_range(pos, range.end).unwrap();
    while let Some(chunk) = cursor.next() {
        if !cursor.line_break() {
            visit(pos, chunk)?;
        }
        pos += chunk.len();
    }
    Ok(())
}

fn exceeds_update_limit(text: &Text, range: ByteRange<usize>) -> bool {
    let mut remaining = UPDATE_LIMIT;
    let mut cursor = text.cursor_range(range.start, range.end).unwrap();
    while let Some(chunk) = cursor.next() {
        let units = if cursor.line_break() {
            1
        } else {
            chunk.encode_utf16().take(remaining + 1).count()
        };
        let Some(left) = remaining.checked_sub(units) else {
            return true;
        };
        remaining = left;
    }
    false
}
