//! Haxe, HXML, Spreadsheet, TTCN, and TTCN-CFG streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Token boundaries and alignment columns use Unicode scalars. Negative Haxe
//! indentation becomes zero. Spreadsheet strings advance through Unicode line
//! separators instead of stalling.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod haxe;
mod hxml;
mod spreadsheet;
mod ttcn;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Haxe => "haxe",
        LegacyMode::Hxml => "hxml",
        LegacyMode::Spreadsheet => "spreadsheet",
        LegacyMode::Ttcn | LegacyMode::TtcnCfg => "ttcn",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let mut data = LanguageData::default();
    let (line, block) = match mode {
        LegacyMode::Haxe | LegacyMode::Ttcn => ("//", Some(("/*", "*/"))),
        LegacyMode::Hxml | LegacyMode::TtcnCfg => ("#", None),
        LegacyMode::Spreadsheet => return data,
        _ => unreachable!(),
    };
    data = data.with(
        "commentTokens",
        CommentTokens {
            line: Some(line),
            block,
        },
    );
    if mode != LegacyMode::Hxml {
        data = data.with("indentOnInput", indent_on_input as fn(&str) -> bool);
    }
    if mode == LegacyMode::Ttcn {
        data = data.with("autocomplete", ttcn::autocomplete());
    }
    data
}

fn indent_on_input(after: &str) -> bool {
    matches!(after.trim_start_matches(js_space), "{" | "}")
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Haxe(haxe::State),
    Hxml(hxml::State),
    Spreadsheet(spreadsheet::State),
    Ttcn(ttcn::State),
    TtcnCfg(ttcn::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Haxe => Scan::Haxe(haxe::State::new(indent_unit)),
            LegacyMode::Hxml => Scan::Hxml(hxml::State::default()),
            LegacyMode::Spreadsheet => Scan::Spreadsheet(spreadsheet::State::default()),
            LegacyMode::Ttcn => Scan::Ttcn(ttcn::State::default()),
            LegacyMode::TtcnCfg => Scan::TtcnCfg(ttcn::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Haxe(_) => LegacyMode::Haxe,
            Scan::Hxml(_) => LegacyMode::Hxml,
            Scan::Spreadsheet(_) => LegacyMode::Spreadsheet,
            Scan::Ttcn(_) => LegacyMode::Ttcn,
            Scan::TtcnCfg(_) => LegacyMode::TtcnCfg,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Haxe(state) => state.token(stream),
            Scan::Hxml(state) => state.token(stream),
            Scan::Spreadsheet(state) => state.token(stream),
            Scan::Ttcn(state) => state.token(stream, false),
            Scan::TtcnCfg(state) => state.token(stream, true),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Haxe(state) => Some(state.indent(text_after, unit)),
            _ => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn digit(ch: char) -> bool {
    ch.is_ascii_digit()
}

fn until_unescaped(stream: &mut StringStream<'_>, end: char) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == end && !escaped {
            return true;
        }
        escaped = !escaped && ch == '\\';
    }
    false
}

fn closes_comment(stream: &mut StringStream<'_>) -> bool {
    let mut star = false;
    for ch in stream {
        if star && ch == '/' {
            return true;
        }
        star = ch == '*';
    }
    false
}
