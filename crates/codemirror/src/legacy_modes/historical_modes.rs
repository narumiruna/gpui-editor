//! Dylan, Eiffel, MUMPS, Oz, and Yacas streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Token boundaries use Unicode scalars. Indentation clamps negative results
//! to zero. Yacas consumes an error scalar where upstream would not advance.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod dylan;
mod eiffel;
mod mumps;
mod oz;
mod yacas;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Dylan => "dylan",
        LegacyMode::Eiffel => "eiffel",
        LegacyMode::Mumps => "mumps",
        LegacyMode::Oz => "oz",
        LegacyMode::Yacas => "yacas",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    let block = Some(("/*", "*/"));
    let comments = match mode {
        LegacyMode::Dylan => CommentTokens { line: None, block },
        LegacyMode::Eiffel => CommentTokens {
            line: Some("--"),
            block: None,
        },
        LegacyMode::Mumps => return data,
        LegacyMode::Oz => CommentTokens {
            line: Some("%"),
            block,
        },
        LegacyMode::Yacas => CommentTokens {
            line: Some("//"),
            block,
        },
        _ => unreachable!(),
    };
    let data = data.with("commentTokens", comments);
    match mode {
        // Keep the original keys, including the spelling of the Oz key.
        LegacyMode::Oz => data.with("indentOnInut", oz::electric_input as fn(&str) -> bool),
        LegacyMode::Yacas => data.with("electricInput", yacas::electric_input as fn(&str) -> bool),
        _ => data,
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Dylan(dylan::State),
    Eiffel(eiffel::State),
    Mumps(mumps::State),
    Oz(oz::State),
    Yacas(yacas::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Dylan => Scan::Dylan(dylan::State::default()),
            LegacyMode::Eiffel => Scan::Eiffel(eiffel::State::default()),
            LegacyMode::Mumps => Scan::Mumps(mumps::State::default()),
            LegacyMode::Oz => Scan::Oz(oz::State::default()),
            LegacyMode::Yacas => Scan::Yacas(yacas::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Dylan(_) => LegacyMode::Dylan,
            Scan::Eiffel(_) => LegacyMode::Eiffel,
            Scan::Mumps(_) => LegacyMode::Mumps,
            Scan::Oz(_) => LegacyMode::Oz,
            Scan::Yacas(_) => LegacyMode::Yacas,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Dylan(state) => state.token(stream),
            Scan::Eiffel(state) => state.token(stream),
            Scan::Mumps(state) => state.token(stream),
            Scan::Oz(state) => state.token(stream),
            Scan::Yacas(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Oz(state) => Some(state.indent(text_after, unit)),
            Scan::Yacas(state) => state.indent(text_after, unit),
            _ => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn leading_word(text: &str) -> &str {
    let end = text.find(|ch| !word(ch)).unwrap_or(text.len());
    &text[..end]
}

#[derive(PartialEq, Eq)]
enum QuoteEnd {
    Closed,
    Escaped,
    Open,
}

fn quoted(stream: &mut StringStream<'_>, quote: char, escape: char) -> QuoteEnd {
    let mut escaped = false;
    for ch in stream {
        if ch == quote && !escaped {
            return QuoteEnd::Closed;
        }
        escaped = !escaped && ch == escape;
    }
    if escaped {
        QuoteEnd::Escaped
    } else {
        QuoteEnd::Open
    }
}

fn block_comment_ended(stream: &mut StringStream<'_>, nested: bool) -> bool {
    let mut depth = 0;
    let mut previous = None;
    for ch in stream {
        match (previous, ch) {
            (Some('*'), '/') if depth == 0 => return true,
            (Some('*'), '/') => depth -= 1,
            (Some('/'), '*') if nested => depth += 1,
            _ => {}
        }
        previous = Some(ch);
    }
    false
}
