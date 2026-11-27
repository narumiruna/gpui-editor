//! mIRC, Tcl, and VB streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Tokens do not split Unicode scalars. Negative VB indentation becomes zero.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod mirc;
mod tcl;
mod vb;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Mirc => "mirc",
        LegacyMode::Tcl => "tcl",
        LegacyMode::Vb => "vb",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    match mode {
        LegacyMode::Mirc => data,
        LegacyMode::Tcl => data.with("commentTokens", comments("#")),
        LegacyMode::Vb => data
            .with("commentTokens", comments("'"))
            .with("closeBrackets", &["(", "[", "{", "\""][..])
            .with("autocomplete", vb::autocomplete()),
        _ => unreachable!(),
    }
}

fn comments(line: &'static str) -> CommentTokens<'static> {
    CommentTokens {
        line: Some(line),
        block: None,
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Mirc(mirc::State),
    Tcl(tcl::State),
    Vb(vb::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Mirc => Scan::Mirc(mirc::State::default()),
            LegacyMode::Tcl => Scan::Tcl(tcl::State::default()),
            LegacyMode::Vb => Scan::Vb(vb::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Mirc(_) => LegacyMode::Mirc,
            Scan::Tcl(_) => LegacyMode::Tcl,
            Scan::Vb(_) => LegacyMode::Vb,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Mirc(state) => state.token(stream),
            Scan::Tcl(state) => state.token(stream),
            Scan::Vb(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Vb(state) => Some(state.indent(text_after, indent_unit)),
            _ => None,
        }
    }
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn operator(ch: char) -> bool {
    "+-*&%=<>!?^/|".contains(ch)
}

fn number_tail(ch: char) -> bool {
    word(ch) || ch == '.'
}

const FUNCTIONS: &[&str] = &[
    "if", "elseif", "else", "and", "not", "or", "eq", "ne", "in", "ni", "for", "foreach", "while",
    "switch",
];

#[derive(Clone, Copy, Debug, Default)]
struct Params {
    before: bool,
    inside: bool,
}

impl Params {
    fn punctuation(&mut self, ch: char, before: bool) {
        if ch == '(' && before {
            self.inside = true;
        } else if ch == ')' {
            self.inside = false;
        }
    }
}

fn starts_region(stream: &mut StringStream<'_>, opener: char) -> bool {
    let Some(after) = remaining(stream)
        .trim_start_matches(' ')
        .strip_prefix(opener)
        .and_then(|after| after.trim_start_matches(' ').strip_prefix(opener))
    else {
        return false;
    };
    stream
        .set_pos(stream.line().len() - after.len())
        .expect("prefix ends at a scalar boundary");
    true
}

fn ends_comment(stream: &mut StringStream<'_>, delimiter: char) -> bool {
    let mut star = false;
    for ch in stream {
        if star && ch == delimiter {
            return true;
        }
        star = ch == '*';
    }
    false
}

fn ends_region(stream: &mut StringStream<'_>, closer: char, delimiter: char) -> bool {
    let mut count = 0;
    for ch in stream {
        if count == 2 && ch == delimiter {
            return true;
        }
        if ch == closer {
            // Three or more closers do not end the region.
            count = (count + 1).min(3);
        } else if ch != ' ' {
            count = 0;
        }
    }
    false
}
