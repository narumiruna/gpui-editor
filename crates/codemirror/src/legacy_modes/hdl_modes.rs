//! Verilog, TL-Verilog, and VHDL streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Token boundaries and alignment columns use Unicode scalars. In TL-Verilog,
//! fallback at a surrogate boundary can differ from the upstream UTF-16 scan.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod tlv;
mod verilog;
mod vhdl;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Verilog | LegacyMode::Tlv => "verilog",
        LegacyMode::Vhdl => "vhdl",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let (line, block, trigger) = match mode {
        LegacyMode::Verilog | LegacyMode::Tlv => (
            "//",
            Some(("/*", "*/")),
            verilog::indent_on_input as fn(&str) -> bool,
        ),
        LegacyMode::Vhdl => ("--", None, vhdl::indent_on_input as fn(&str) -> bool),
        _ => unreachable!(),
    };
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(line),
                block,
            },
        )
        .with("indentOnInput", trigger)
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Verilog(verilog::State),
    Tlv(verilog::State, tlv::State),
    Vhdl(vhdl::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Verilog => Scan::Verilog(verilog::State::new(indent_unit)),
            LegacyMode::Tlv => Scan::Tlv(verilog::State::new(indent_unit), tlv::State::default()),
            LegacyMode::Vhdl => Scan::Vhdl(vhdl::State::new(indent_unit)),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Verilog(_) => LegacyMode::Verilog,
            Scan::Tlv(..) => LegacyMode::Tlv,
            Scan::Vhdl(_) => LegacyMode::Vhdl,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Verilog(state) => state.token(stream, None),
            Scan::Tlv(state, tlv) => state.token(stream, Some(tlv)),
            Scan::Vhdl(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Verilog(state) => state.indent(text_after, unit, None),
            Scan::Tlv(state, tlv) => state.indent(text_after, unit, tlv.indent()),
            Scan::Vhdl(state) => Some(state.indent(text_after, unit)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Top,
    Statement,
    Bracket(char),
    Block(&'static str),
}

#[derive(Clone, Copy, Debug)]
struct Context {
    kind: Kind,
    indented: usize,
    column: usize,
    align: Option<bool>,
}

#[derive(Clone, Debug)]
struct Contexts {
    stack: Vec<Context>,
    indented: usize,
    start_unit: usize,
}

impl Contexts {
    fn new(start_unit: usize) -> Self {
        Self {
            stack: vec![Context {
                kind: Kind::Top,
                indented: 0,
                column: 0,
                align: Some(false),
            }],
            indented: 0,
            start_unit,
        }
    }

    fn current(&self) -> Context {
        self.stack[self.stack.len() - 1]
    }

    fn outer(&self) -> Context {
        self.stack[self.stack.len().saturating_sub(2)]
    }

    fn start_line(&mut self, stream: &StringStream<'_>) {
        if stream.sol() {
            self.set_alignment(false);
            self.indented = stream.indentation();
        }
    }

    fn set_alignment(&mut self, align: bool) {
        self.stack.last_mut().unwrap().align.get_or_insert(align);
    }

    fn push(&mut self, kind: Kind, stream: &StringStream<'_>) {
        self.stack.push(Context {
            kind,
            indented: self.indented,
            column: stream.column(),
            align: None,
        });
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            let ctx = self.stack.pop().unwrap();
            if matches!(ctx.kind, Kind::Bracket(_)) {
                self.indented = ctx.indented;
            }
        }
    }

    fn pop_statements(&mut self) {
        while self.current().kind == Kind::Statement {
            self.pop();
        }
    }

    fn indent_from(&self, ctx: Context, extra: usize) -> usize {
        if ctx.kind == Kind::Top {
            // The upstream root has indentation -start_unit. Clamp to zero.
            extra.saturating_sub(self.start_unit)
        } else {
            ctx.indented.saturating_add(extra)
        }
    }
}

fn bracket_close(ch: char) -> Option<char> {
    Some(match ch {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    })
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn identifier(ch: char) -> bool {
    word(ch) || ch == '$'
}
