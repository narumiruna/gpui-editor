//! APL, Fortran, COBOL, Pascal, and Smalltalk streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Tokens use UTF-8 scalar boundaries and fixed columns count scalars. Indent
//! results truncate to non-negative whole columns.

use std::collections::HashMap;

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod apl;
mod cobol;
mod fortran;
mod pascal;
mod smalltalk;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Apl => "apl",
        LegacyMode::Fortran => "fortran",
        LegacyMode::Cobol => "cobol",
        LegacyMode::Pascal => "pascal",
        LegacyMode::Smalltalk => "smalltalk",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Pascal => data
            .with(
                "commentTokens",
                CommentTokens {
                    line: None,
                    block: Some(("(*", "*)")),
                },
            )
            .with("indentOnInput", pascal_indent_on_input as fn(&str) -> bool),
        LegacyMode::Smalltalk => data.with(
            "indentOnInput",
            smalltalk_indent_on_input as fn(&str) -> bool,
        ),
        LegacyMode::Apl | LegacyMode::Fortran | LegacyMode::Cobol => data,
        _ => unreachable!(),
    }
}

fn pascal_indent_on_input(input: &str) -> bool {
    matches!(input.trim_start_matches(js_space), "{" | "}")
}

fn smalltalk_indent_on_input(input: &str) -> bool {
    input.trim_start_matches(js_space) == "]"
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Apl,
    Fortran(fortran::State),
    Cobol(cobol::State),
    Pascal(pascal::State),
    Smalltalk(smalltalk::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Apl => Scan::Apl,
            LegacyMode::Fortran => Scan::Fortran(fortran::State::default()),
            LegacyMode::Cobol => Scan::Cobol(cobol::State::default()),
            LegacyMode::Pascal => Scan::Pascal(pascal::State::default()),
            LegacyMode::Smalltalk => Scan::Smalltalk(smalltalk::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Apl => LegacyMode::Apl,
            Scan::Fortran(_) => LegacyMode::Fortran,
            Scan::Cobol(_) => LegacyMode::Cobol,
            Scan::Pascal(_) => LegacyMode::Pascal,
            Scan::Smalltalk(_) => LegacyMode::Smalltalk,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Apl => apl::token(stream),
            Scan::Fortran(state) => state.token(stream),
            Scan::Cobol(state) => state.token(stream),
            Scan::Pascal(state) => state.token(stream),
            Scan::Smalltalk(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Cobol(state) => Some(state.indent()),
            Scan::Smalltalk(state) => Some(state.indent(text_after, indent_unit)),
            _ => None,
        }
    }

    pub(super) fn blank_line(&mut self, _indent_unit: usize) {
        if let Scan::Smalltalk(state) = &mut self.0 {
            state.blank_line();
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn identifier(ch: char) -> bool {
    word(ch) || ch == '$'
}

fn number_tail(ch: char) -> bool {
    word(ch) || ch == '.'
}

// Fortran and Pascal keep the quote active only after a final unpaired backslash.
fn string_continues(stream: &mut StringStream<'_>, quote: char) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == quote && !escaped {
            return false;
        }
        escaped = !escaped && ch == '\\';
    }
    escaped
}

// Later tables take precedence when a word has more than one style.
fn word_styles(tables: &[(&'static str, &'static str)]) -> HashMap<&'static str, &'static str> {
    tables
        .iter()
        .flat_map(|&(style, words)| {
            words
                .split_ascii_whitespace()
                .map(move |word| (word, style))
        })
        .collect()
}
