//! Markup streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use std::borrow::Cow;

use super::LegacyMode;
use crate::stream_parser::StringStream;

mod gherkin;
mod textile;
mod tiddlywiki;
mod tiki;
mod troff;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Gherkin => "gherkin",
        LegacyMode::Troff => "troff",
        LegacyMode::Textile => "textile",
        LegacyMode::TiddlyWiki => "tiddlywiki",
        LegacyMode::Tiki => "tiki",
        _ => unreachable!(),
    })
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Gherkin(gherkin::State),
    Troff,
    Textile(textile::State),
    TiddlyWiki(tiddlywiki::State),
    Tiki(tiki::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Gherkin => Scan::Gherkin(gherkin::State::default()),
            LegacyMode::Troff => Scan::Troff,
            LegacyMode::Textile => Scan::Textile(textile::State::default()),
            LegacyMode::TiddlyWiki => Scan::TiddlyWiki(tiddlywiki::State::default()),
            LegacyMode::Tiki => Scan::Tiki(tiki::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Gherkin(_) => LegacyMode::Gherkin,
            Scan::Troff => LegacyMode::Troff,
            Scan::Textile(_) => LegacyMode::Textile,
            Scan::TiddlyWiki(_) => LegacyMode::TiddlyWiki,
            Scan::Tiki(_) => LegacyMode::Tiki,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<Cow<'static, str>> {
        match &mut self.0 {
            Scan::Gherkin(state) => state.token(stream),
            Scan::Troff => troff::token(stream),
            Scan::Textile(state) => return state.token(stream),
            Scan::TiddlyWiki(state) => state.token(stream),
            Scan::Tiki(state) => state.token(stream),
        }
        .map(Cow::Borrowed)
    }

    pub(super) fn indent(&self, _text_after: &str, _indent_unit: usize) -> Option<usize> {
        // Tiki only pushes contexts for strings, without the start-of-line
        // flag that its indent callback requires for a nonzero result.
        matches!(self.0, Scan::Tiki(_)).then_some(0)
    }

    pub(super) fn blank_line(&mut self) {
        if let Scan::Textile(state) = &mut self.0 {
            state.blank_line();
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn eat_str(stream: &mut StringStream<'_>, text: &str) -> bool {
    stream.match_str(text, true, false)
}

fn remaining<'a>(stream: &StringStream<'a>) -> &'a str {
    &stream.line()[stream.pos()..]
}

fn advance(stream: &mut StringStream<'_>, bytes: usize) {
    stream.set_pos(stream.pos() + bytes).unwrap();
}
