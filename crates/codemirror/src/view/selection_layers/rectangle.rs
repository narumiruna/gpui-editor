//! Rectangular drag selection and modifier-driven pointer state.

use std::{
    num::NonZeroUsize,
    sync::{Arc, LazyLock},
};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Assoc, ChangeDesc, EditError, EditorSelection, EditorState, Extension, Facet, MapMode,
    SelectionRange, Text,
    view::keymap::{KeyEvent, KeyModifiers},
};

/// Pointer-down input. Button zero is the primary mouse button.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RectangleStartEvent {
    pub button: u8,
    pub modifiers: KeyModifiers,
}

pub type RectangularSelectionFilter = Arc<dyn Fn(RectangleStartEvent) -> bool + Send + Sync>;

#[derive(Clone, Default)]
pub struct RectangularSelectionOptions {
    /// Defaults to the primary button with Alt held.
    pub event_filter: Option<RectangularSelectionFilter>,
}

type Filters = Facet<RectangularSelectionFilter, Vec<RectangularSelectionFilter>>;
static RECTANGLE_FILTERS: LazyLock<Filters> = LazyLock::new(Facet::collect);

/// Register a rectangular pointer drag style. The host calls
/// [`RectangularSelection::start`] when it receives a pointer-down event.
pub fn rectangular_selection(options: RectangularSelectionOptions) -> Extension {
    RECTANGLE_FILTERS.of(options
        .event_filter
        .unwrap_or_else(|| Arc::new(|event| event.button == 0 && event.modifiers.alt)))
}

/// A host hit test, which can use a nearest estimated position outside the
/// visible area. At the logical line end, supply the absolute distance in
/// columns from the viewport reference caret, rounded to the nearest column.
/// Use `None` if that measurement is unavailable. Away from the line end the
/// text determines the column and this measurement is ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RectangleHit {
    pub position: usize,
    pub line_end_column: Option<usize>,
}

const MAX_OFFSET: usize = 2000;

#[derive(Clone, Copy, Debug)]
struct Endpoint {
    line: usize,
    column: Option<usize>,
    units: usize,
}

impl Endpoint {
    fn from_hit(
        lines: &Text,
        tab_size: NonZeroUsize,
        hit: RectangleHit,
    ) -> Result<Self, EditError> {
        let line = lines.line_at(hit.position).ok_or(EditError::InvalidRange)?;
        // A native hit between CR and LF acts like the line end.
        let offset = hit.position.min(line.range.end) - line.range.start;
        let units = line.text[..offset].encode_utf16().count();
        let column = if units > MAX_OFFSET {
            None
        } else if hit.position >= line.range.end {
            hit.line_end_column
        } else {
            Some(
                columns(line.text, tab_size)
                    .find(|&(byte, _)| byte >= offset)
                    .unwrap()
                    .1,
            )
        };
        Ok(Self {
            line: line.number,
            column,
            units,
        })
    }
}

/// One rectangular drag. Retain it until the host ends the gesture and update
/// it for each accepted document change. Lines are logical, not wrapped rows.
/// Native lines end at LF or CRLF, not a lone CR. Columns use Unicode
/// graphemes and tab stops. CodeMirror uses different cluster rules, so some
/// text (such as Hangul Jamo) differs.
#[derive(Clone, Debug)]
pub struct RectangularSelection {
    start: Endpoint,
    initial: EditorSelection,
}

impl RectangularSelection {
    /// Start the first matching registered drag style, or return `None`.
    pub fn start(
        state: &EditorState,
        event: RectangleStartEvent,
        hit: RectangleHit,
    ) -> Result<Option<Self>, EditError> {
        Self::start_with_lines(state, event, hit, state.text())
    }

    /// Start a drag with the host's displayed line breaks. The line view must
    /// preserve the original document's byte offsets.
    pub fn start_with_lines(
        state: &EditorState,
        event: RectangleStartEvent,
        hit: RectangleHit,
        lines: &Text,
    ) -> Result<Option<Self>, EditError> {
        let filters = state.context().facet(&RECTANGLE_FILTERS)?;
        if !filters.iter().any(|filter| filter(event)) {
            return Ok(None);
        }
        Ok(Some(Self {
            start: Endpoint::from_hit(lines, state.tab_size(), hit)?,
            initial: state.selection().clone(),
        }))
    }

    /// Map the start line's original start position before insertions. Retain
    /// the start column and trim its raw offset to the new line's length.
    pub fn update(
        &mut self,
        previous: &EditorState,
        state: &EditorState,
        changes: &ChangeDesc,
    ) -> Result<(), EditError> {
        if changes.is_empty() {
            return Ok(());
        }
        let start = previous
            .text()
            .line(self.start.line)
            .ok_or(EditError::InvalidRange)?
            .range
            .start;
        let mapped = changes
            .map_pos(start, Assoc::Before, MapMode::Simple)
            .ok_or(EditError::InvalidRange)?;
        let line = state
            .text()
            .line_at(mapped)
            .ok_or(EditError::InvalidRange)?;
        let initial = self.initial.map_desc(changes)?;
        self.start.line = line.number;
        self.start.units = self.start.units.min(line.text.encode_utf16().count());
        self.initial = initial;
        Ok(())
    }

