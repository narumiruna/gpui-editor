//! FCL, Forth, ECL, IDL, Pig, and Puppet streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Tokens and columns follow the native stream's UTF-8 boundaries and scalar columns.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod ecl;
mod fcl;
mod forth;
mod idl;
mod pig;
mod puppet;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Fcl => "fcl",
        LegacyMode::Forth => "forth",
        LegacyMode::Ecl => "ecl",
        LegacyMode::Idl => "idl",
        LegacyMode::Pig => "pig",
        LegacyMode::Puppet => "puppet",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Fcl => data.with(
            "commentTokens",
            CommentTokens {
                line: Some("//"),
                block: Some(("(*", "*)")),
            },
        ),
        LegacyMode::Ecl => data.with("indentOnInput", ecl_indent_input as fn(&str) -> bool),
        LegacyMode::Idl => data.with("autocomplete", idl::autocomplete()),
        LegacyMode::Pig => data.with("autocomplete", pig::autocomplete()),
        LegacyMode::Forth | LegacyMode::Puppet => data,
        _ => unreachable!(),
    }
}

fn ecl_indent_input(text: &str) -> bool {
    matches!(text.trim_start_matches(js_space), "{" | "}")
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Fcl(fcl::State),
    Forth(forth::State),
    Ecl(ecl::State),
    Idl,
    Pig(pig::State),
    Puppet(puppet::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Fcl => Scan::Fcl(fcl::State::new(indent_unit)),
            LegacyMode::Forth => Scan::Forth(forth::State::default()),
            LegacyMode::Ecl => Scan::Ecl(ecl::State::new(indent_unit)),
            LegacyMode::Idl => Scan::Idl,
            LegacyMode::Pig => Scan::Pig(pig::State::default()),
            LegacyMode::Puppet => Scan::Puppet(puppet::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Fcl(_) => LegacyMode::Fcl,
            Scan::Forth(_) => LegacyMode::Forth,
            Scan::Ecl(_) => LegacyMode::Ecl,
            Scan::Idl => LegacyMode::Idl,
            Scan::Pig(_) => LegacyMode::Pig,
            Scan::Puppet(_) => LegacyMode::Puppet,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Fcl(state) => state.token(stream),
            Scan::Forth(state) => state.token(stream),
            Scan::Ecl(state) => state.token(stream),
            Scan::Idl => idl::token(stream),
            Scan::Pig(state) => state.token(stream),
            Scan::Puppet(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Fcl(state) => Some(state.indent(text_after, indent_unit)),
            Scan::Ecl(state) => Some(state.indent(text_after, indent_unit)),
            _ => None,
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn identifier(ch: char) -> bool {
    word(ch) || ch == '$'
}

fn number_tail(ch: char) -> bool {
    word(ch) || ch == '.'
}

fn listed(list: &str, word: &str) -> bool {
    list.split_ascii_whitespace().any(|entry| entry == word)
}

fn close_comment(stream: &mut StringStream<'_>, alternate_close: bool) -> bool {
    let mut star = false;
    for ch in stream {
        if star && (ch == '/' || (alternate_close && ch == ')')) {
            return true;
        }
        star = ch == '*';
    }
    false
}

// ECL and Pig retain a quote only after a final unpaired backslash.
fn string_continues(stream: &mut StringStream<'_>, quote: char) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == quote && !escaped {
            return false;
        }
        escaped = !escaped && ch == '\\';
    }
    escaped
}

#[derive(Clone, Copy, Debug, Default)]
enum ScanMode {
    #[default]
    Base,
    Comment,
    String(char),
}

#[derive(Clone, Debug)]
struct Context<K> {
    kind: K,
    indented: usize,
    column: usize,
    align: Option<bool>,
}

impl<K> Context<K> {
    fn new(kind: K, indented: usize, column: usize) -> Self {
        Self {
            kind,
            indented,
            column,
            align: None,
        }
    }

    fn indent(&self, closing: bool, unit: usize) -> usize {
        if self.align == Some(true) {
            self.column.saturating_add(usize::from(!closing))
        } else {
            self.indented.saturating_add(if closing { 0 } else { unit })
        }
    }
}
