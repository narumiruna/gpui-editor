use std::sync::LazyLock;

use regex::Regex;

use super::{eat_text, remaining, word};
use crate::{
    language::LanguageData,
    legacy_modes::{js_line_terminator, js_space},
    stream_parser::StringStream,
};

const OPENERS: &str = "begin function type struct immutable let macro for while quote if else elseif try finally catch do";
const CLOSERS: &str = "end else elseif catch finally";
const KEYWORDS: &str = "if else elseif while for begin let end do try catch finally return break continue
    global local const export import importall using function where macro module baremodule struct type
    mutable immutable quote typealias abstract primitive bitstype";
const BUILTINS: &str = "true false nothing NaN Inf";

pub(super) fn data(data: LanguageData) -> LanguageData {
    static COMPLETIONS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        [KEYWORDS, BUILTINS]
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .collect()
    });
    data.with("indentOnInput", indent_on_input as fn(&str) -> bool)
        .with("closeBrackets", &["(", "[", "{", "\""][..])
        .with("autocomplete", COMPLETIONS.as_slice())
}

fn indent_on_input(line: &str) -> bool {
    matches!(
        line.trim_start_matches(js_space),
        "end" | "else" | "catch" | "finally"
    )
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    scopes: Vec<Scope>,
    last: Last,
    leaving_expr: bool,
    definition: bool,
    arrays: usize,
    generators: usize,
    comments: usize,
    parameters: usize,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Annotation,
    Comment,
    Char,
    String(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    Block,
    Array,
    Generator,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Last {
    #[default]
    Other,
    Definition,
    End,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = match self.scan {
            Scan::Base => self.base(stream),
            Scan::Annotation => Some(self.annotation(stream)),
            Scan::Comment => Some(self.comment(stream)),
            Scan::Char => Some(self.character(stream)),
            Scan::String(delimiter) => Some(self.string(stream, delimiter)),
        };
        if style.is_some() && !stream.current().is_empty() {
            self.last = match stream.current() {
                "function" | "macro" | "type" | "struct" | "immutable" => Last::Definition,
                "end" => Last::End,
                _ => Last::Other,
            };
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let closing = matches!(text_after, "]" | ")")
            || keyword(text_after, "end catch").is_some()
            || text_after.starts_with("else")
            || text_after.starts_with("finally");
        self.scopes
            .len()
            .saturating_sub(usize::from(closing))
            .saturating_mul(unit)
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if remaining(stream).starts_with("#=") {
            self.scan = Scan::Comment;
            return Some(self.comment(stream));
        }
        let leaving_expr = std::mem::take(&mut self.leaving_expr) && !stream.sol();
        if leaving_expr && stream.eat_while(|ch| ch == '\'') {
            return Some("operator");
        }
        if stream.peek() == Some('.') {
            let dots = remaining(stream)
                .bytes()
                .take_while(|&ch| ch == b'.')
                .count();
            stream.set_pos(stream.pos() + dots).unwrap();
            return Some(if dots >= 4 { "error" } else { "operator" });
        }
        if stream.eat_space() {
            return None;
        }
        let ch = stream.peek()?;
        match ch {
            '#' => {
                stream.skip_to_end();
                return Some("comment");
            }
            '[' => {
                self.scopes.push(Scope::Array);
                self.arrays += 1;
            }
            '(' => {
                self.scopes.push(Scope::Generator);
                self.generators += 1;
            }
            ']' if self.arrays > 0 => {
                self.close(Scope::Array);
                self.arrays -= 1;
            }
            ')' if self.generators > 0 => {
                self.close(Scope::Generator);
                self.generators -= 1;
            }
            _ => {}
        }
        if self.arrays > 0 {
            if self.last == Last::End && stream.eat(':').is_some() {
                return Some("operator");
            }
            if eat_text(stream, "end") {
                return Some("number");
            }
        }
        let tail = remaining(stream);
        if keyword(tail, OPENERS).is_some() {
            self.scopes.push(Scope::Block);
        }
        if keyword(tail, CLOSERS).is_some() {
            self.scopes.pop();
        }
        if tail
            .strip_prefix("::")
            .is_some_and(|tail| !tail.starts_with([':', '$']))
        {
            eat_text(stream, "::");
            self.scan = Scan::Annotation;
            return Some(self.annotation(stream));
        }
        if !leaving_expr && decorated(stream, ':') {
            return Some("builtin");
        }
        if operator(stream) {
            return Some("operator");
        }
        if ch.is_ascii_digit() {
            number(stream);
            if remaining(stream)
                .strip_prefix("im")
                .is_some_and(|tail| !tail.starts_with(word))
            {
                eat_text(stream, "im");
            }
            self.leaving_expr = true;
            return Some("number");
        }
        if stream.eat('\'').is_some() {
            self.scan = Scan::Char;
            return Some(self.character(stream));
        }
        if let Some(delimiter) = string_prefix(stream) {
            self.scan = Scan::String(delimiter);
            return Some(self.string(stream, delimiter));
        }
        if decorated(stream, '@') {
            return Some("meta");
        }
        if ";,()[]{}".contains(ch) {
            stream.next();
            return None;
        }
        if let Some(keyword) = keyword(tail, KEYWORDS) {
            eat_text(stream, keyword);
            return Some("keyword");
        }
        if let Some(builtin) = keyword(tail, BUILTINS) {
            eat_text(stream, builtin);
            return Some("builtin");
        }
        if identifier(stream, false) {
            if self.definition || self.last == Last::Definition {
                self.definition = stream.peek() == Some('.');
                return Some(if self.definition { "variable" } else { "def" });
            }
            self.leaving_expr = true;
            return Some("variable");
        }
        stream.next();
        Some("error")
    }

    fn close(&mut self, scope: Scope) {
        while let Some(top) = self.scopes.pop() {
            if top == scope {
                break;
            }
        }
        self.leaving_expr = true;
    }

    fn annotation(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        stream.eat_while(|ch| !js_space(ch) && !",;{}()=".contains(ch));
        if stream.eat('{').is_some() {
            self.parameters += 1;
        } else if stream.eat('}').is_some() && self.parameters > 0 {
            self.parameters -= 1;
        }
        if self.parameters > 0 {
            let text = remaining(stream).split(js_line_terminator).next().unwrap();
            if let Some(offset) = text.find(['{', '}']) {
                stream.set_pos(stream.pos() + offset).unwrap();
            } else {
                stream.next();
            }
        } else {
            self.scan = Scan::Base;
        }
        "builtin"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if eat_text(stream, "#=") {
            self.comments += 1;
        }
        let line = remaining(stream).split(js_line_terminator).next().unwrap();
        let next = line.find("#=").into_iter().chain(line.find("=#")).min();
        if let Some(offset) = next {
            stream.set_pos(stream.pos() + offset).unwrap();
            if eat_text(stream, "=#") {
                self.comments -= 1;
                if self.comments == 0 {
                    self.scan = Scan::Base;
                }
            }
        } else {
            stream.skip_to_end();
        }
        "comment"
    }

    fn character(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        static CHAR: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"^(?:\\[0-7]{1,3}|\\x[0-9A-Fa-f]{1,2}|\\[abefnrtv0%?'"\\]|[^'\\])'"#)
                .unwrap()
        });
        let valid = stream.match_regex(&CHAR, true).is_some() || unicode_character(stream);
        if valid {
            self.leaving_expr = true;
            self.scan = Scan::Base;
            return "string";
        }
        if stream.peek() != Some('\'') && stream.skip_to("'") {
            stream.next();
            self.scan = Scan::Base;
        } else {
            stream.skip_to_end();
        }
        "error"
    }

    fn string(&mut self, stream: &mut StringStream<'_>, delimiter: &'static str) -> &'static str {
        if stream.eat('\\').is_some() {
            stream.next();
        } else if eat_text(stream, delimiter) {
            self.scan = Scan::Base;
            self.leaving_expr = true;
            return "string";
        } else {
            stream.eat_if(|ch| matches!(ch, '`' | '"'));
        }
        stream.eat_while(|ch| !matches!(ch, '\\' | '`' | '"'));
        "string"
    }
}

