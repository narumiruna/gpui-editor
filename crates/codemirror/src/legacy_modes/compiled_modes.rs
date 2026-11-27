//! Elm, Swift, D, and Crystal streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Byte offsets and columns use the Rust scalar stream. State copies are
//! independent snapshots. Negative indents are clamped to zero.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod crystal;
mod d;
mod elm;
mod swift;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Elm => "elm",
        LegacyMode::Swift => "swift",
        LegacyMode::D => "d",
        LegacyMode::Crystal => "crystal",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, block) = match mode {
        LegacyMode::Elm => ("--", None),
        LegacyMode::Swift | LegacyMode::D => ("//", Some(("/*", "*/"))),
        LegacyMode::Crystal => ("#", None),
        _ => unreachable!(),
    };
    let data = LanguageData::default().with(
        "commentTokens",
        CommentTokens {
            line: Some(line),
            block,
        },
    );
    match mode {
        LegacyMode::Swift => data
            .with("indentOnInput", swift::indent_on_input as fn(&str) -> bool)
            .with("closeBrackets", &["(", "[", "{", "'", "\"", "`"][..]),
        LegacyMode::D => data.with("indentOnInput", d::indent_on_input as fn(&str) -> bool),
        LegacyMode::Crystal => data.with(
            "indentOnInput",
            crystal::indent_on_input as fn(&str) -> bool,
        ),
        _ => data,
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Elm(elm::State),
    Swift(swift::State),
    D(d::State),
    Crystal(crystal::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Elm => Scan::Elm(elm::State::default()),
            LegacyMode::Swift => Scan::Swift(swift::State::default()),
            LegacyMode::D => Scan::D(d::State::new(indent_unit)),
            LegacyMode::Crystal => Scan::Crystal(crystal::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Elm(_) => LegacyMode::Elm,
            Scan::Swift(_) => LegacyMode::Swift,
            Scan::D(_) => LegacyMode::D,
            Scan::Crystal(_) => LegacyMode::Crystal,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Elm(state) => state.token(stream),
            Scan::Swift(state) => state.token(stream),
            Scan::D(state) => state.token(stream),
            Scan::Crystal(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Elm(_) => None,
            Scan::Swift(state) => Some(state.indent(text_after, unit)),
            Scan::D(state) => state.indent(text_after, unit),
            Scan::Crystal(state) => Some(state.indent(text_after, unit)),
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn in_words(words: &str, text: &str) -> bool {
    words.split_ascii_whitespace().any(|word| word == text)
}

fn take(stream: &mut StringStream<'_>, text: &str) -> bool {
    stream.match_str(text, true, false)
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}
