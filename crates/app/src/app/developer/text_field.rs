//! Plain text input for feedback, search, and the command palette.

use std::{borrow::Cow, ops::Range};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementInputHandler, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, Global, KeyBinding, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Rgba, ScrollHandle, ShapedLine, Subscription, TextRun,
    UTF16Selection, Window, actions, canvas, div, fill, point, prelude::*, px, rgb, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    editor::{Copy, Cut, Paste, SelectAll},
    theme::Theme,
};

const MAX_BYTES: usize = 16 * 1024;
const FEEDBACK_CONTEXT: &str = "FeedbackTextField";
const SEARCH_CONTEXT: &str = "SearchTextField";
const PALETTE_CONTEXT: &str = "PaletteTextField";

/// The user pressed Enter in the search field without an active IME composition.
pub struct SearchSubmitted;

pub(crate) enum PaletteInput {
    Previous,
    Next,
    Run,
}

#[derive(Clone, Copy)]
enum Mode {
    Feedback,
    Search,
    Palette,
}

struct KeysInstalled;
impl Global for KeysInstalled {}

actions!(
    feedback_text_field,
    [
        Left,
        Right,
        SelectLeft,
        SelectRight,
        Up,
        Down,
        SelectUp,
        SelectDown,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        Backspace,
        Delete,
        Newline,
        SubmitSearch,
        RunPalette,
        KeepPaletteFocus,
    ]
);

pub struct TextField {
    mode: Mode,
    focus: FocusHandle,
    blur: Option<Subscription>,
    text: String,
    editable: bool,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    selecting: bool,
    layout: Option<Layout>,
    preferred_x: Option<Pixels>,
    scroll: ScrollHandle,
    reveal_caret: bool,
}

