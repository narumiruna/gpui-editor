//! Shell, CMake, and Dockerfile streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod cmake;
mod dockerfile;
mod shell;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    match mode {
        LegacyMode::Shell => Some("shell"),
        LegacyMode::CMake => Some("cmake"),
        LegacyMode::Dockerfile => None,
        _ => unreachable!(),
    }
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    let comments = CommentTokens {
        line: Some("#"),
        block: None,
    };
    match mode {
        LegacyMode::Shell => shell::data(data.with("commentTokens", comments)),
        LegacyMode::CMake => data,
        LegacyMode::Dockerfile => data.with("commentTokens", comments),
        _ => unreachable!(),
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Shell(shell::State),
    CMake(cmake::State),
    Dockerfile(dockerfile::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Shell => Scan::Shell(shell::State::default()),
            LegacyMode::CMake => Scan::CMake(cmake::State::default()),
            LegacyMode::Dockerfile => Scan::Dockerfile(dockerfile::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Shell(_) => LegacyMode::Shell,
            Scan::CMake(_) => LegacyMode::CMake,
            Scan::Dockerfile(_) => LegacyMode::Dockerfile,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Shell(state) => state.token(stream),
            Scan::CMake(state) => state.token(stream),
            Scan::Dockerfile(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, _text_after: &str, _indent_unit: usize) -> Option<usize> {
        // None of the three modes defines indentation rules.
        None
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
