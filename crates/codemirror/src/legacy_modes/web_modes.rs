//! Concrete `mode/javascript` and `mode/xml` exports from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! The HTML export uses the XML stream's HTML rules. It does not nest other modes.
//! Token offsets and columns follow the Rust scalar stream.

use super::{LegacyMode, js_line_terminator, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod javascript;
mod xml;
pub(super) mod xml_factory;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::JavaScript => "javascript",
        LegacyMode::TypeScript => "typescript",
        LegacyMode::Json | LegacyMode::JsonLd => "json",
        LegacyMode::Xml | LegacyMode::Html => "xml",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Xml | LegacyMode::Html => data
            .with(
                "commentTokens",
                CommentTokens {
                    line: None,
                    block: Some(("<!--", "-->")),
                },
            )
            .with("indentOnInput", xml_indent_input as fn(&str) -> bool),
        LegacyMode::JavaScript | LegacyMode::TypeScript | LegacyMode::Json | LegacyMode::JsonLd => {
            let data = data
                .with("indentOnInput", js_indent_input as fn(&str) -> bool)
                .with("wordChars", "$")
                .with("closeBrackets", &["(", "[", "{", "'", "\"", "`"][..]);
            if matches!(mode, LegacyMode::Json | LegacyMode::JsonLd) {
                data
            } else {
                data.with(
                    "commentTokens",
                    CommentTokens {
                        line: Some("//"),
                        block: Some(("/*", "*/")),
                    },
                )
            }
        }
        _ => unreachable!(),
    }
}

fn js_indent_input(text: &str) -> bool {
    let text = text.trim_start_matches(js_space);
    matches!(text, "default:" | "{" | "}")
        || text
            .strip_prefix("case ")
            .is_some_and(|tail| tail.ends_with(':') && !tail.contains(js_line_terminator))
}

fn xml_indent_input(text: &str) -> bool {
    text.strip_suffix('>')
        .and_then(|text| text.rsplit_once("</"))
        .is_some_and(|(_, name)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| js_space(ch) || xml::word(ch) || ch == ':')
        })
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    JavaScript(javascript::State),
    Xml(xml::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Xml | LegacyMode::Html => Scan::Xml(xml::State::new(mode)),
            _ => Scan::JavaScript(javascript::State::new(mode, indent_unit)),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match &self.0 {
            Scan::JavaScript(state) => state.mode,
            Scan::Xml(state) => state.mode,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::JavaScript(state) => state.token(stream),
            Scan::Xml(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::JavaScript(state) => state.indent(text_after, indent_unit),
            Scan::Xml(state) => state.indent(text_after, indent_unit),
        }
    }
}