impl TextField {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_mode(Mode::Feedback, cx)
    }

    pub fn search(cx: &mut Context<Self>) -> Self {
        Self::with_mode(Mode::Search, cx)
    }

    pub(crate) fn palette(cx: &mut Context<Self>) -> Self {
        Self::with_mode(Mode::Palette, cx)
    }

    pub(in crate::app) fn bind_keys(cx: &mut App) {
        cx.set_global(KeysInstalled);
        for context in [FEEDBACK_CONTEXT, SEARCH_CONTEXT, PALETTE_CONTEXT].map(Some) {
            cx.bind_keys([
                KeyBinding::new("left", Left, context),
                KeyBinding::new("right", Right, context),
                KeyBinding::new("shift-left", SelectLeft, context),
                KeyBinding::new("shift-right", SelectRight, context),
                KeyBinding::new("cmd-left", LineStart, context),
                KeyBinding::new("cmd-right", LineEnd, context),
                KeyBinding::new("cmd-shift-left", SelectLineStart, context),
                KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
                KeyBinding::new("cmd-a", SelectAll, context),
                KeyBinding::new("backspace", Backspace, context),
                KeyBinding::new("delete", Delete, context),
                KeyBinding::new("cmd-c", Copy, context),
                KeyBinding::new("cmd-x", Cut, context),
                KeyBinding::new("cmd-v", Paste, context),
            ]);
        }
        let feedback = Some(FEEDBACK_CONTEXT);
        cx.bind_keys([
            KeyBinding::new("up", Up, feedback),
            KeyBinding::new("down", Down, feedback),
            KeyBinding::new("shift-up", SelectUp, feedback),
            KeyBinding::new("shift-down", SelectDown, feedback),
            KeyBinding::new("enter", Newline, feedback),
            KeyBinding::new("shift-enter", Newline, feedback),
            KeyBinding::new("enter", SubmitSearch, Some(SEARCH_CONTEXT)),
            KeyBinding::new("shift-enter", Newline, Some(SEARCH_CONTEXT)),
            KeyBinding::new("up", Up, Some(PALETTE_CONTEXT)),
            KeyBinding::new("down", Down, Some(PALETTE_CONTEXT)),
            KeyBinding::new("enter", RunPalette, Some(PALETTE_CONTEXT)),
            KeyBinding::new("tab", KeepPaletteFocus, Some(PALETTE_CONTEXT)),
            KeyBinding::new("shift-tab", KeepPaletteFocus, Some(PALETTE_CONTEXT)),
        ]);
    }

    fn with_mode(mode: Mode, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<KeysInstalled>() {
            Self::bind_keys(cx);
        }
        Self {
            mode,
            focus: cx.focus_handle(),
            blur: None,
            text: String::new(),
            editable: true,
            anchor: 0,
            cursor: 0,
            marked: None,
            selecting: false,
            layout: None,
            preferred_x: None,
            scroll: ScrollHandle::new(),
            reveal_caret: false,
        }
    }

    /// Returns the plain text, limited to 16 KiB of UTF-8, including any active composition.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub(super) fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn replacement_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        range
            .map(|range| from_utf16(&self.text, range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection())
    }

    fn replace(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if !self.editable {
            return None;
        }
        let text = match self.mode {
            Mode::Feedback => Cow::Borrowed(text),
            Mode::Search | Mode::Palette => single_line(text),
        };
        let available = MAX_BYTES - (self.text.len() - range.len());
        let inserted = &text[..text.floor_char_boundary(available)];
        // At the limit, do not erase the selection if none of the input fits.
        if inserted.is_empty() && !text.is_empty() {
            return None;
        }
        let inserted_range = range.start..range.start + inserted.len();
        self.text.replace_range(range, inserted);
        self.layout = None;
        self.move_to(inserted_range.end, false, cx);
        Some(inserted_range)
    }

    fn move_to(&mut self, cursor: usize, extend: bool, cx: &mut Context<Self>) {
        self.cursor = cursor;
        if !extend {
            self.anchor = cursor;
        }
        self.marked = None;
        self.preferred_x = None;
        self.reveal_caret = true;
        cx.notify();
    }

    fn step(&self, right: bool) -> usize {
        let mut boundaries = self.text.grapheme_indices(true);
        if right {
            boundaries
                .find(|(offset, _)| *offset > self.cursor)
                .map_or(self.text.len(), |(offset, _)| offset)
        } else {
            boundaries
                .rfind(|(offset, _)| *offset < self.cursor)
                .map_or(0, |(offset, _)| offset)
        }
    }

    fn horizontal(&mut self, right: bool, extend: bool, cx: &mut Context<Self>) {
        let selection = self.selection();
        let cursor = if extend || selection.is_empty() {
            self.step(right)
        } else if right {
            selection.end
        } else {
            selection.start
        };
        self.move_to(cursor, extend, cx);
    }

    fn vertical(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else { return };
        let row = layout.row_index(self.cursor);
        let x = self
            .preferred_x
            .unwrap_or_else(|| layout.rows[row].x(self.cursor));
        let next = if down { row + 1 } else { row.wrapping_sub(1) };
        let cursor = layout
            .rows
            .get(next)
            .map_or(if down { self.text.len() } else { 0 }, |row| {
                row.index_at(x)
            });
        self.move_to(cursor, extend, cx);
        self.preferred_x = Some(x);
    }

    fn line_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        if let Some(layout) = &self.layout {
            let range = &layout.rows[layout.row_index(self.cursor)].range;
            self.move_to(if end { range.end } else { range.start }, extend, cx);
        }
    }

    fn remove(&mut self, right: bool, cx: &mut Context<Self>) {
        let mut range = self.selection();
        if range.is_empty() {
            let other = self.step(right);
            range = self.cursor.min(other)..self.cursor.max(other);
        }
        if !range.is_empty() {
            self.replace(range, "", cx);
        }
    }

    fn copy(&self, cx: &mut Context<Self>) -> bool {
        let range = self.selection();
        if range.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.text[range].to_owned()));
        true
    }

    fn palette_input(&self, input: PaletteInput, cx: &mut Context<Self>) {
        if self.editable && !self.is_composing() {
            cx.emit(input);
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::Palette) && self.is_composing() {
            return;
        }
        self.focus(window);
        if let Some(layout) = &self.layout {
            self.selecting = true;
            self.move_to(layout.index_at(event.position), event.modifiers.shift, cx);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting
            && let Some(layout) = &self.layout
        {
            self.move_to(layout.index_at(event.position), true, cx);
        }
    }
}

impl EventEmitter<SearchSubmitted> for TextField {}
impl EventEmitter<PaletteInput> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn single_line(text: &str) -> Cow<'_, str> {
    const LINE_BREAKS: [char; 7] = [
        '\n', '\r', '\x0B', '\x0C', '\u{0085}', '\u{2028}', '\u{2029}',
    ];
    if text.contains(LINE_BREAKS) {
        // Replace one code unit at a time to preserve the IME's UTF-16 offsets.
        Cow::Owned(text.replace(LINE_BREAKS, " "))
    } else {
        Cow::Borrowed(text)
    }
}

