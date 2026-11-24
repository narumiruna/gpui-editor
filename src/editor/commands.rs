use gpui::{Context, Window};

use super::Editor;
use crate::document::{Section, SectionKind};

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
            K::InlineMath => (self.selection(), "$x^2$".into(), self.selection().start),
            K::Image => (
                self.selection(),
                "![Image](image.png)".into(),
                self.selection().start,
            ),
            K::Link => {
                let selection = self.selection();
                let label = if selection.is_empty() {
                    "Link"
                } else {
                    &self.source()[selection.clone()]
                };
                let label = label
                    .replace('\\', "\\\\")
                    .replace(['\r', '\n'], " ")
                    .replace('[', "\\[")
                    .replace(']', "\\]");
                let probe = selection.start;
                (selection, format!("[{label}](https://example.com)"), probe)
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
