//! Go, Rust, and Jinja2 streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! These are not the modern grammar loaders. The native stream measures
//! positions in UTF-8 bytes and columns in Unicode scalars.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod go;
mod jinja2;
mod rust;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Go => "go",
        LegacyMode::Rust => "rust",
        LegacyMode::Jinja2 => "jinja2",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let comments = match mode {
        LegacyMode::Go | LegacyMode::Rust => CommentTokens {
            line: Some("//"),
            block: Some(("/*", "*/")),
        },
        LegacyMode::Jinja2 => CommentTokens {
            // Upstream puts "##" inside `block`, not in the line comment field.
            line: None,
            block: Some(("{#", "#}")),
        },
        _ => unreachable!(),
    };
    let data = LanguageData::default().with("commentTokens", comments);
    match mode {
        LegacyMode::Go => data.with("indentOnInput", go::indent_on_input as fn(&str) -> bool),
        LegacyMode::Rust => data
            .with("name", "rust")
            .with("dontIndentStates", &["comment"][..])
            .with("indentOnInput", rust::indent_on_input as fn(&str) -> bool),
        _ => data,
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Go(go::State),
    Rust(rust::State),
    Jinja2(jinja2::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Go => Scan::Go(go::State::new(indent_unit)),
            LegacyMode::Rust => Scan::Rust(rust::State::default()),
            LegacyMode::Jinja2 => Scan::Jinja2(jinja2::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Go(_) => LegacyMode::Go,
            Scan::Rust(_) => LegacyMode::Rust,
            Scan::Jinja2(_) => LegacyMode::Jinja2,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Go(state) => state.token(stream),
            Scan::Rust(state) => state.token(stream),
            Scan::Jinja2(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Go(state) => state.indent(text_after, unit),
            Scan::Rust(state) => state.indent(text_after),
            Scan::Jinja2(_) => None,
        }
    }
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn take(stream: &mut StringStream<'_>, text: &str) -> bool {
    stream.match_str(text, true, false)
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn in_words(words: &str, text: &str) -> bool {
    words.split_ascii_whitespace().any(|word| word == text)
}
