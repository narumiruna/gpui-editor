//! Mathematica, Octave, SAS, q, and Modelica streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod mathematica;
mod modelica;
mod octave;
mod q;
mod sas;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Mathematica => "mathematica",
        LegacyMode::Octave => "octave",
        LegacyMode::Sas => "sas",
        LegacyMode::Q => "q",
        LegacyMode::Modelica => "modelica",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, block) = match mode {
        LegacyMode::Mathematica => (None, Some(("(*", "*)"))),
        LegacyMode::Octave => (Some("%"), None),
        LegacyMode::Sas => (None, Some(("/*", "*/"))),
        LegacyMode::Q => (Some("/"), None),
        LegacyMode::Modelica => (Some("//"), Some(("/*", "*/"))),
        _ => unreachable!(),
    };
    let data = LanguageData::default().with("commentTokens", CommentTokens { line, block });
    if mode == LegacyMode::Modelica {
        data.with("autocomplete", modelica::completions())
    } else {
        data
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Mathematica(mathematica::State),
    Octave(octave::State),
    Sas(sas::State),
    Q(q::State),
    Modelica(modelica::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, _indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Mathematica => Scan::Mathematica(mathematica::State::default()),
            LegacyMode::Octave => Scan::Octave(octave::State::default()),
            LegacyMode::Sas => Scan::Sas(sas::State::default()),
            LegacyMode::Q => Scan::Q(q::State::default()),
            LegacyMode::Modelica => Scan::Modelica(modelica::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Mathematica(_) => LegacyMode::Mathematica,
            Scan::Octave(_) => LegacyMode::Octave,
            Scan::Sas(_) => LegacyMode::Sas,
            Scan::Q(_) => LegacyMode::Q,
            Scan::Modelica(_) => LegacyMode::Modelica,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Mathematica(state) => state.token(stream),
            Scan::Octave(state) => state.token(stream),
            Scan::Sas(state) => state.token(stream),
            Scan::Q(state) => state.token(stream),
            Scan::Modelica(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Modelica(state) => state.indent(text_after, unit),
            // Upstream q never records punctuation, so its context stays empty.
            Scan::Q(_) => Some(0),
            _ => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn close_string(stream: &mut StringStream<'_>) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == '"' && !escaped {
            return true;
        }
        escaped = !escaped && ch == '\\';
    }
    false
}
