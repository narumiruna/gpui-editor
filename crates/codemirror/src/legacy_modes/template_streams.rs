//! CoffeeScript, LiveScript, PEG.js, and Pug streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Token offsets and columns use the native stream's UTF-8 boundaries and scalar columns.

use super::{LegacyMode, web_modes};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

macro_rules! re {
    ($pattern:expr) => {{
        static RE: std::sync::LazyLock<regress::Regex> =
            std::sync::LazyLock::new(|| regress::Regex::new($pattern).expect("valid mode pattern"));
        &*RE
    }};
}

mod coffeescript;
mod livescript;
mod pegjs;
mod pug;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    match mode {
        LegacyMode::CoffeeScript => Some("coffeescript"),
        LegacyMode::LiveScript => Some("livescript"),
        LegacyMode::PegJs => Some("pegjs"),
        LegacyMode::Pug => None,
        _ => unreachable!(),
    }
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    if mode == LegacyMode::CoffeeScript {
        data.with(
            "commentTokens",
            CommentTokens {
                line: Some("#"),
                block: None,
            },
        )
    } else {
        data
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    CoffeeScript(coffeescript::State),
    LiveScript(livescript::State),
    PegJs(Box<pegjs::State>),
    Pug(Box<pug::State>),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::CoffeeScript => Scan::CoffeeScript(coffeescript::State::default()),
            LegacyMode::LiveScript => Scan::LiveScript(livescript::State::default()),
            LegacyMode::PegJs => Scan::PegJs(Box::default()),
            LegacyMode::Pug => Scan::Pug(Box::new(pug::State::new(indent_unit))),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::CoffeeScript(_) => LegacyMode::CoffeeScript,
            Scan::LiveScript(_) => LegacyMode::LiveScript,
            Scan::PegJs(_) => LegacyMode::PegJs,
            Scan::Pug(_) => LegacyMode::Pug,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::CoffeeScript(state) => state.token(stream),
            Scan::LiveScript(state) => Some(state.token(stream)),
            Scan::PegJs(state) => state.token(stream),
            Scan::Pug(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, _indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::CoffeeScript(state) => Some(state.indent(text_after)),
            Scan::LiveScript(state) => Some(state.indent()),
            _ => None,
        }
    }
}

fn js_state(indent_unit: usize) -> web_modes::State {
    web_modes::State::new(LegacyMode::JavaScript, indent_unit)
}

fn remaining<'a>(stream: &StringStream<'a>) -> &'a str {
    &stream.line()[stream.pos()..]
}

fn prefix(stream: &mut StringStream<'_>, pattern: &regress::Regex) -> bool {
    let Some(found) = pattern.find(remaining(stream)).filter(|m| m.start() == 0) else {
        return false;
    };
    stream.set_pos(stream.pos() + found.end()).unwrap();
    true
}