fn keyword(text: &str, words: &'static str) -> Option<&'static str> {
    words.split_ascii_whitespace().find(|item| {
        text.strip_prefix(item)
            .is_some_and(|rest| !rest.starts_with(word))
    })
}

fn decorated(stream: &mut StringStream<'_>, prefix: char) -> bool {
    static ASCII_OPERATOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(?:[<>]:|[<>=]=|<<=?|>>>?=?|=>|--?>|<--[->]?|//|\.{2,3}|[.\\%*+\-<>!/^|&]=?|[?$~:])",
        )
        .unwrap()
    });
    let pos = stream.pos();
    if stream.eat(prefix).is_none() {
        return false;
    }
    if identifier(stream, true) || stream.match_regex(&ASCII_OPERATOR, true).is_some() {
        return true;
    }
    stream.set_pos(pos).unwrap();
    false
}

fn operator(stream: &mut StringStream<'_>) -> bool {
    static OPERATOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"^(?:[<>]:|[<>=]=|[!=]==|<<=?|>>>?=?|=>?|--?>|<--[->]?|//|[\\%*+\-<>!/^|&÷⊻]=?|[?$~:×∈∉∋∌∘√∛∩∪≠≤≥⊆⊈⊊⋅])"
    ).unwrap()
    });
    if stream.match_regex(&OPERATOR, true).is_some() {
        return true;
    }
    let tail = remaining(stream);
    if let Some(op) = keyword(tail, "in isa") {
        let after = &tail[op.len()..];
        // In the upstream RegExp constructor the escape before the dot is lost.
        if !after.starts_with('(')
            && !after.chars().next().is_some_and(|ch| {
                ch.len_utf16() == 1
                    && !js_line_terminator(ch)
                    && after[ch.len_utf8()..].starts_with('(')
            })
        {
            return eat_text(stream, op);
        }
    }
    false
}