    /// Produce a selection, without dispatching it. Short lines get a cursor
    /// at the end. With `multiple`, append the original, mapped ranges before
    /// normalizing, with the first new range as the primary range.
    ///
    /// When column data is missing or an offset exceeds 2000 UTF-16 units,
    /// use the pinned raw-offset fallback. It can run beyond a line's end;
    /// return `InvalidSelection` if a result exceeds the document or splits
    /// a UTF-16 surrogate pair. An empty result restores the original ranges.
    pub fn selection(
        &self,
        state: &EditorState,
        hit: Option<RectangleHit>,
        multiple: bool,
    ) -> Result<EditorSelection, EditError> {
        self.selection_with_lines(state, hit, multiple, state.text())
    }

    /// Produce a selection using the same line view as at the start of the
    /// drag. End the drag if the host replaces that view.
    pub fn selection_with_lines(
        &self,
        state: &EditorState,
        hit: Option<RectangleHit>,
        multiple: bool,
        lines: &Text,
    ) -> Result<EditorSelection, EditError> {
        let Some(hit) = hit else {
            return Ok(self.initial.clone());
        };
        let tab_size = state.tab_size();
        let end = Endpoint::from_hit(lines, tab_size, hit)?;
        let a = self.start;
        let b = end;
        let mut ranges = Vec::new();
        for number in a.line.min(b.line)..=a.line.max(b.line) {
            let line = lines.line(number).ok_or(EditError::InvalidRange)?;
            let range = match (a.column, b.column) {
                (Some(a_col), Some(b_col)) if a.units.max(b.units) <= MAX_OFFSET => {
                    let mut columns = columns(line.text, tab_size);
                    let Some((start, col)) = columns.find(|&(_, col)| col >= a_col.min(b_col))
                    else {
                        ranges.push(SelectionRange::new(line.range.end, line.range.end));
                        continue;
                    };
                    let end = if col >= a_col.max(b_col) {
                        start
                    } else {
                        columns
                            .find(|&(_, col)| col >= a_col.max(b_col))
                            .map_or(line.text.len(), |(end, _)| end)
                    };
                    SelectionRange::new(line.range.start + start, line.range.start + end)
                }
                _ => {
                    let end = a.units.max(b.units);
                    if line.text.encode_utf16().count() > end {
                        continue;
                    }
                    SelectionRange::new(
                        offset_in_units(lines, line.range.start, a.units.min(b.units))
                            .ok_or(EditError::InvalidSelection)?,
                        offset_in_units(lines, line.range.end, end)
                            .ok_or(EditError::InvalidSelection)?,
                    )
                }
            };
            ranges.push(range);
        }
        if ranges.is_empty() {
            return Ok(self.initial.clone());
        }
        if multiple {
            ranges.extend_from_slice(self.initial.ranges());
        }
        EditorSelection::new(ranges, 0)
    }
}

fn columns(line: &str, tab_size: NonZeroUsize) -> impl Iterator<Item = (usize, usize)> {
    std::iter::once((0, 0)).chain(line.grapheme_indices(true).scan(
        0usize,
        move |col, (start, grapheme)| {
            let width = if grapheme == "\t" {
                tab_size.get() - *col % tab_size.get()
            } else {
                1
            };
            *col = col.saturating_add(width);
            Some((start + grapheme.len(), *col))
        },
    ))
}

fn offset_in_units(text: &Text, mut start: usize, mut units: usize) -> Option<usize> {
    for chunk in text.iter_range(start..text.len())? {
        if units == 0 {
            return Some(start);
        }
        if chunk == "\r\n" {
            units -= 1;
            start += 2;
        } else {
            for ch in chunk.chars() {
                if units == 0 {
                    return Some(start);
                }
                units = units.checked_sub(ch.len_utf16())?;
                start += ch.len_utf8();
            }
        }
    }
    (units == 0).then_some(start)
}

/// Modifier that controls the pointer cursor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CrosshairModifier {
    #[default]
    Alt,
    Control,
    Shift,
    Meta,
}

/// Keep one per installed crosshair. Event methods return whether the host
/// must update the pointer. This state does not consume input.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CrosshairCursor {
    modifier: CrosshairModifier,
    down: bool,
}

pub fn crosshair_cursor(modifier: CrosshairModifier) -> CrosshairCursor {
    CrosshairCursor {
        modifier,
        down: false,
    }
}

impl CrosshairCursor {
    pub fn is_down(self) -> bool {
        self.down
    }

    pub fn key_down(&mut self, event: KeyEvent<'_>) -> bool {
        self.set(event.name == self.modifier.name() || self.modifier.held(event.modifiers))
    }

    pub fn key_up(&mut self, event: KeyEvent<'_>) -> bool {
        if event.name == self.modifier.name() || !self.modifier.held(event.modifiers) {
            return self.set(false);
        }
        false
    }

    pub fn pointer_move(&mut self, modifiers: KeyModifiers) -> bool {
        self.set(self.modifier.held(modifiers))
    }

    fn set(&mut self, down: bool) -> bool {
        let changed = self.down != down;
        self.down = down;
        changed
    }
}

impl CrosshairModifier {
    fn name(self) -> &'static str {
        match self {
            Self::Alt => "Alt",
            Self::Control => "Control",
            Self::Shift => "Shift",
            Self::Meta => "Meta",
        }
    }

    fn held(self, modifiers: KeyModifiers) -> bool {
        match self {
            Self::Alt => modifiers.alt,
            Self::Control => modifiers.control,
            Self::Shift => modifiers.shift,
            Self::Meta => modifiers.meta,
        }
    }
}
