//! R, Julia, Lua, and Perl streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Streams use UTF-8 scalar boundaries. Negative upstream indentation is
//! clamped to zero, and a token always advances the cursor.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod julia;
mod lua;
mod perl;
mod r;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::R => "r",
        LegacyMode::Julia => "julia",
        LegacyMode::Lua => "lua",
        LegacyMode::Perl => "perl",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, block) = match mode {
        LegacyMode::R | LegacyMode::Perl => ("#", None),
        LegacyMode::Julia => ("#", Some(("#=", "=#"))),
        LegacyMode::Lua => ("--", Some(("--[[", "]]--"))),
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
        LegacyMode::R => r::data(data),
        LegacyMode::Julia => julia::data(data),
        LegacyMode::Lua => data.with("indentOnInput", lua::indent_on_input as fn(&str) -> bool),
        LegacyMode::Perl => data.with("wordChars", "$"),
        _ => unreachable!(),
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    R(r::State),
    Julia(julia::State),
    Lua(lua::State),
    Perl(perl::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::R => Scan::R(r::State::new(indent_unit)),
            LegacyMode::Julia => Scan::Julia(julia::State::default()),
            LegacyMode::Lua => Scan::Lua(lua::State::default()),
            LegacyMode::Perl => Scan::Perl(perl::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::R(_) => LegacyMode::R,
            Scan::Julia(_) => LegacyMode::Julia,
            Scan::Lua(_) => LegacyMode::Lua,
            Scan::Perl(_) => LegacyMode::Perl,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::R(state) => state.token(stream),
            Scan::Julia(state) => state.token(stream),
            Scan::Lua(state) => state.token(stream),
            Scan::Perl(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::R(state) => Some(state.indent(text_after, unit)),
            Scan::Julia(state) => Some(state.indent(text_after, unit)),
            Scan::Lua(state) => Some(state.indent(text_after, unit)),
            Scan::Perl(_) => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn eat_text(stream: &mut StringStream<'_>, text: &str) -> bool {
    stream.match_str(text, true, false)
}
