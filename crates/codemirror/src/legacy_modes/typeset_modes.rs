//! Concrete modes from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod sass;
mod stex;
mod stylus;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Sass => "sass",
        LegacyMode::Stex | LegacyMode::StexMath => "stex",
        LegacyMode::Stylus => "stylus",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, block) = match mode {
        LegacyMode::Sass | LegacyMode::Stylus => ("//", Some(("/*", "*/"))),
        LegacyMode::Stex | LegacyMode::StexMath => ("%", None),
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
        LegacyMode::Sass => data.with("autocomplete", super::CSS_KEYWORDS.all),
        LegacyMode::Stylus => data
            .with("autocomplete", stylus::AUTOCOMPLETE)
            .with("indentOnInput", indent_on_input as fn(&str) -> bool),
        _ => data,
    }
}

fn indent_on_input(text: &str) -> bool {
    text.trim_start_matches(js_space) == "}"
}

#[derive(Clone, Debug)]
pub(super) enum State {
    Sass(sass::State),
    Stex(LegacyMode, stex::State),
    Stylus(stylus::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        match mode {
            LegacyMode::Sass => Self::Sass(sass::State::default()),
            LegacyMode::Stex | LegacyMode::StexMath => {
                Self::Stex(mode, stex::State::new(mode == LegacyMode::StexMath))
            }
            LegacyMode::Stylus => Self::Stylus(stylus::State::default()),
            _ => unreachable!(),
        }
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self {
            Self::Sass(_) => LegacyMode::Sass,
            Self::Stex(mode, _) => *mode,
            Self::Stylus(_) => LegacyMode::Stylus,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self {
            Self::Sass(state) => state.token(stream),
            Self::Stex(_, state) => state.token(stream),
            Self::Stylus(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match self {
            Self::Sass(state) => Some(state.indent()),
            Self::Stex(..) => None,
            Self::Stylus(state) => Some(state.indent(text_after, indent_unit)),
        }
    }

    pub(super) fn blank_line(&mut self) {
        if let Self::Stex(_, state) = self {
            state.blank_line();
        }
    }
}
