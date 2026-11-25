//! One GPUI input surface backed by offsets in the Markdown source.

use std::{collections::HashMap, ops::Range, path::PathBuf};

use crate::{
    document::{Document, Section, SectionKind},
    workspace::Workspace,
};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementInputHandler, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, ScrollHandle, UTF16Selection, Window, actions, canvas, div, fill, point,
    prelude::*, px, rgb, rgba, size,
};

mod commands;
mod layout;
mod toolbar;
#[cfg(test)]
use commands::Command;
use layout::{Graphic, ItemPaint, Layout};
pub(crate) use toolbar::ToolbarAssets;

actions!(
    editor,
    [
        Left,
        Right,
        SelectLeft,
        SelectRight,
        Up,
        Down,
        SelectUp,
        SelectDown,
        SelectAll,
        Backspace,
        Delete,
        Newline,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", Left, Some("Editor")),
        KeyBinding::new("right", Right, Some("Editor")),
        KeyBinding::new("shift-left", SelectLeft, Some("Editor")),
        KeyBinding::new("shift-right", SelectRight, Some("Editor")),
        KeyBinding::new("up", Up, Some("Editor")),
        KeyBinding::new("down", Down, Some("Editor")),
        KeyBinding::new("shift-up", SelectUp, Some("Editor")),
        KeyBinding::new("shift-down", SelectDown, Some("Editor")),
        KeyBinding::new("cmd-a", SelectAll, Some("Editor")),
        KeyBinding::new("backspace", Backspace, Some("Editor")),
        KeyBinding::new("delete", Delete, Some("Editor")),
        KeyBinding::new("enter", Newline, Some("Editor")),
        KeyBinding::new("cmd-c", Copy, Some("Editor")),
        KeyBinding::new("cmd-x", Cut, Some("Editor")),
        KeyBinding::new("cmd-v", Paste, Some("Editor")),
        KeyBinding::new("cmd-z", Undo, Some("Editor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Editor")),
    ]);
}

