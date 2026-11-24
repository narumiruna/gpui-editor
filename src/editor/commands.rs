use gpui::{Context, Window};

use super::Editor;
use crate::document::{Document, Section, SectionKind};

#[derive(Clone, Copy)]
pub(super) enum Command {
    Heading,
    Bullet,
    Bold,
    Italic,
    Add(SectionKind),
    EditLink,
    RemoveItem,
}

fn can_edit_as_item(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::FrontMatter
            | SectionKind::Mermaid
            | SectionKind::InlineMath
            | SectionKind::DisplayMath
            | SectionKind::Image
            | SectionKind::Link
    )
}

impl Editor {
    pub(super) fn activate(&mut self, mut section: Section, select: bool, cx: &mut Context<Self>) {
        if matches!(section.kind, SectionKind::Image | SectionKind::Link) {
            section.content = section.source.clone();
        }
        self.cursor = section.content.start;
        self.anchor = if select {
            section.content.end
        } else {
            self.cursor
        };
        self.active = Some(section);
        self.preferred_x = None;
        self.marked = None;
        self.message = None;
        cx.notify();
    }

    pub(super) fn command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use SectionKind as K;
        if !self.input_enabled {
            return;
        }
        window.focus(&self.focus);
        self.message = None;
        match command {
            Command::Heading => self.block(K::Heading, "# ", cx),
            Command::Bullet => self.block(K::Bullet, "- ", cx),
            Command::Bold => self.inline(K::Strong, "**", cx),
            Command::Italic => self.inline(K::Emphasis, "*", cx),
            Command::Add(kind) => self.insert(kind, cx),
            Command::EditLink => {
                if let Some(link) = self.section_at_cursor(|kind| kind == K::Link) {
                    self.activate(link, false, cx);
                } else {
                    self.message =
                        Some("Place the caret next to the link, then select Edit link".into());
                }
            }
            Command::RemoveItem => {
                if let Some(section) = self
                    .active
                    .clone()
                    .or_else(|| self.section_at_cursor(can_edit_as_item))
                {
                    self.active = None;
                    self.replace(section.source, "", cx);
                } else {
                    self.message = Some(
                        "Place the caret next to an item or click it before removing it".into(),
                    );
                }
            }
        }
        cx.notify();
    }

    fn section_at_cursor(&self, predicate: impl Fn(SectionKind) -> bool) -> Option<Section> {
        self.document.sections().into_iter().find(|section| {
            predicate(section.kind)
                && section.source.start <= self.cursor
                && self.cursor <= section.source.end
        })
    }

    fn insert(&mut self, kind: SectionKind, cx: &mut Context<Self>) {
        use SectionKind as K;
        if kind == K::FrontMatter
            && let Some(section) = self
                .document
                .sections()
                .into_iter()
                .find(|it| it.kind == kind)
        {
            self.activate(section, false, cx);
            return;
        }
        let (range, snippet, probe) = match kind {
            K::FrontMatter => (0..0, "---\ntitle: Document\n---\n\n".into(), 0),
            K::Mermaid | K::DisplayMath => {
                let source = self.source();
                let end = self.cursor
                    + source[self.cursor..]
                        .find('\n')
                        .unwrap_or(source.len() - self.cursor);
                let body = if kind == K::Mermaid {
                    "```mermaid\nflowchart LR\n    A[Start] --> B[End]\n```"
                } else {
                    "$$\nx^2\n$$"
                };
                (end..end, format!("\n\n{body}\n\n"), end + 2)
            }
            K::InlineMath => {
                let selection = self.selection();
                let body = if selection.is_empty() {
                    "x^2"
                } else {
                    &self.source()[selection.clone()]
                };
                let probe = selection.start;
                let snippet = format!("${body}$");
                let mut source = self.source().to_owned();
                source.replace_range(selection.clone(), &snippet);
                let wrapped = probe..probe + snippet.len();
                if !Document::new(source)
                    .sections()
                    .iter()
                    .any(|section| section.kind == K::InlineMath && section.source == wrapped)
                {
                    self.message = Some(
                        "Cannot wrap this text as inline math here. Select text within one paragraph or use Display math."
                            .into(),
                    );
                    return;
                }
                (selection, snippet, probe)
            }
            K::Image | K::Link => {
                let (prefix, placeholder, destination) = if kind == K::Image {
                    ("!", "Image", "image.png")
                } else {
                    ("", "Link", "https://example.com")
                };
                let selection = self.selection();
                let label = if selection.is_empty() {
                    placeholder
                } else {
                    &self.source()[selection.clone()]
                };
                let label = label
                    .replace('\\', "\\\\")
                    .replace(['\r', '\n'], " ")
                    .replace('[', "\\[")
                    .replace(']', "\\]");
                let probe = selection.start;
                (
                    selection,
                    format!("{prefix}[{label}]({destination})"),
                    probe,
                )
            }
            _ => return,
        };
        self.active = None;
        if self.replace(range, &snippet, cx)
            && let Some(section) = self
                .document
                .sections()
                .into_iter()
                .find(|section| section.kind == kind && section.source.start == probe)
        {
            self.activate(section, true, cx);
        }
    }

    fn block(&mut self, kind: SectionKind, marker: &str, cx: &mut Context<Self>) {
        let source = self.source();
        let cursor = self.selection().start;
        let start = source[..cursor].rfind('\n').map_or(0, |index| index + 1);
        let existing = self.document.sections().into_iter().find(|section| {
            matches!(section.kind, SectionKind::Heading | SectionKind::Bullet)
                && section.source.start == start
        });
        if let Some(section) = existing {
            let prefix = &source[section.source.start..section.content.start];
            if prefix.contains('\n') || prefix.is_empty() {
                self.message =
                    Some("This block's existing marker cannot be changed in place".into());
            } else {
                let replacement = if kind == section.kind { "" } else { marker };
                let suffix = &source[section.content.end..section.source.end];
                let suffix_marker = suffix.trim();
                if section.kind == SectionKind::Heading
                    && !suffix_marker.is_empty()
                    && suffix_marker.bytes().all(|byte| byte == b'#')
                {
                    let newline = &suffix[suffix.trim_end_matches(['\r', '\n']).len()..];
                    let body = &source[section.content.clone()];
                    let text = format!("{replacement}{body}{newline}");
                    self.replace(section.source, &text, cx);
                } else {
                    self.replace(start..section.content.start, replacement, cx);
                }
            }
        } else {
            let trailing = if source
                .as_bytes()
                .get(start)
                .is_none_or(|byte| *byte == b'\n')
            {
                "Text"
            } else {
                ""
            };
            self.replace(start..start, &format!("{marker}{trailing}"), cx);
        }
    }

    fn inline(&mut self, kind: SectionKind, marker: &str, cx: &mut Context<Self>) {
        let selection = self.selection();
        if let Some(section) = self.document.sections().into_iter().find(|section| {
            section.kind == kind
                && section.content.start <= selection.start
                && section.content.end >= selection.end
        }) {
            let content = self.source()[section.content.clone()].to_owned();
            self.replace(section.source, &content, cx);
            return;
        }
        if selection.is_empty() {
            let start = selection.start;
            if self.replace(selection, &format!("{marker}text{marker}"), cx) {
                self.anchor = start + marker.len();
                self.cursor = self.anchor + "text".len();
            }
            return;
        }
        // Apply an inline style to each selected line separately; inline Markdown
        // delimiters cannot enclose a blank line.
        let blocks = self
            .document
            .sections()
            .into_iter()
            .filter(|section| matches!(section.kind, SectionKind::Heading | SectionKind::Bullet))
            .collect::<Vec<_>>();
        let source = &self.source()[selection.clone()];
        let mut content = String::new();
        let mut start = selection.start;
        for line in source.split_inclusive('\n') {
            let end = start + line.len();
            let mut text_start = start;
            let mut text_end = end;
            for block in blocks
                .iter()
                .filter(|block| block.source.start < end && block.source.end > start)
            {
                text_start = text_start.max(block.content.start).min(end);
                if block.kind == SectionKind::Heading {
                    text_end = text_end.min(block.content.end).max(text_start);
                }
            }
            let editable = &line[text_start - start..text_end - start];
            let value = editable.trim();
            if value.is_empty() {
                content.push_str(line);
            } else {
                let left = text_start - start + editable.len() - editable.trim_start().len();
                content.push_str(&line[..left]);
                content.push_str(marker);
                content.push_str(value);
                content.push_str(marker);
                content.push_str(&line[left + value.len()..]);
            }
            start = end;
        }
        self.replace(selection, &content, cx);
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn activation_starts_vertical_movement_from_the_new_caret_column(cx: &mut TestAppContext) {
        let source = "---\ntitle: Document\nwidth: 120\n---";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.preferred_x = Some(gpui::px(1_000.));
                editor.command(Command::Add(SectionKind::FrontMatter), window, cx);
                assert_eq!(editor.cursor, source.find("title").unwrap());
                editor.layout = Some(editor.shape(window, cx));
                editor.move_vertical(true, false, cx);
                assert_eq!(editor.cursor, source.find("width").unwrap());
                assert_eq!(editor.preferred_x, Some(gpui::px(0.)));
            });
        });
    }

    #[gpui::test]
    fn deleting_a_selection_in_link_source_preserves_visible_delimiters(cx: &mut TestAppContext) {
        let source = "[**x**](https://example.com)";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.command(Command::EditLink, window, cx);
                assert_eq!(editor.active.as_ref().unwrap().kind, SectionKind::Link);
                editor.anchor = "[**".len();
                editor.cursor = "[**x".len();
                editor.remove(false, cx);
                assert_eq!(editor.source(), "[****](https://example.com)");
                assert!(editor.active.is_some());
            });
        });
    }

    #[gpui::test]
    fn rejects_unsupported_inline_math_selections_without_an_edit(cx: &mut TestAppContext) {
        for (prefix, body) in [
            ("Before ", "a\n\nb"),
            ("Before ", "a\r\n \t\r\nb"),
            ("Before ", "a$b"),
            (r"Before \", "a+b"),
        ] {
            let source = format!("{prefix}{body} after");
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let selection = prefix.len()..prefix.len() + body.len();
                    editor.anchor = selection.start;
                    editor.cursor = selection.end;
                    editor.command(Command::Add(SectionKind::InlineMath), window, cx);
                    assert_eq!(editor.source(), source);
                    assert_eq!(editor.selection(), selection);
                    assert!(editor.active.is_none());
                    assert!(editor.undo.is_empty());
                    assert!(editor.message.as_deref().unwrap().contains("inline math"));
                });
            });
        }
    }

    #[gpui::test]
    fn inserts_inline_math_and_images_from_the_selection(cx: &mut TestAppContext) {
        for (kind, selected, expected) in [
            (SectionKind::InlineMath, "a+b", "$a+b$"),
            (SectionKind::InlineMath, "a\nb", "$a\nb$"),
            (
                SectionKind::Image,
                r"圖[例]\]",
                r"![圖\[例\]\\\]](image.png)",
            ),
        ] {
            let (editor, cx) = cx.add_window_view(|window, cx| {
                Editor::new(format!("Before {selected} after"), window, cx)
            });
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.anchor = "Before ".len();
                    editor.cursor = editor.anchor + selected.len();
                    editor.command(Command::Add(kind), window, cx);
                    assert_eq!(editor.source(), format!("Before {expected} after"));
                    assert_eq!(
                        editor.active.as_ref().map(|section| section.kind),
                        Some(kind)
                    );
                });
            });
        }
    }
}
