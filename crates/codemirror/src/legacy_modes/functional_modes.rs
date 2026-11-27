//! Haskell, ML, and Erlang streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod erlang;
mod haskell;
mod mllike;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    match mode {
        LegacyMode::Haskell => Some("haskell"),
        LegacyMode::Erlang => Some("erlang"),
        LegacyMode::OCaml | LegacyMode::FSharp | LegacyMode::Sml => None,
        _ => unreachable!(),
    }
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, block) = match mode {
        LegacyMode::Haskell => (Some("--"), Some(("{-", "-}"))),
        LegacyMode::Erlang => (Some("%"), None),
        LegacyMode::OCaml => (None, Some(("(*", "*)"))),
        LegacyMode::FSharp | LegacyMode::Sml => (Some("//"), Some(("(*", "*)"))),
        _ => unreachable!(),
    };
    let data = LanguageData::default().with("commentTokens", CommentTokens { line, block });
    match mode {
        LegacyMode::Haskell | LegacyMode::Erlang => data,
        _ => data.with("autocomplete", mllike::Dialect::from_mode(mode).hints()),
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Haskell(haskell::State),
    MlLike(mllike::State),
    Erlang(erlang::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, _indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Haskell => Scan::Haskell(haskell::State::default()),
            LegacyMode::Erlang => Scan::Erlang(erlang::State::default()),
            _ => Scan::MlLike(mllike::State::new(mllike::Dialect::from_mode(mode))),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match &self.0 {
            Scan::Haskell(_) => LegacyMode::Haskell,
            Scan::MlLike(state) => state.mode(),
            Scan::Erlang(_) => LegacyMode::Erlang,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Haskell(state) => state.token(stream),
            Scan::MlLike(state) => state.token(stream),
            Scan::Erlang(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Erlang(state) => state.indent(text_after, unit),
            _ => None,
        }
    }
}

fn ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}