pub struct Editor {
    focus: FocusHandle,
    document: Document,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    active: Option<Section>,
    selecting: bool,
    layout: Option<Layout>,
    input_enabled: bool,
    scroll: ScrollHandle,
    reveal_caret: bool,
    preferred_x: Option<Pixels>,
    graphics: HashMap<(String, SectionKind, u32), Result<Graphic, String>>,
    images: HashMap<String, ImageEntry>,
    next_image_request: u64,
    image_requests: usize,
    allow_remote_images: bool,
    has_blocked_images: bool,
    path: Option<PathBuf>,
    workspace: Option<Workspace>,
    hover: Option<String>,
    message: Option<String>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

enum ImageEntry {
    Loading(u64),
    Ready(Result<Graphic, String>),
    BudgetExceeded,
}

#[derive(Clone)]
struct Snapshot {
    source: String,
    anchor: usize,
    cursor: usize,
    active: Option<Section>,
}

impl Editor {
    pub fn new(source: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        cx.on_blur(&focus, window, |editor, _, cx| {
            editor.finish_active();
            editor.marked = None;
            cx.notify();
        })
        .detach();
        Self {
            focus,
            document: Document::new(source),
            anchor: 0,
            cursor: 0,
            marked: None,
            active: None,
            selecting: false,
            layout: None,
            input_enabled: true,
            scroll: ScrollHandle::new(),
            reveal_caret: false,
            preferred_x: None,
            graphics: HashMap::new(),
            images: HashMap::new(),
            next_image_request: 0,
            image_requests: 0,
            allow_remote_images: false,
            has_blocked_images: false,
            path: None,
            workspace: None,
            hover: None,
            message: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn source(&self) -> &str {
        self.document.source()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>, workspace: Option<Workspace>) {
        self.path = path;
        self.workspace = workspace;
        self.images.clear();
        self.allow_remote_images = false;
    }

    pub(crate) fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub(crate) fn set_input_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        debug_assert!(enabled || !self.is_composing());
        self.input_enabled = enabled;
        if !enabled {
            self.selecting = false;
        }
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn finish_active(&mut self) {
        if let Some(active) = self.active.take() {
            for position in [&mut self.cursor, &mut self.anchor] {
                if *position > active.source.start && *position < active.source.end {
                    *position = active.source.end;
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            source: self.source().to_owned(),
            anchor: self.anchor,
            cursor: self.cursor,
            active: self.active.clone(),
        }
    }

    fn restore(&mut self, state: Snapshot, cx: &mut Context<Self>) {
        self.document = Document::new(state.source);
        self.cursor = state.cursor;
        self.anchor = state.anchor;
        self.active = state.active;
        self.marked = None;
        self.layout = None;
        self.reveal_caret = true;
        self.preferred_x = None;
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled {
            return false;
        }
        let previous = self.marked.is_none().then(|| self.snapshot());
        let Ok((range, inserted)) = self
            .document
            .replacement(range, text, self.active.is_none())
        else {
            self.message = Some("Cannot edit this Markdown range".into());
            cx.notify();
            return false;
        };
        self.layout = None;
        self.reveal_caret = true;
        self.preferred_x = None;
        self.message = None;
        if let Some(active) = &mut self.active {
            if range.start >= active.content.start && range.end <= active.content.end {
                let removed = range.len();
                active.content.end = active.content.end - removed + text.len();
                active.source.end = active.source.end - removed + inserted;
            } else {
                self.active = None;
            }
        }
        if let Some(previous) = previous {
            if self.undo.len() == 100 {
                self.undo.remove(0);
            }
            self.undo.push(previous);
            self.redo.clear();
        }
        self.marked = None;
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        cx.notify();
        true
    }

    fn move_horizontal(&mut self, right: bool, extend: bool, cx: &mut Context<Self>) {
        let selection = self.selection();
        let position = if !extend && !selection.is_empty() {
            if right {
                selection.end
            } else {
                selection.start
            }
        } else if let Some(layout) = &self.layout {
            layout.step(self.cursor, right)
        } else {
            self.cursor
        };
        self.move_to(position, extend, cx);
        self.reveal_caret = true;
    }

    fn move_vertical(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        if let Some((position, x)) = self
            .layout
            .as_ref()
            .and_then(|layout| layout.vertical(self.cursor, self.preferred_x, down))
        {
            self.move_to(position, extend, cx);
            self.preferred_x = Some(x);
            self.reveal_caret = true;
        }
    }

    fn move_to(&mut self, position: usize, extend: bool, cx: &mut Context<Self>) {
        if !self.source().is_char_boundary(position) {
            return;
        }
        self.preferred_x = None;
        self.cursor = position;
        if !extend {
            self.anchor = position;
        }
        if self.active.as_ref().is_some_and(|section| {
            position < section.content.start || position > section.content.end
        }) {
            self.finish_active();
        }
        self.marked = None;
        cx.notify();
    }

    fn remove(&mut self, right: bool, cx: &mut Context<Self>) {
        let mut selection = self.selection();
        if selection.is_empty() {
            if let Some(layout) = &self.layout {
                let mut at = self.cursor;
                if self.active.is_none() {
                    let inline: Vec<_> = self
                        .document
                        .sections()
                        .into_iter()
                        .filter(|section| {
                            matches!(section.kind, SectionKind::Strong | SectionKind::Emphasis)
                        })
                        .collect();
                    // Skip hidden delimiters in the rendered view, not in source edit mode.
                    while let Some(next) = inline.iter().find_map(|section| {
                        if right && at == section.source.start {
                            Some(section.content.start)
                        } else if right && at == section.content.end {
                            Some(section.source.end)
                        } else if !right && at == section.source.end {
                            Some(section.content.end)
                        } else if !right && at == section.content.start {
                            Some(section.source.start)
                        } else {
                            None
                        }
                    }) {
                        at = next;
                    }
                }
                let other = layout.step(at, right);
                selection = at.min(other)..at.max(other);
            }
            if selection.is_empty() {
                return;
            }
        }
        self.replace(selection, "", cx);
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled {
            return;
        }
        let Some(layout) = &self.layout else { return };
        let position = layout.index_at(event.position);
        let item = layout.item_at(event.position);
        let hit = item.and_then(|item| item.target.clone());
        let in_source = self.active.as_ref().is_some_and(|active| {
            active.content.start <= position
                && position <= active.content.end
                && layout.contains_row(event.position)
                && hit.is_none()
        });
        let position = if in_source {
            position
        } else if let Some(active) = &self.active
            && active.content.contains(&position)
        {
            active.source.end
        } else {
            position
        };
        if !in_source {
            self.finish_active();
        }
        self.selecting = true;
        window.focus(&self.focus);
        if let Some(section) = hit.filter(|_| !event.modifiers.shift) {
            self.selecting = false;
            if section.kind == SectionKind::Link {
                self.move_to(section.source.start, false, cx);
                match self.link_target(&section) {
                    Ok(crate::media::Target::Web(url)) => cx.open_url(url.as_str()),
                    Ok(crate::media::Target::Local(path)) => cx.emit(OpenLocalDocument(path)),
                    Err(error) => self.message = Some(error),
                }
                cx.notify();
            } else {
                self.activate(section, false, cx);
            }
        } else {
            self.move_to(position, event.modifiers.shift, cx);
        }
    }

    fn link_target(&self, section: &Section) -> Result<crate::media::Target, String> {
        crate::media::link_target(
            section.destination.as_deref().unwrap_or_default(),
            self.path.as_deref(),
            self.workspace.as_ref().map(Workspace::root),
        )
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else { return };
        if self.selecting {
            self.move_to(layout.index_at(event.position), true, cx);
        }
        let next = self
            .layout
            .as_ref()
            .and_then(|layout| layout.item_at(event.position))
            .and_then(|item| item.target.as_ref())
            .filter(|section| section.kind == SectionKind::Link)
            .map(|section| match self.link_target(section) {
                Ok(crate::media::Target::Web(url)) => url.to_string(),
                Ok(crate::media::Target::Local(path)) => path.display().to_string(),
                Err(_) => section.destination.clone().unwrap_or_default(),
            });
        if self.hover != next {
            self.hover = next;
            cx.notify();
        }
    }
}

pub struct OpenLocalDocument(pub PathBuf);
impl EventEmitter<OpenLocalDocument> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset || units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end)
}

fn to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    text[..range.start].encode_utf16().count()..text[..range.end].encode_utf16().count()
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.source();
        let bytes = from_utf16(text, range);
        *actual = Some(to_utf16(text, bytes.clone()));
        text.get(bytes).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !self.input_enabled && !ignore_disabled_input {
            return None;
        }
        Some(UTF16Selection {
            range: to_utf16(self.source(), self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .clone()
            .map(|range| to_utf16(self.source(), range))
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
        let range = range
            .map(|range| from_utf16(self.source(), range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| from_utf16(self.source(), range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let start = range.start;
        if self.replace(range, text, cx) && !text.is_empty() {
            self.marked = Some(start..start + text.len());
            if let Some(selected) = selected {
                let selected = from_utf16(text, selected);
                self.anchor = start + selected.start;
                self.cursor = start + selected.end;
            }
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.layout
            .as_ref()?
            .bounds_at(from_utf16(self.source(), range).start)
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let byte = self.layout.as_ref()?.index_at(position);
        Some(self.source().get(..byte)?.encode_utf16().count())
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.shape(window, cx);
        let width = layout
            .rows
            .iter()
            .map(|row| row.width)
            .max()
            .unwrap_or(px(0.));
        let height = layout.rows.last().map_or(px(26.), |row| row.y + row.height);
        let editor = cx.entity();
        let status = self
            .message
            .clone()
            .or_else(|| self.hover.clone())
            .unwrap_or_default();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x202531))
            .child(toolbar::toolbar(cx))
            .child(
                div()
                    .id("editor")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .p_4()
                    .cursor(CursorStyle::IBeam)
                    .key_context("Editor")
                    .track_focus(&self.focus)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|editor, _, _, _| editor.selecting = false),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|editor, _, _, _| editor.selecting = false),
                    )
                    .on_action(
                        cx.listener(|e, _: &Left, _, cx| e.move_horizontal(false, false, cx)),
                    )
                    .on_action(
                        cx.listener(|e, _: &Right, _, cx| e.move_horizontal(true, false, cx)),
                    )
                    .on_action(
                        cx.listener(|e, _: &SelectLeft, _, cx| e.move_horizontal(false, true, cx)),
                    )
                    .on_action(
                        cx.listener(|e, _: &SelectRight, _, cx| e.move_horizontal(true, true, cx)),
                    )
                    .on_action(cx.listener(|e, _: &Up, _, cx| e.move_vertical(false, false, cx)))
                    .on_action(cx.listener(|e, _: &Down, _, cx| e.move_vertical(true, false, cx)))
                    .on_action(
                        cx.listener(|e, _: &SelectUp, _, cx| e.move_vertical(false, true, cx)),
                    )
                    .on_action(
                        cx.listener(|e, _: &SelectDown, _, cx| e.move_vertical(true, true, cx)),
                    )
                    .on_action(cx.listener(|e, _: &SelectAll, _, cx| {
                        let range = e
                            .active
                            .as_ref()
                            .map_or(0..e.source().len(), |section| section.content.clone());
                        e.anchor = range.start;
                        e.cursor = range.end;
                        e.reveal_caret = true;
                        e.preferred_x = None;
                        cx.notify();
                    }))
                    .on_action(cx.listener(|e, _: &Backspace, _, cx| e.remove(false, cx)))
                    .on_action(cx.listener(|e, _: &Delete, _, cx| e.remove(true, cx)))
                    .on_action(cx.listener(|e, _: &Newline, window, cx| {
                        e.replace_text_in_range(None, "\n", window, cx)
                    }))
                    .on_action(cx.listener(|e, _: &Copy, _, cx| {
                        if !e.selection().is_empty() {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                e.source()[e.selection()].to_owned(),
                            ));
                        }
                    }))
                    .on_action(cx.listener(|e, _: &Cut, _, cx| {
                        if !e.selection().is_empty() {
                            let text = e.source()[e.selection()].to_owned();
                            if e.replace(e.selection(), "", cx) {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                    }))
                    .on_action(cx.listener(|e, _: &Paste, window, cx| {
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            e.replace_text_in_range(None, &text, window, cx);
                        }
                    }))
                    .on_action(cx.listener(|e, _: &Undo, _, cx| {
                        if e.input_enabled
                            && let Some(previous) = e.undo.pop()
                        {
                            e.redo.push(e.snapshot());
                            e.restore(previous, cx);
                        }
                    }))
                    .on_action(cx.listener(|e, _: &Redo, _, cx| {
                        if e.input_enabled
                            && let Some(next) = e.redo.pop()
                        {
                            e.undo.push(e.snapshot());
                            e.restore(next, cx);
                        }
                    }))
                    .child(
                        canvas(
                            move |bounds, _, _| Layout {
                                origin: bounds.origin,
                                ..layout
                            },
                            move |bounds, layout, window, cx| {
                                let e = editor.read(cx);
                                let selection = e.selection();
                                let cursor = e.cursor;
                                let focus = e.focus.clone();
                                if e.input_enabled {
                                    window.handle_input(
                                        &focus,
                                        ElementInputHandler::new(bounds, editor.clone()),
                                        cx,
                                    );
                                }
                                for row in &layout.rows {
                                    for item in &row.items {
                                        let top = layout.origin + point(item.left, row.y);
                                        let start = selection.start.max(item.source.start);
                                        let end = selection.end.min(item.source.end);
                                        if start < end {
                                            let x = item.x(start);
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    layout.origin + point(x, row.y),
                                                    size(item.x(end) - x, row.height),
                                                ),
                                                rgba(0x4a86ed50),
                                            ));
                                        }
                                        match &item.paint {
                                            ItemPaint::Text(line) => {
                                                let _ = line.paint(top, row.height, window, cx);
                                            }
                                            ItemPaint::Graphic(graphic) => {
                                                let _ = window.paint_image(
                                                    Bounds::new(
                                                        top + point(
                                                            px(0.),
                                                            (row.height - graphic.height) / 2.,
                                                        ),
                                                        size(graphic.width, graphic.height),
                                                    ),
                                                    Default::default(),
                                                    graphic.image.clone(),
                                                    0,
                                                    false,
                                                );
                                            }
                                        }
                                    }
                                }
                                if selection.is_empty()
                                    && focus.is_focused(window)
                                    && let Some(caret) = layout.bounds_at(cursor)
                                {
                                    window.paint_quad(fill(caret, rgb(0x1463b7)));
                                }
                                editor.update(cx, |e, _| {
                                    if std::mem::take(&mut e.reveal_caret)
                                        && focus.is_focused(window)
                                        && let Some(caret) = layout.bounds_at(cursor)
                                        && reveal_caret(&e.scroll, caret)
                                    {
                                        window.on_next_frame(|window, _| window.refresh());
                                    }
                                    e.layout = Some(layout);
                                });
                            },
                        )
                        .w(width + px(2.))
                        .min_w_full()
                        .h(height)
                        .min_h_full(),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .bg(rgb(0xf1f3f6))
                    .overflow_hidden()
                    .min_h(px(24.))
                    .child(status)
                    .when(self.has_blocked_images, |status| {
                        status.child(
                            div()
                                .id("allow-remote-images")
                                .text_color(rgb(0x145aac))
                                .cursor_pointer()
                                .on_click(cx.listener(|e, _, _, cx| {
                                    e.allow_remote_images = true;
                                    cx.notify();
                                }))
                                .child("Load remote images for this document"),
                        )
                    }),
            )
    }
}