fn wide(ch: char, all: bool) -> bool {
    // The upstream UTF-16 ranges include both surrogate code units.
    ch >= '\u{a1}' && (all || ch != '\u{2218}')
}

fn identifier(stream: &mut StringStream<'_>, all: bool) -> bool {
    if stream
        .eat_if(|ch| ch.is_ascii_alphabetic() || ch == '_' || wide(ch, all))
        .is_none()
    {
        return false;
    }
    stream.eat_while(|ch| word(ch) || wide(ch, all));
    stream.eat_while(|ch| ch == '!');
    true
}

fn string_prefix(stream: &mut StringStream<'_>) -> Option<&'static str> {
    if stream.eat('`').is_some() {
        return Some("`");
    }
    let tail = remaining(stream)
        .trim_start_matches(|ch: char| ch.is_ascii_alphabetic() || ch == '_' || wide(ch, true));
    let delimiter = if tail.starts_with("\"\"\"") {
        "\"\"\""
    } else if tail.starts_with('"') {
        "\""
    } else {
        return None;
    };
    stream
        .set_pos(stream.line().len() - tail.len() + delimiter.len())
        .unwrap();
    Some(delimiter)
}

fn number(stream: &mut StringStream<'_>) {
    static PREFIXED: [LazyLock<Regex>; 4] = [
        LazyLock::new(|| Regex::new(r"(?i)^0x\.[0-9a-f_]+p[+-]?[_0-9]+").unwrap()),
        LazyLock::new(|| Regex::new(r"(?i)^0x[0-9a-f_]+").unwrap()),
        LazyLock::new(|| Regex::new(r"(?i)^0b[01_]+").unwrap()),
        LazyLock::new(|| Regex::new(r"(?i)^0o[0-7_]+").unwrap()),
    ];
    static EXPONENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^[ef][+-]?[_0-9]+").unwrap());
    static DECIMAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^[0-9][_0-9]*(?:e[+-]?[0-9]+)?").unwrap());
    for pattern in &PREFIXED {
        stream.match_regex(pattern, true);
    }
    let start = stream.pos();
    if stream.eat_if(|ch| ch.is_ascii_digit()).is_some() {
        stream.eat_while(|ch| ch.is_ascii_digit() || ch == '_');
    }
    if stream.eat('.').is_some() && stream.peek() != Some('.') {
        if stream.eat_if(|ch| ch.is_ascii_digit()).is_some() {
            stream.eat_while(|ch| ch.is_ascii_digit() || ch == '_');
        }
    } else {
        stream.set_pos(start).unwrap();
    }
    stream.match_regex(&EXPONENT, true);
    stream.match_regex(&DECIMAL, true);
}

fn unicode_character(stream: &mut StringStream<'_>) -> bool {
    let tail = remaining(stream);
    let Some(hex) = tail
        .strip_prefix("\\u")
        .or_else(|| tail.strip_prefix("\\U"))
    else {
        return false;
    };
    let count = hex.bytes().take_while(u8::is_ascii_hexdigit).count();
    if count == 0
        || count > 8
        || hex.as_bytes().get(count) != Some(&b'\'')
        || count > 4 && !tail.starts_with("\\U")
    {
        return false;
    }
    let value = u32::from_str_radix(&hex[..count], 16).unwrap();
    let valid = if count <= 4 {
        !(0xd800..=0xdfff).contains(&value)
    } else {
        value <= 0x10ffff
    };
    stream
        .set_pos(stream.pos() + 2 + count + usize::from(valid))
        .unwrap();
    valid
}
