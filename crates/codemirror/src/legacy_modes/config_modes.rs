//! Nginx, Mbox, Asterisk, and RPM streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod asterisk;
mod mbox;
mod nginx;
mod rpm;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Nginx => "nginx",
        LegacyMode::Mbox => "mbox",
        LegacyMode::Asterisk => "asterisk",
        LegacyMode::RpmChanges => "rpmchanges",
        LegacyMode::RpmSpec => "rpmspec",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Nginx => data.with("indentOnInput", nginx_indent_input as fn(&str) -> bool),
        LegacyMode::Mbox => data.with("autocomplete", mbox::HEADERS),
        LegacyMode::Asterisk => data.with(
            "commentTokens",
            CommentTokens {
                line: Some(";"),
                block: Some((";--", "--;")),
            },
        ),
        LegacyMode::RpmChanges | LegacyMode::RpmSpec => data,
        _ => unreachable!(),
    }
}

fn nginx_indent_input(text: &str) -> bool {
    text.trim_start_matches(js_space) == "}"
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Nginx(nginx::State),
    Mbox(mbox::State),
    Asterisk(asterisk::State),
    RpmChanges,
    RpmSpec(rpm::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Nginx => Scan::Nginx(nginx::State::default()),
            LegacyMode::Mbox => Scan::Mbox(mbox::State::default()),
            LegacyMode::Asterisk => Scan::Asterisk(asterisk::State::default()),
            LegacyMode::RpmChanges => Scan::RpmChanges,
            LegacyMode::RpmSpec => Scan::RpmSpec(rpm::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Nginx(_) => LegacyMode::Nginx,
            Scan::Mbox(_) => LegacyMode::Mbox,
            Scan::Asterisk(_) => LegacyMode::Asterisk,
            Scan::RpmChanges => LegacyMode::RpmChanges,
            Scan::RpmSpec(_) => LegacyMode::RpmSpec,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Nginx(state) => state.token(stream),
            Scan::Mbox(state) => state.token(stream),
            Scan::Asterisk(state) => state.token(stream),
            Scan::RpmChanges => rpm::changes_token(stream),
            Scan::RpmSpec(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Nginx(state) => Some(state.indent(text_after, indent_unit)),
            _ => None,
        }
    }

    pub(super) fn blank_line(&mut self, _indent_unit: usize) {
        if let Scan::Mbox(state) = &mut self.0 {
            *state = mbox::State::default();
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