fn reveal_caret(scroll: &ScrollHandle, caret: Bounds<Pixels>) -> bool {
    let viewport = scroll.bounds().inset(px(4.));
    if viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
        return false;
    }
    let current = scroll.offset();
    let limit = scroll.max_offset();
    let adjust = |start: Pixels, end: Pixels, low: Pixels, high: Pixels| {
        if start < low {
            low - start
        } else if end > high {
            high - end
        } else {
            px(0.)
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

#[cfg(test)]
mod tests {
    use std::fs;

    use gpui::{Modifiers, TestAppContext};

    use super::*;

    #[gpui::test]
    fn navigation_never_uses_layout_offsets_from_before_an_edit(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("a好".into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.layout.is_some());
                assert!(e.replace(0..1, "", cx));
                assert!(e.layout.is_none());
                e.move_horizontal(true, false, cx);
                assert_eq!(
                    e.selected_text_range(false, window, cx).unwrap().range,
                    0..0
                );
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                e.move_horizontal(true, false, cx);
                assert_eq!(e.cursor, "好".len());
            })
        });
    }

    #[gpui::test]
    fn vertical_movement_keeps_the_column_and_extends_selection(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("ABCDE\nX\nABCDE".into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                e.move_to(4, false, cx);
            })
        });
        cx.update(|_, cx| bind_keys(cx));
        cx.simulate_keystrokes("down shift-down");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.anchor, "ABCDE\nX".len());
            assert_eq!(e.cursor, "ABCDE\nX\nABCD".len());
        });
    }

    #[gpui::test]
    fn caret_reveal_scrolls_both_axes_after_paste(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                e.replace(
                    0..0,
                    &format!("{}{}", "line\n".repeat(50), "x".repeat(250)),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert!(e.scroll.offset().x < px(0.));
            assert!(e.scroll.offset().y < px(0.));
        });
    }

    #[gpui::test]
    fn trailing_bold_markers_have_a_caret_stop_and_are_removed_with_last_character(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("**x**".into(), window, cx));
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                let layout = e.layout.as_ref().unwrap();
                assert!(layout.bounds_at(5).is_some());
                assert_eq!(layout.step(3, true), 5);
                e.move_to(5, false, cx);
                e.remove(false, cx);
                assert_eq!(e.source(), "");
                assert_eq!(e.cursor, 0);
            })
        });
    }

    #[gpui::test]
    fn closing_heading_markers_have_a_caret_stop_before_the_line_break(cx: &mut TestAppContext) {
        let heading = "# Title ###";
        for eol in ["", "\n", "\r\n"] {
            let source = format!("{heading}{eol}");
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
            cx.update(|window, cx| {
                editor.update(cx, |e, cx| {
                    window.focus(&e.focus);
                    e.move_to("# Title".len(), false, cx);
                });
                bind_keys(cx);
            });
            cx.run_until_parked();
            cx.simulate_keystrokes("right");
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.cursor, heading.len());
                let layout = editor.layout.as_ref().unwrap();
                assert!(layout.bounds_at(editor.cursor).is_some());
                assert_eq!(
                    layout.index_at(layout.origin + point(px(500.), px(0.))),
                    heading.len()
                );
            });
            cx.simulate_keystrokes("enter");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), format!("{heading}\n{eol}")));
        }
    }

    #[gpui::test]
    fn backspace_in_source_mode_deletes_a_visible_marker(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new("[**xy**](https://example.com)".into(), window, cx)
        });
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                let link = e
                    .document
                    .sections()
                    .into_iter()
                    .find(|s| s.kind == SectionKind::Link);
                e.activate(link.unwrap(), false, cx);
                e.move_to(3, false, cx);
            });
            bind_keys(cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "[*xy**](https://example.com)"));
    }

    #[gpui::test]
    fn empty_mermaid_body_has_a_caret_stop(cx: &mut TestAppContext) {
        let source = "```mermaid\n```";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let block = e
                    .document
                    .sections()
                    .into_iter()
                    .find(|s| s.kind == SectionKind::Mermaid);
                e.activate(block.unwrap(), false, cx);
                let layout = e.shape(window, cx);
                assert_eq!(e.cursor, "```mermaid\n".len());
                assert!(layout.bounds_at(e.cursor).is_some());
                assert_eq!(layout.index_at(point(px(40.), px(0.))), e.cursor);
                assert_eq!(e.source(), source);
            });
        });
    }

    #[gpui::test]
    fn preview_cap_does_not_label_a_valid_formula_invalid(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let formulas: Vec<_> = (0..129).map(|n| format!("x^{n}")).collect();
                let source = formulas.iter().map(|body| format!("${body}$\n")).collect();
                e.document = Document::new(source);
                for body in formulas.into_iter().take(128) {
                    e.graphics.insert(
                        (
                            body,
                            SectionKind::InlineMath,
                            window.scale_factor().to_bits(),
                        ),
                        Err("test cached preview".into()),
                    );
                }
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            let item = e.layout.as_ref().unwrap().rows.iter().rev()
                .find_map(|row| row.items.last()).unwrap();
            assert!(matches!(&item.paint, ItemPaint::Text(line) if line.text.as_ref() == "Preview limit reached"));
        });
    }

    #[gpui::test]
    fn remote_images_need_consent_and_old_requests_do_not_replace_new_ones(
        cx: &mut TestAppContext,
    ) {
        let image_url = "https://example.invalid/picture.png";
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(format!("![Preview]({image_url})"), window, cx)
        });
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.has_blocked_images);
                assert_eq!(e.image_requests, 0);
                assert!(e.images.is_empty());
                e.images
                    .insert("old.png".into(), ImageEntry::Ready(Err("old".into())));
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| editor.update(cx, |e, _| {
            assert!(e.images.is_empty());
            e.images.insert("same.png".into(), ImageEntry::Loading(1));
            e.image_requests = 2;
            e.set_path(Some("new/doc.md".into()), None);
            e.images.insert("same.png".into(), ImageEntry::Loading(2));
            e.complete_image_request("same.png".into(), 1, Err("stale".into()));
            assert!(matches!(e.images.get("same.png"), Some(ImageEntry::Loading(2))));
            e.complete_image_request("same.png".into(), 2, Err("new".into()));
            assert!(matches!(e.images.get("same.png"), Some(ImageEntry::Ready(Err(message))) if message == "new"));
            assert_eq!(e.image_requests, 0);
        }));
    }

    #[gpui::test]
    fn edits_markdown_in_one_surface(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.md");
        let original = include_str!("../../document/tests/fixtures/sample.md");
        fs::write(&path, original).unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(fs::read_to_string(&path).unwrap(), window, cx)
        });
        cx.update(|_, cx| bind_keys(cx));
        let (heading, formula, paragraph) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let heading = editor.source().find("原始標題").unwrap();
            let paragraph = editor.source().find("行內公式").unwrap();
            let formula = layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items
                        .iter()
                        .find(|item| {
                            matches!(item.paint, ItemPaint::Graphic(_))
                                && item
                                    .target
                                    .as_ref()
                                    .is_some_and(|section| section.kind == SectionKind::InlineMath)
                        })
                        .map(|item| {
                            layout.origin
                                + point(item.left + item.width / 2., row.y + row.height / 2.)
                        })
                })
                .expect("the formula must render as a graphic");
            (
                layout.bounds_at(heading).unwrap().center(),
                formula,
                layout.bounds_at(paragraph).unwrap().center(),
            )
        });
        cx.simulate_click(heading, Modifiers::none());
        cx.simulate_keystrokes("shift-right shift-right shift-right shift-right");
        cx.simulate_input("新標題");
        cx.simulate_click(formula, Modifiers::none());
        cx.update(|_, cx| assert!(editor.read(cx).active.is_some()));
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("y^3");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.active.is_some());
            fs::write(&path, editor.source()).unwrap();
        });
        cx.simulate_click(paragraph, Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.active.is_none());
            assert!(editor.layout.as_ref().unwrap().rows.iter().any(|row| {
                row.items
                    .iter()
                    .any(|item| matches!(item.paint, ItemPaint::Graphic(_)))
            }));
        });
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            original
                .replacen("# 原始標題", "# 新標題", 1)
                .replacen("$x^2$", "$y^3$", 1)
        );
        cx.simulate_mouse_down(heading, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(paragraph, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(paragraph, MouseButton::Left, Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.source()[editor.selection()].contains("\n\n"));
        });
    }

    #[gpui::test]
    fn inserts_and_removes_editable_items(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        for kind in [
            SectionKind::FrontMatter,
            SectionKind::Mermaid,
            SectionKind::InlineMath,
            SectionKind::DisplayMath,
            SectionKind::Image,
            SectionKind::Link,
        ] {
            cx.update(|window, cx| {
                editor.update(cx, |e, cx| {
                    e.command(Command::Add(kind), window, cx);
                    assert_eq!(e.active.as_ref().map(|s| s.kind), Some(kind));
                    e.command(Command::RemoveItem, window, cx);
                    assert!(
                        e.document
                            .sections()
                            .iter()
                            .all(|section| section.kind != kind)
                    );
                })
            });
        }
    }

    #[gpui::test]
    fn uses_one_undo_history_for_formatting_and_typing(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("選取\n\n也選取".into(), window, cx));
        cx.update(|_, cx| bind_keys(cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.anchor = 0;
                e.cursor = e.source().len();
                e.command(Command::Bold, window, cx);
                assert_eq!(e.source(), "**選取**\n\n**也選取**");
                e.command(Command::Heading, window, cx);
                assert_eq!(e.source(), "**選取**\n\n# **也選取**");
            })
        });
        cx.simulate_keystrokes("cmd-z cmd-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "選取\n\n也選取"));
        cx.simulate_keystrokes("cmd-shift-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "**選取**\n\n**也選取**"));
    }

    #[gpui::test]
    fn formatting_across_headings_keeps_both_block_markers(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new("# First\n\n# Second ###\n".into(), window, cx)
        });
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.anchor = "# ".len();
                e.cursor = "# First\n\n# Second".len();
                e.command(Command::Bold, window, cx);
                assert_eq!(e.source(), "# **First**\n\n# **Second** ###\n");
                e.command(Command::Bullet, window, cx);
                assert_eq!(e.source(), "# **First**\n\n- **Second**\n");
            })
        });
    }

    #[gpui::test]
    fn invalid_rich_item_keeps_its_source_and_can_be_repaired(cx: &mut TestAppContext) {
        let source = "# Graph\n\n```mermaid\nflowchart LR\nA[Start] -->\n```\n\nParagraph";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        let click_item = |e: &Editor| {
            let layout = e.layout.as_ref().unwrap();
            layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items
                        .iter()
                        .find(|item| {
                            item.target
                                .as_ref()
                                .is_some_and(|section| section.kind == SectionKind::Mermaid)
                        })
                        .map(|item| {
                            (
                                matches!(item.paint, ItemPaint::Graphic(_)),
                                layout.origin
                                    + point(item.left + item.width / 2., row.y + row.height / 2.),
                            )
                        })
                })
                .unwrap()
        };
        let (painted, center) = cx.update(|_, cx| click_item(editor.read(cx)));
        assert!(!painted);
        cx.simulate_click(center, Modifiers::none());
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                let section = e.active.clone().unwrap();
                e.replace(section.content, "flowchart LR\nA[Start] --> B[End]", cx);
            })
        });
        let outside = cx.update(|_, cx| {
            let e = editor.read(cx);
            e.layout
                .as_ref()
                .unwrap()
                .bounds_at(e.source().find("Paragraph").unwrap())
                .unwrap()
                .center()
        });
        cx.simulate_click(outside, Modifiers::none());
        assert!(cx.update(|_, cx| click_item(editor.read(cx)).0));
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).source(),
                source.replace("A[Start] -->", "A[Start] --> B[End]")
            )
        });
    }
}
