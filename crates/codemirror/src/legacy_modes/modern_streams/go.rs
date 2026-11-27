use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, in_words, js_space, word};

const KEYWORDS: &str = "break case chan const continue default defer else fallthrough for func go \
    goto if import interface map package range return select struct switch type var bool byte \
    complex64 complex128 float32 float64 int8 int16 int32 int64 string uint8 uint16 uint32 \
    uint64 int uint uintptr error rune any comparable";
const ATOMS: &str = "true false iota nil append cap close complex copy delete imag len make new \
    panic print println real recover";

static LEADING_DOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]+([eE][-+]?[0-9]+)?").unwrap());
static LEADING_ZERO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[xX][0-9a-fA-F]+|0[0-7]+)").unwrap());
static DECIMAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]*\.?[0-9]*([eE][-+]?[0-9]+)?").unwrap());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scan {
    Base,
    String(char),
    Comment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Top,
    Close(char),
    Case,
}

#[derive(Clone, Debug)]
struct Context {
    kind: Kind,
    indented: usize,
    column: usize,
    align: Option<bool>,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    scan: Scan,
    stack: Vec<Context>,
    indented: usize,
    start_unit: usize,
}

impl State {
    pub(super) fn new(start_unit: usize) -> Self {
        Self {
            scan: Scan::Base,
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

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            let ctx = self.stack.last_mut().unwrap();
            ctx.align.get_or_insert(false);
            self.indented = stream.indentation();
            if ctx.kind == Kind::Case {
                ctx.kind = Kind::Close('}');
            }
        }
        if stream.eat_space() {
            return None;
        }
        let (style, marker) = match self.scan {
            Scan::Base => self.base(stream),
            Scan::String(quote) => (Some(self.string(stream, quote)), None),
            Scan::Comment => (Some(self.comment(stream)), None),
        };
        if style != Some("comment") {
            let ctx = self.stack.last_mut().unwrap();
            ctx.align.get_or_insert(true);
            if let Some(marker) = marker {
                match marker {
                    Marker::Open(close) => self.stack.push(Context {
                        kind: Kind::Close(close),
                        indented: self.indented,
                        column: stream.column(),
                        align: None,
                    }),
                    Marker::Case => ctx.kind = Kind::Case,
                    Marker::Close(ch) if ctx.kind == Kind::Close(ch) && self.stack.len() > 1 => {
                        self.indented = self.stack.pop().unwrap().indented;
                    }
                    Marker::Close(_) => {}
                }
            }
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        if self.scan != Scan::Base {
            return None;
        }
        let ctx = self.stack.last().unwrap();
        let closing = match ctx.kind {
            Kind::Top => false,
            Kind::Close(ch) => text_after.starts_with(ch),
            Kind::Case => ["case", "default"].into_iter().any(|name| {
                text_after
                    .strip_prefix(name)
                    .is_some_and(|rest| rest.chars().next().is_none_or(|ch| !word(ch)))
            }),
        };
        // A case label aligns with the context's indentation, not its column.
        if ctx.align == Some(true) && !(ctx.kind == Kind::Case && closing) {
            Some(ctx.column.saturating_add(usize::from(!closing)))
        } else if self.stack.len() == 1 {
            // The upstream root has indentation -start_unit. Clamp to zero.
            Some((if closing { 0 } else { unit }).saturating_sub(self.start_unit))
        } else {
            Some(ctx.indented.saturating_add(if closing { 0 } else { unit }))
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (Option<&'static str>, Option<Marker>) {
        let Some(ch) = stream.next() else {
            return (None, None);
        };
        match ch {
            '"' | '\'' | '`' => (Some(self.string(stream, ch)), None),
            '.' | '0'..='9' => {
                let pattern = match ch {
                    '.' => &*LEADING_DOT,
                    '0' => &*LEADING_ZERO,
                    _ => &*DECIMAL,
                };
                stream.match_regex(pattern, true);
                (Some("number"), None)
            }
            '{' => (None, Some(Marker::Open('}'))),
            '[' => (None, Some(Marker::Open(']'))),
            '(' => (None, Some(Marker::Open(')'))),
            '}' | ']' | ')' => (None, Some(Marker::Close(ch))),
            ',' | ';' | ':' => (None, None),
            '/' if stream.eat('*').is_some() => (Some(self.comment(stream)), None),
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                (Some("comment"), None)
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                (Some("operator"), None)
            }
            _ => {
                stream.eat_while(|ch| word(ch) || ch == '$' || ch >= '\u{a1}');
                let text = stream.current();
                if in_words(KEYWORDS, text) {
                    (
                        Some("keyword"),
                        matches!(text, "case" | "default").then_some(Marker::Case),
                    )
                } else if in_words(ATOMS, text) {
                    (Some("atom"), None)
                } else {
                    (Some("variable"), None)
                }
            }
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        let mut escaped = false;
        for ch in stream {
            if ch == quote && !escaped {
                self.scan = Scan::Base;
                return "string";
            }
            escaped = !escaped && quote != '`' && ch == '\\';
        }
        self.scan = if escaped || quote == '`' {
            Scan::String(quote)
        } else {
            Scan::Base
        };
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.skip_to("*/") {
            stream.set_pos(stream.pos() + 2).unwrap();
            self.scan = Scan::Base;
        } else {
            stream.skip_to_end();
            self.scan = Scan::Comment;
        }
        "comment"
    }
}

enum Marker {
    Open(char),
    Close(char),
    Case,
}

fn operator(ch: char) -> bool {
    "+-*&^%:=<>!|/".contains(ch)
}

pub(super) fn indent_on_input(text: &str) -> bool {
    let mut chars = text.chars();
    if !chars.next().is_some_and(js_space) {
        return false;
    }
    let rest = chars.as_str();
    matches!(rest, "{" | "}" | "case ")
        || rest
            .strip_prefix("default")
            .is_some_and(|rest| rest.trim_start_matches(js_space) == ":")
}
