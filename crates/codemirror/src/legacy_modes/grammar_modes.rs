//! DTD, NTriples, and SPARQL streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod dtd;
mod ntriples;
mod sparql;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Dtd => "dtd",
        LegacyMode::NTriples => "ntriples",
        LegacyMode::Sparql => "sparql",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Dtd => data.with("indentOnInput", dtd_indent_input as fn(&str) -> bool),
        LegacyMode::NTriples => data,
        LegacyMode::Sparql => data.with(
            "commentTokens",
            CommentTokens {
                line: Some("#"),
                block: None,
            },
        ),
        _ => unreachable!(),
    }
}

fn dtd_indent_input(text: &str) -> bool {
    matches!(text.trim_start_matches(js_space), "]" | ">")
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Dtd(dtd::State),
    NTriples,
    Sparql(sparql::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Dtd => Scan::Dtd(dtd::State::default()),
            LegacyMode::NTriples => Scan::NTriples,
            LegacyMode::Sparql => Scan::Sparql(sparql::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Dtd(_) => LegacyMode::Dtd,
            Scan::NTriples => LegacyMode::NTriples,
            Scan::Sparql(_) => LegacyMode::Sparql,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Dtd(state) => state.token(stream),
            Scan::NTriples => ntriples::token(stream),
            Scan::Sparql(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Dtd(state) => Some(state.indent(text_after, indent_unit)),
            Scan::NTriples => None,
            Scan::Sparql(state) => Some(state.indent(text_after, indent_unit)),
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

// The escape flag starts afresh on each call, including on continuation lines.
fn close_quote(stream: &mut StringStream<'_>, quote: char) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == quote && !escaped {
            return true;
        }
        escaped = !escaped && ch == '\\';
    }
    false
}
