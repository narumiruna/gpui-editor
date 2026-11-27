//! Cypher, Solr, Sieve, Web IDL, and XQuery streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod cypher;
mod sieve;
mod solr;
mod webidl;
mod xquery;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Cypher => "cypher",
        LegacyMode::Solr => "solr",
        LegacyMode::Sieve => "sieve",
        LegacyMode::WebIdl => "webidl",
        LegacyMode::XQuery => "xquery",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Cypher | LegacyMode::Solr => data,
        LegacyMode::Sieve => data.with("indentOnInput", sieve_indent_input as fn(&str) -> bool),
        LegacyMode::WebIdl => data.with("autocomplete", webidl::autocomplete()),
        LegacyMode::XQuery => data.with(
            "commentTokens",
            CommentTokens {
                line: None,
                block: Some(("(:", ":)")),
            },
        ),
        _ => unreachable!(),
    }
}

fn sieve_indent_input(text: &str) -> bool {
    text.trim_start_matches(js_space) == "}"
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Cypher(cypher::State),
    Solr(solr::State),
    Sieve(sieve::State),
    WebIdl(webidl::State),
    XQuery(xquery::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Cypher => Scan::Cypher(cypher::State::default()),
            LegacyMode::Solr => Scan::Solr(solr::State::default()),
            LegacyMode::Sieve => Scan::Sieve(sieve::State::default()),
            LegacyMode::WebIdl => Scan::WebIdl(webidl::State::default()),
            LegacyMode::XQuery => Scan::XQuery(xquery::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Cypher(_) => LegacyMode::Cypher,
            Scan::Solr(_) => LegacyMode::Solr,
            Scan::Sieve(_) => LegacyMode::Sieve,
            Scan::WebIdl(_) => LegacyMode::WebIdl,
            Scan::XQuery(_) => LegacyMode::XQuery,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Cypher(state) => state.token(stream),
            Scan::Solr(state) => state.token(stream),
            Scan::Sieve(state) => state.token(stream),
            Scan::WebIdl(state) => state.token(stream),
            Scan::XQuery(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Cypher(state) => Some(state.indent(text_after, indent_unit)),
            Scan::Sieve(state) => Some(state.indent(text_after, indent_unit)),
            Scan::Solr(_) | Scan::WebIdl(_) | Scan::XQuery(_) => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

// Solr and Sieve keep the quote open only if this line ends with an escape.
fn quote_continues(stream: &mut StringStream<'_>) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == '"' && !escaped {
            return false;
        }
        escaped = !escaped && ch == '\\';
    }
    escaped
}
