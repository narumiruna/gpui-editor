//! One GPUI input surface backed by offsets in the Markdown source.

use std::{collections::HashMap, ops::Range, path::PathBuf};

use crate::document::{Document, Section, SectionKind};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementInputHandler, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, UTF16Selection, Window, actions, canvas, div, fill, point, prelude::*, px, rgb,
    rgba, size,
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
    graphics: HashMap<(String, SectionKind, u32), Result<Graphic, String>>,
    images: HashMap<String, Option<Result<Graphic, String>>>,
    path: Option<PathBuf>,
    workspace: Option<PathBuf>,
    hover: Option<String>,
    message: Option<String>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
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
            graphics: HashMap::new(),
            images: HashMap::new(),
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

    pub fn set_path(&mut self, path: Option<PathBuf>, workspace: Option<PathBuf>) {
        self.path = path;
        self.workspace = workspace;
        self.images.clear();
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
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) -> bool {
        let previous = self.marked.is_none().then(|| self.snapshot());
        if self.document.replace(range.clone(), text).is_err() {
            self.message = Some("Cannot edit this Markdown range".into());
            cx.notify();
            return false;
        }
        self.message = None;
        if let Some(active) = &mut self.active {
            if range.start >= active.content.start && range.end <= active.content.end {
                let removed = range.len();
                active.content.end = active.content.end - removed + text.len();
                active.source.end = active.source.end - removed + text.len();
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
    }

    fn move_to(&mut self, position: usize, extend: bool, cx: &mut Context<Self>) {
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
                let other = layout.step(self.cursor, right);
                selection = self.cursor.min(other)..self.cursor.max(other);
            }
            if selection.is_empty() {
                return;
            }
        }
        self.replace(selection, "", cx);
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
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
            self.workspace.as_deref(),
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
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
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
        Some(self.source()[..byte].encode_utf16().count())
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
                    .on_action(cx.listener(|e, _: &SelectAll, _, cx| {
                        let range = e
                            .active
                            .as_ref()
                            .map_or(0..e.source().len(), |section| section.content.clone());
                        e.anchor = range.start;
                        e.cursor = range.end;
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
                        if let Some(previous) = e.undo.pop() {
                            e.redo.push(e.snapshot());
                            e.restore(previous, cx);
                        }
                    }))
                    .on_action(cx.listener(|e, _: &Redo, _, cx| {
                        if let Some(next) = e.redo.pop() {
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
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, editor.clone()),
                                    cx,
                                );
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
                                editor.update(cx, |e, _| e.layout = Some(layout));
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
                    .child(status),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gpui::{Modifiers, TestAppContext};

    use super::*;

    #[gpui::test]
    fn edits_markdown_in_one_surface(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.md");
        let original = include_str!("../tests/fixtures/sample.md");
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
