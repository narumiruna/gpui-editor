//! MSC, VBScript, and Velocity streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Tokens do not split Unicode scalars. Negative VBScript indentation becomes
//! zero. MSC attribute-list state is local to a scan, not shared across documents.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod mscgen;
mod vbscript;
mod velocity;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Mscgen | LegacyMode::Msgenny | LegacyMode::Xu => "mscgen",
        LegacyMode::VbScript | LegacyMode::VbScriptAsp => "vbscript",
        LegacyMode::Velocity => "velocity",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, open, close) = match mode {
        LegacyMode::Mscgen | LegacyMode::Msgenny | LegacyMode::Xu => ("#", "/*", "*/"),
        LegacyMode::Velocity => ("##", "#*", "*#"),
        LegacyMode::VbScript | LegacyMode::VbScriptAsp => return LanguageData::default(),
        _ => unreachable!(),
    };
    LanguageData::default().with(
        "commentTokens",
        CommentTokens {
            line: Some(line),
            block: Some((open, close)),
        },
    )
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Msc(LegacyMode, mscgen::State),
    Vb(LegacyMode, vbscript::State),
    Velocity(velocity::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Mscgen | LegacyMode::Msgenny | LegacyMode::Xu => {
                Scan::Msc(mode, mscgen::State::default())
            }
            LegacyMode::VbScript | LegacyMode::VbScriptAsp => {
                Scan::Vb(mode, vbscript::State::default())
            }
            LegacyMode::Velocity => Scan::Velocity(velocity::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Msc(mode, _) | Scan::Vb(mode, _) => mode,
            Scan::Velocity(_) => LegacyMode::Velocity,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Msc(mode, state) => state.token(*mode, stream),
            Scan::Vb(mode, state) => state.token(*mode == LegacyMode::VbScriptAsp, stream),
            Scan::Velocity(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Vb(_, state) => Some(state.indent(text_after, indent_unit)),
            _ => None,
        }
    }
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn advance(stream: &mut StringStream<'_>, bytes: usize) {
    stream
        .set_pos(stream.pos() + bytes)
        .expect("matched prefix ends at a scalar boundary");
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn starts_ascii_case(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn word_prefix(text: &str, keyword: &str) -> bool {
    starts_ascii_case(text, keyword) && !text[keyword.len()..].starts_with(word)
}