// macOS uses UTF-16 indices. Round offsets inside a surrogate pair down to a valid boundary.
fn from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    let byte_offset = |offset| {
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            units += ch.len_utf16();
            if units > offset {
                return byte;
            }
        }
        text.len()
    };
    byte_offset(range.start.min(range.end))..byte_offset(range.end)
}

fn to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    let start = text[..range.start].encode_utf16().count();
    start..start + text[range].encode_utf16().count()
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let bytes = from_utf16(&self.text, range);
        *actual = Some(to_utf16(&self.text, bytes.clone()));
        Some(self.text[bytes].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: to_utf16(&self.text, self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| to_utf16(&self.text, range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(self.replacement_range(range), text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(inserted) = self.replace(self.replacement_range(range), text, cx)
            && !inserted.is_empty()
        {
            if let Some(selected) = selected {
                let relative = from_utf16(&self.text[inserted.clone()], selected);
                self.anchor = inserted.start + relative.start;
                self.cursor = inserted.start + relative.end;
            }
            self.marked = Some(inserted);
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let bytes = from_utf16(&self.text, range);
        let row = layout.row_index(bytes.start);
        let start = layout.rows[row].x(bytes.start);
        let end = layout.rows[row].x(bytes.end);
        Some(layout.rect(row, start.min(end), (end - start).abs().max(px(1.))))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        if !self.scroll.bounds().contains(&position) {
            return None;
        }
        let byte = self.layout.as_ref()?.index_at(position);
        Some(self.text[..byte].encode_utf16().count())
    }
}

// Keep byte offsets in the original string, including CRLF pasted from other apps.
fn line_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    text.grapheme_indices(true)
        .filter(|(_, grapheme)| matches!(*grapheme, "\n" | "\r" | "\r\n"))
        .chain(std::iter::once((text.len(), "")))
        .map(move |(end, newline)| {
            let range = start..end;
            start = end + newline.len();
            range
        })
}

struct Row {
    range: Range<usize>,
    // Do not allocate a shaped line for empty rows.
    line: Option<Box<ShapedLine>>,
}

impl Row {
    fn x(&self, byte: usize) -> Pixels {
        self.line.as_ref().map_or(px(0.), |line| {
            line.x_for_index(byte.min(self.range.end) - self.range.start)
        })
    }

    fn index_at(&self, x: Pixels) -> usize {
        let Some(line) = &self.line else {
            return self.range.start;
        };
        let Some(byte) = line.index_for_x(x) else {
            return self.range.end;
        };
        // GPUI returns a glyph position; snap it to a whole grapheme for editing.
        let (start, grapheme) = line
            .text
            .grapheme_indices(true)
            .rfind(|(start, _)| *start <= byte)
            .expect("nonempty row has a grapheme");
        let end = start + grapheme.len();
        self.range.start
            + if (x - line.x_for_index(start)).abs() <= (x - line.x_for_index(end)).abs() {
                start
            } else {
                end
            }
    }
}

#[derive(Clone, Copy)]
struct FieldColors {
    surface: Rgba,
    border: Rgba,
    text: Rgba,
    accent: Rgba,
    selection: Rgba,
}

struct Layout {
    origin: Point<Pixels>,
    line_height: Pixels,
    rows: Vec<Row>,
}

impl Layout {
    fn shape(text: &str, color: Rgba, window: &Window) -> Self {
        let mut run = TextRun {
            len: 0,
            font: window.text_style().font(),
            color: color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let rows = line_ranges(text)
            .map(|range| {
                let line = (!range.is_empty()).then(|| {
                    run.len = range.len();
                    Box::new(window.text_system().shape_line(
                        text[range.clone()].to_owned().into(),
                        px(14.),
                        std::slice::from_ref(&run),
                        None,
                    ))
                });
                Row { range, line }
            })
            .collect();
        Self {
            origin: Point::default(),
            line_height: px(20.),
            rows,
        }
    }

    fn row_index(&self, byte: usize) -> usize {
        self.rows.partition_point(|row| row.range.start <= byte) - 1
    }

    fn row_at_y(&self, y: Pixels) -> usize {
        ((y - self.origin.y) / self.line_height) as usize
    }

    fn index_at(&self, position: Point<Pixels>) -> usize {
        if position.y < self.origin.y {
            return 0;
        }
        self.rows.get(self.row_at_y(position.y)).map_or_else(
            || self.rows.last().unwrap().range.end,
            |row| row.index_at(position.x - self.origin.x),
        )
    }

    fn rect(&self, row: usize, x: Pixels, width: Pixels) -> Bounds<Pixels> {
        Bounds::new(
            self.origin + point(x, self.line_height * row),
            size(width, self.line_height),
        )
    }

    fn paint(
        &self,
        selection: Range<usize>,
        marked: Option<Range<usize>>,
        colors: FieldColors,
        window: &mut Window,
        cx: &mut App,
    ) {
        let clip = window.content_mask().bounds;
        let first = self.row_at_y(clip.top()).min(self.rows.len());
        let last = (self.row_at_y(clip.bottom()) + 1).min(self.rows.len());
        for i in first..last {
            let row = &self.rows[i];
            let next_start = self
                .rows
                .get(i + 1)
                .map_or(row.range.end, |row| row.range.start);
            let start = selection.start.max(row.range.start);
            let end = selection.end.min(next_start);
            if start < end {
                let left = row.x(start);
                let right = row.x(end);
                let newline_width = if end > row.range.end { px(5.) } else { px(0.) };
                window.paint_quad(fill(
                    self.rect(i, left.min(right), (right - left).abs() + newline_width),
                    colors.selection,
                ));
            }
            if let Some(line) = &row.line {
                let _ = line.paint(
                    self.rect(i, px(0.), px(0.)).origin,
                    self.line_height,
                    window,
                    cx,
                );
            }
            if let Some(marked) = &marked {
                let start = marked.start.max(row.range.start);
                let end = marked.end.min(row.range.end);
                if start < end {
                    let left = row.x(start);
                    let right = row.x(end);
                    let mut underline =
                        self.rect(i, left.min(right), (right - left).abs().max(px(1.)));
                    underline.origin.y += self.line_height - px(2.);
                    underline.size.height = px(1.);
                    window.paint_quad(fill(underline, colors.accent));
                }
            }
        }
    }
}

fn reveal_caret(scroll: &ScrollHandle, caret: Bounds<Pixels>) -> bool {
    let viewport = scroll.bounds().inset(px(8.));
    if viewport.size.width <= px(0.) || viewport.size.height < caret.size.height {
        return false;
    }
    let current = scroll.offset();
    let limit = scroll.max_offset();
    let adjust = |start: Pixels, end: Pixels, low: Pixels, high: Pixels| {
        if start < low {
            low - start
        } else {
            (high - end).min(px(0.))
        }
    };
    let next = point(
        (current.x
            + adjust(
                caret.left(),
                caret.right(),
                viewport.left(),
                viewport.right(),
            ))
        .clamp(-limit.width, px(0.)),
        (current.y
            + adjust(
                caret.top(),
                caret.bottom(),
                viewport.top(),
                viewport.bottom(),
            ))
        .clamp(-limit.height, px(0.)),
    );
    if next == current {
        return false;
    }
    scroll.set_offset(next);
    true
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.blur.is_none() {
            self.blur = Some(cx.on_blur(&self.focus, window, |field, _, cx| {
                field.marked = None;
                field.selecting = false;
                cx.notify();
            }));
        }
        let (id, key_context, field_height, colors) = match self.mode {
            Mode::Feedback => (
                "feedback-text-field",
                FEEDBACK_CONTEXT,
                px(144.),
                FieldColors {
                    surface: rgb(0xffffff),
                    border: rgb(0xd3dce9),
                    text: rgb(0x172336),
                    accent: rgb(0x1463b7),
                    selection: rgba(0x4a86ed50),
                },
            ),
            Mode::Search | Mode::Palette => {
                let theme = Theme::for_window(window);
                let (id, context) = if matches!(self.mode, Mode::Palette) {
                    ("palette-text-field", PALETTE_CONTEXT)
                } else {
                    ("search-text-field", SEARCH_CONTEXT)
                };
                (
                    id,
                    context,
                    px(38.),
                    FieldColors {
                        surface: theme.surface,
                        border: if self.focus.is_focused(window) {
                            theme.accent
                        } else {
                            theme.border
                        },
                        text: theme.text,
                        accent: theme.accent,
                        selection: theme.selection,
                    },
                )
            }
        };
        let layout = Layout::shape(self.text(), colors.text, window);
        let width = layout
            .rows
            .iter()
            .filter_map(|row| row.line.as_ref().map(|line| line.width))
            .max()
            .unwrap_or(px(0.));
        let height = layout.line_height * layout.rows.len();
        let field = cx.entity();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .w_full()
            .h(field_height)
            .min_w_0()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .map(|element| match self.mode {
                Mode::Feedback => element.overflow_scroll(),
                Mode::Search | Mode::Palette => element
                    .tab_index(0)
                    .flex_none()
                    .overflow_y_hidden()
                    .overflow_x_scroll(),
            })
            .track_scroll(&self.scroll)
            .cursor(CursorStyle::IBeam)
            .track_focus(&self.focus)
            .key_context(key_context)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|e, _, _, _| e.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|e, _, _, _| e.selecting = false),
            )
            .on_action(cx.listener(|e, _: &Left, _, cx| e.horizontal(false, false, cx)))
            .on_action(cx.listener(|e, _: &Right, _, cx| e.horizontal(true, false, cx)))
            .on_action(cx.listener(|e, _: &SelectLeft, _, cx| e.horizontal(false, true, cx)))
            .on_action(cx.listener(|e, _: &SelectRight, _, cx| e.horizontal(true, true, cx)))
            .on_action(cx.listener(|e, _: &Up, _, cx| {
                if matches!(e.mode, Mode::Palette) {
                    e.palette_input(PaletteInput::Previous, cx);
                } else {
                    e.vertical(false, false, cx);
                }
            }))
            .on_action(cx.listener(|e, _: &Down, _, cx| {
                if matches!(e.mode, Mode::Palette) {
                    e.palette_input(PaletteInput::Next, cx);
                } else {
                    e.vertical(true, false, cx);
                }
            }))
            .on_action(cx.listener(|e, _: &SelectUp, _, cx| e.vertical(false, true, cx)))
            .on_action(cx.listener(|e, _: &SelectDown, _, cx| e.vertical(true, true, cx)))
            .on_action(cx.listener(|e, _: &LineStart, _, cx| e.line_edge(false, false, cx)))
            .on_action(cx.listener(|e, _: &LineEnd, _, cx| e.line_edge(true, false, cx)))
            .on_action(cx.listener(|e, _: &SelectLineStart, _, cx| e.line_edge(false, true, cx)))
            .on_action(cx.listener(|e, _: &SelectLineEnd, _, cx| e.line_edge(true, true, cx)))
            .on_action(cx.listener(|e, _: &SelectAll, _, cx| {
                e.anchor = 0;
                e.move_to(e.text.len(), true, cx);
            }))
            .on_action(cx.listener(|e, _: &Backspace, _, cx| e.remove(false, cx)))
            .on_action(cx.listener(|e, _: &Delete, _, cx| e.remove(true, cx)))
            .on_action(cx.listener(|e, _: &Newline, window, cx| {
                if matches!(e.mode, Mode::Feedback) {
                    e.replace_text_in_range(None, "\n", window, cx);
                }
            }))
            .when(matches!(self.mode, Mode::Search), |element| {
                element.on_action(cx.listener(|e, _: &SubmitSearch, _, cx| {
                    if e.editable && !e.is_composing() {
                        cx.emit(SearchSubmitted);
                    }
                }))
            })
            .when(matches!(self.mode, Mode::Palette), |element| {
                element
                    .on_action(cx.listener(|e, _: &RunPalette, _, cx| {
                        e.palette_input(PaletteInput::Run, cx);
                    }))
                    .on_action(|_: &KeepPaletteFocus, _, _| {})
            })
            .on_action(cx.listener(|e, _: &Copy, _, cx| {
                e.copy(cx);
            }))
            .on_action(cx.listener(|e, _: &Cut, _, cx| {
                if e.editable && e.copy(cx) {
                    e.replace(e.selection(), "", cx);
                }
            }))
            .on_action(cx.listener(|e, _: &Paste, window, cx| {
                if e.editable
                    && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                {
                    e.replace_text_in_range(None, &text, window, cx);
                }
            }))
            .child(
                canvas(
                    move |bounds, _, _| Layout {
                        origin: bounds.origin,
                        ..layout
                    },
                    move |bounds, layout, window, cx| {
                        let input = field.read(cx);
                        let focus = input.focus.clone();
                        let selection = input.selection();
                        let marked = input.marked.clone();
                        let row = layout.row_index(input.cursor);
                        let caret = layout.rect(row, layout.rows[row].x(input.cursor), px(2.));
                        let focused = focus.is_focused(window);
                        if input.editable {
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, field.clone()),
                                cx,
                            );
                        }
                        let show_caret = input.editable && focused && selection.is_empty();
                        layout.paint(selection, marked, colors, window, cx);
                        if show_caret {
                            window.paint_quad(fill(caret, colors.accent));
                        }
                        field.update(cx, |field, _| {
                            field.layout = Some(layout);
                            if std::mem::take(&mut field.reveal_caret)
                                && focused
                                && reveal_caret(&field.scroll, caret)
                            {
                                window.on_next_frame(|window, _| window.refresh());
                            }
                        });
                    },
                )
                .w(width + px(7.))
                .min_w_full()
                .h(height),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{Image, ImageFormat, TestAppContext};

    use super::*;

    #[test]
    fn lines_preserve_mixed_line_endings_and_trailing_empty_row() {
        let text = "é\r\n\n好\r";
        assert_eq!(
            line_ranges(text).collect::<Vec<_>>(),
            [0..2, 4..4, 5..8, 9..9]
        );
        assert_eq!(
            line_ranges("").collect::<Vec<_>>(),
            std::iter::once(0..0).collect::<Vec<_>>()
        );
    }

    #[gpui::test]
    fn keyboard_edits_graphemes_and_pastes_plain_multiline_text(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::new(cx));
        cx.update(|window, cx| {
            field.read(cx).focus(window);
            cx.write_to_clipboard(ClipboardItem::new_string("**好**\r\n".into()));
        });
        cx.simulate_input("e\u{301}👩‍💻");
        cx.simulate_keystrokes("left backspace enter cmd-v");
        cx.update(|_, cx| assert_eq!(field.read(cx).text(), "\n**好**\r\n👩‍💻"));
        cx.simulate_keystrokes("delete cmd-a cmd-x");
        cx.update(|_, cx| {
            assert_eq!(field.read(cx).text(), "");
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("\n**好**\r\n")
            );
        });
    }

    #[gpui::test]
    fn native_edit_menu_actions_operate_on_the_focused_feedback_field(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::new(cx));
        cx.update(|window, cx| field.read(cx).focus(window));
        cx.simulate_input("feedback");
        cx.dispatch_action(crate::editor::SelectAll);
        cx.dispatch_action(crate::editor::Copy);
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("feedback")
            );
            assert_eq!(field.read(cx).text(), "feedback");
        });
        cx.dispatch_action(crate::editor::Cut);
        cx.update(|_, cx| assert_eq!(field.read(cx).text(), ""));
        cx.dispatch_action(crate::editor::Paste);
        cx.update(|_, cx| assert_eq!(field.read(cx).text(), "feedback"));
    }

    #[gpui::test]
    fn image_only_clipboard_does_not_replace_selected_text(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::new(cx));
        cx.update(|window, cx| {
            field.read(cx).focus(window);
            cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                ImageFormat::Png,
                Vec::new(),
            )));
        });
        cx.simulate_input("keep this");
        cx.simulate_keystrokes("cmd-a cmd-v");
        cx.update(|_, cx| {
            let field = field.read(cx);
            assert_eq!(field.text(), "keep this");
            assert_eq!(field.selection(), 0..9);
        });
    }

    #[gpui::test]
    fn caret_scrolls_into_view_and_vertical_navigation_keeps_column(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::new(cx));
        cx.simulate_resize(size(px(360.), px(240.)));
        let text = format!("{}{}", "abcd\nx\nabcd\n".repeat(20), "z".repeat(200));
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.focus(window);
                field.replace_text_in_range(None, &text, window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let field = field.read(cx);
            assert!(field.scroll.offset().x < px(0.));
            assert!(field.scroll.offset().y < px(0.));
        });
        cx.simulate_keystrokes("cmd-a left right right right down shift-down");
        cx.update(|_, cx| assert_eq!(field.read(cx).selection(), 6..10));
    }

    #[gpui::test]
    fn ime_selection_is_relative_to_marked_text_and_commit_replaces_it(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::new(cx));
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "😀\n字", window, cx);
                field.replace_and_mark_text_in_range(Some(3..3), "𠮷かな", Some(2..3), window, cx);
                assert_eq!(field.text(), "😀\n𠮷かな字");
                assert_eq!(field.marked_text_range(window, cx), Some(3..7));
                assert_eq!(
                    field.selected_text_range(false, window, cx).unwrap().range,
                    5..6
                );
                field.replace_text_in_range(None, "吉", window, cx);
                assert_eq!(field.text(), "😀\n吉字");
                assert_eq!(field.marked_text_range(window, cx), None);
            })
        });
    }

    #[gpui::test]
    fn search_uses_one_row_for_paste_and_scrolls_horizontally(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::search(cx));
        cx.simulate_resize(size(px(160.), px(160.)));
        let long = "hello".repeat(100);
        cx.update(|window, cx| {
            field.read(cx).focus(window);
            cx.write_to_clipboard(ClipboardItem::new_string(format!("{long}\r\n好\u{2028}👩‍💻")));
        });
        cx.simulate_keystrokes("cmd-v");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let field = field.read(cx);
            assert_eq!(field.text(), format!("{long}  好 👩‍💻"));
            assert_eq!(field.layout.as_ref().unwrap().rows.len(), 1);
            assert!(field.scroll.offset().x < px(0.));
            assert_eq!(field.scroll.offset().y, px(0.));
        });
        cx.simulate_keystrokes("shift-left cmd-c cmd-x");
        cx.update(|_, cx| {
            assert_eq!(field.read(cx).text(), format!("{long}  好 "));
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("👩‍💻")
            );
        });
    }

    #[gpui::test]
    fn search_notifies_during_composition_but_submits_only_after_commit(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::search(cx));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let (_observe, _subscribe) = cx.update(|window, cx| {
            field.read(cx).focus(window);
            let observe = cx.observe(&field, {
                let observed = observed.clone();
                move |field, cx| {
                    let field = field.read(cx);
                    observed
                        .borrow_mut()
                        .push((field.text().to_owned(), field.is_composing()));
                }
            });
            let subscribe = cx.subscribe(&field, {
                let submitted = submitted.clone();
                move |field, _: &SearchSubmitted, cx| {
                    submitted
                        .borrow_mut()
                        .push(field.read(cx).text().to_owned());
                }
            });
            (observe, subscribe)
        });
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_and_mark_text_in_range(
                    None,
                    "𠮷\u{2028}かな",
                    Some(3..4),
                    window,
                    cx,
                );
                assert_eq!(field.marked_text_range(window, cx), Some(0..5));
                assert_eq!(
                    field.selected_text_range(false, window, cx).unwrap().range,
                    3..4
                );
            });
        });
        cx.simulate_keystrokes("enter");
        assert!(submitted.borrow().is_empty());
        assert_eq!(observed.borrow().last(), Some(&("𠮷 かな".into(), true)));

        cx.simulate_input("吉");
        assert_eq!(observed.borrow().last(), Some(&("吉".into(), false)));
        cx.simulate_keystrokes("shift-enter");
        assert!(submitted.borrow().is_empty());
        cx.simulate_keystrokes("enter");
        assert_eq!(*submitted.borrow(), ["吉"]);
        cx.update(|_, cx| assert_eq!(field.read(cx).text(), "吉"));
    }

    #[gpui::test]
    fn byte_limit_keeps_whole_scalars_and_respects_replacement_capacity(cx: &mut TestAppContext) {
        let (field, cx) = cx.add_window_view(|_, cx| TextField::new(cx));
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                let prefix = "a".repeat(MAX_BYTES - 5);
                field.replace_text_in_range(None, &format!("{prefix}😀好"), window, cx);
                assert_eq!(field.text(), format!("{prefix}😀"));
                assert_eq!(field.text().len(), MAX_BYTES - 1);
                field.replace_and_mark_text_in_range(None, "é", Some(1..1), window, cx);
                assert!(field.marked.is_none());
                field.replace_and_mark_text_in_range(
                    Some(prefix.len()..prefix.len() + 2),
                    "好é😀",
                    Some(99..99),
                    window,
                    cx,
                );
                assert_eq!(field.text(), format!("{prefix}好é"));
                assert_eq!(field.text().len(), MAX_BYTES);
                assert_eq!(field.cursor, MAX_BYTES);
            })
        });
    }
}
