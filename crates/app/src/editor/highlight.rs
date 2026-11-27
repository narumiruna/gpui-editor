//! Syntax colors for editable source text. All ranges are UTF-8 byte offsets.

use std::{ffi::OsStr, ops::Range, path::Path, sync::LazyLock};

use gpui::{Context, Rgba};
use two_face::re_exports::syntect::{
    easy::ScopeRangeIterator,
    parsing::{ParseState, Scope, ScopeStack, SyntaxSet},
};

use super::Editor;
use crate::theme::Theme;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 16 * 1024;
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Comment,
    Keyword,
    String,
    Constant,
    Name,
}

impl Kind {
    pub(super) fn color(self, theme: Theme) -> Rgba {
        match self {
            Self::Comment => theme.muted,
            Self::Keyword => theme.syntax_keyword,
            Self::String => theme.syntax_string,
            Self::Constant => theme.syntax_constant,
            Self::Name => theme.accent,
        }
    }
}

static RULES: LazyLock<[(Scope, Kind); 7]> = LazyLock::new(|| {
    [
        ("comment", Kind::Comment),
        ("keyword", Kind::Keyword),
        ("storage", Kind::Keyword),
        ("string", Kind::String),
        ("constant", Kind::Constant),
        ("entity.name", Kind::Name),
        ("support.type", Kind::Name),
    ]
    .map(|(prefix, kind)| (Scope::new(prefix).expect("valid syntax scope"), kind))
});

pub(super) struct Span {
    pub(super) range: Range<usize>,
    pub(super) kind: Kind,
}

#[derive(Default)]
pub(super) struct Highlighting {
    revision: u64,
    pending: bool,
    result: Option<Vec<Span>>,
}

impl Highlighting {
    pub(super) fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.result = None;
    }

    pub(super) fn spans(&self) -> &[Span] {
        self.result.as_deref().unwrap_or_default()
    }
}

impl Editor {
    pub(super) fn request_highlighting(&mut self, cx: &mut Context<Self>) {
        if self.highlighting.pending || self.highlighting.result.is_some() {
            return;
        }
        let source = self.source();
        let extension = self
            .path
            .as_deref()
            .and_then(Path::extension)
            .and_then(OsStr::to_str);
        let Some(extension) = extension.filter(|_| !source.is_empty() && source.len() <= MAX_BYTES)
        else {
            self.highlighting.result = Some(Vec::new());
            return;
        };
        let source = source.to_owned();
        let extension = extension.to_owned();
        let revision = self.highlighting.revision;
        self.highlighting.pending = true;
        let work = cx
            .background_executor()
            .spawn(async move { parse(&source, &extension).unwrap_or_default() });
        cx.spawn(async move |editor, cx| {
            let spans = work.await;
            let _ = editor.update(cx, |editor, cx| {
                editor.highlighting.pending = false;
                if editor.highlighting.revision == revision {
                    editor.highlighting.result = Some(spans);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn parse(source: &str, extension: &str) -> Option<Vec<Span>> {
    let lines = source.split_inclusive('\n');
    if lines.clone().any(|line| line.len() > MAX_LINE_BYTES) {
        return None;
    }
    let syntax = SYNTAXES.find_syntax_by_extension(extension)?;
    let mut parser = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut offset = 0;
    for line in lines {
        let ops = parser.parse_line(line, &SYNTAXES).ok()?;
        for (range, op) in ScopeRangeIterator::new(&ops, line) {
            stack.apply(op).ok()?;
            if range.is_empty() {
                continue;
            }
            let kind = stack.as_slice().iter().rev().find_map(|&scope| {
                RULES
                    .iter()
                    .find_map(|&(prefix, kind)| prefix.is_prefix_of(scope).then_some(kind))
            });
            if let Some(kind) = kind {
                line.get(range.clone())?;
                let range = offset + range.start..offset + range.end;
                if let Some(last) = spans.last_mut()
                    && last.range.end == range.start
                    && last.kind == kind
                {
                    last.range.end = range.end;
                } else {
                    spans.push(Span { range, kind });
                }
            }
        }
        offset += line.len();
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn discards_stale_highlights_and_refreshes_after_edits(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::new("const n = 42;\n".into(), window, cx);
            editor.set_path(Some("first.ts".into()), None);
            editor
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.highlighting.spans().iter().any(|span| {
                    span.kind == Kind::Constant && &e.source()[span.range.clone()] == "42"
                }));
                e.set_path(Some("second.ts".into()), None);
                e.request_highlighting(cx);
                assert!(e.highlighting.pending);
                assert!(e.replace(0..e.source().len(), "// 已編輯\n", cx));
                assert!(e.highlighting.spans().is_empty());
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                assert_eq!(e.source(), "// 已編輯\n");
                assert!(!e.highlighting.spans().is_empty());
                assert!(
                    e.highlighting
                        .spans()
                        .iter()
                        .all(|span| span.kind == Kind::Comment)
                );
                e.set_path(Some("second.unsupported".into()), None);
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| assert!(editor.read(cx).highlighting.spans().is_empty()));
    }

    #[test]
    fn highlights_requested_code_and_configuration_formats() {
        for (extension, source, token, kind) in [
            (
                "py",
                "def hello():\n    return '你好'\n",
                "def",
                Kind::Keyword,
            ),
            ("rs", "fn main() { let n = 1; }\n", "let", Kind::Keyword),
            (
                "go",
                "package main\nvar port = 8080\n",
                "8080",
                Kind::Constant,
            ),
            ("ts", "const port: number = 8080;\n", "const", Kind::Keyword),
            ("js", "const user = '你好';\n", "你好", Kind::String),
            ("toml", "[server]\nport = 8080\n", "8080", Kind::Constant),
            ("yaml", "server:\n  port: 8080\n", "8080", Kind::Constant),
            ("yml", "server:\n  port: 8080\n", "8080", Kind::Constant),
            ("JSON", "{\"port\": 8080}\n", "8080", Kind::Constant),
        ] {
            let start = source.find(token).unwrap();
            let spans =
                parse(source, extension).unwrap_or_else(|| panic!("missing {extension} syntax"));
            assert!(
                spans.iter().any(|span| {
                    span.kind == kind
                        && span.range.start <= start
                        && span.range.end >= start + token.len()
                }),
                "{extension}: {token}"
            );
            assert!(
                spans
                    .iter()
                    .all(|span| source.get(span.range.clone()).is_some())
            );
        }
    }

    #[test]
    fn retains_multiline_state_and_falls_back_for_unknown_or_long_lines() {
        let source = "message = \"\"\"first\r\n第二行\r\n\"\"\"\r\n";
        let second = source.find("第二行").unwrap();
        assert!(
            parse(source, "py")
                .unwrap()
                .iter()
                .any(|span| { span.kind == Kind::String && span.range.contains(&second) })
        );
        assert!(parse(source, "no-such-language").is_none());
        assert!(parse(&"x".repeat(MAX_LINE_BYTES + 1), "js").is_none());
    }
}
