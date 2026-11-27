//! YAML, Turtle, and EBNF streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod ebnf;
mod turtle;
mod yaml;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Yaml => "yaml",
        LegacyMode::Turtle => "turtle",
        LegacyMode::Ebnf => "ebnf",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Yaml | LegacyMode::Turtle => data.with(
            "commentTokens",
            CommentTokens {
                line: Some("#"),
                block: None,
            },
        ),
        LegacyMode::Ebnf => data,
        _ => unreachable!(),
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Yaml(yaml::State),
    Turtle(turtle::State),
    Ebnf(ebnf::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Yaml => Scan::Yaml(yaml::State::default()),
            LegacyMode::Turtle => Scan::Turtle(turtle::State::default()),
            LegacyMode::Ebnf => Scan::Ebnf(ebnf::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Yaml(_) => LegacyMode::Yaml,
            Scan::Turtle(_) => LegacyMode::Turtle,
            Scan::Ebnf(_) => LegacyMode::Ebnf,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Yaml(state) => state.token(stream),
            Scan::Turtle(state) => state.token(stream),
            Scan::Ebnf(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Turtle(state) => Some(state.indent(text_after, indent_unit)),
            Scan::Yaml(_) | Scan::Ebnf(_) => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
