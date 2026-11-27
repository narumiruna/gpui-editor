use std::{collections::HashMap, sync::LazyLock};

use regex::Regex;

use super::{remaining, word};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    depth: isize,
    after_do: bool,
    dot_string: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.after_do = false;
        }
        let style = self.scan(stream);
        match stream.current() {
            "." => {
                // A dot takes the style of a following identifier, or an error.
                return if self.scan(stream) == Some("variable") {
                    Some("variable")
                } else {
                    Some("error")
                };
            }
            "[" | "(" | "{" => self.depth = self.depth.saturating_add(1),
            "]" | ")" | "}" => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let text = text_after.trim_matches(js_space);
        let depth = match keyword(leading_word(text)) {
            Some(Keyword::Close | Keyword::End | Keyword::Middle) => self.depth.saturating_sub(1),
            _ => self.depth,
        };
        usize::try_from(depth).unwrap_or(0).saturating_mul(unit)
    }

    fn scan(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if self.dot_string {
            stream.skip_to_end();
            return Some("string");
        }
        if stream.eat_space() {
            return None;
        }
        let ch = stream.peek()?;
        if ch == '\'' {
            stream.skip_to_end();
            return Some("comment");
        }
        if matches!(ch, '0'..='9' | '.' | '&') && number(stream) {
            return Some("number");
        }
        if stream.eat('"').is_some() {
            // A string scanned after a dot has the delimiter .". The upstream
            // string scanner reaches each quote but cannot match that delimiter.
            self.dot_string = stream.current() == ".\"";
            if self.dot_string || !stream.skip_to("\"") {
                stream.skip_to_end();
            } else {
                stream.eat('"');
            }
            return Some("string");
        }
        if eat_any(
            stream,
            &[
                "//=", ">>=", "<<=", "**=", "+=", "-=", "*=", "%=", "/=", "&=", "|=", "^=",
            ],
        ) {
            return None;
        }
        if eat_any(stream, &["==", "<>", "<=", ">=", "<<", ">>", "//", "**"])
            || stream.eat_if(|ch| "+-*/%&\\|^~<>!".contains(ch)).is_some()
        {
            return Some("operator");
        }
        if stream.eat_if(|ch| "()[]{}@,:`=;.".contains(ch)).is_some() {
            return None;
        }
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '#' {
            let rest = remaining(stream);
            let len = if let Some(after) = rest.strip_prefix('#') {
                1 + leading_word(after).len()
            } else {
                leading_word(rest).len()
            };
            let kind = keyword(&rest[..len]);
            if kind.is_some() || ch != '#' {
                stream
                    .set_pos(stream.pos() + len)
                    .expect("ASCII word ends at a scalar boundary");
                return Some(kind.map_or("variable", |kind| self.apply_keyword(kind)));
            }
        }
        stream.next();
        Some("error")
    }

    fn apply_keyword(&mut self, kind: Keyword) -> &'static str {
        match kind {
            Keyword::Operator => return "operator",
            Keyword::Do => {
                self.depth = self.depth.saturating_add(1);
                self.after_do = true;
            }
            Keyword::Open => {
                if !std::mem::take(&mut self.after_do) {
                    self.depth = self.depth.saturating_add(1);
                }
            }
            Keyword::End => self.depth = self.depth.saturating_sub(2),
            Keyword::Close => self.depth = self.depth.saturating_sub(1),
            Keyword::Middle | Keyword::Other => {}
        }
        "keyword"
    }
}

fn eat_any(stream: &mut StringStream<'_>, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| stream.match_str(prefix, true, false))
}

fn number(stream: &mut StringStream<'_>) -> bool {
    static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:[0-9]*\.[0-9]+[Ff]?|[0-9]+\.[0-9]*F?)").expect("valid float pattern")
    });
    static INTEGER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:&[Hh][0-9a-fA-F]+|&[Oo][0-7]+|[1-9][0-9]*F?[Jj]?)")
            .expect("valid integer pattern")
    });
    if stream.match_regex(&FLOAT, true).is_some() {
        stream.eat_if(|ch| matches!(ch, 'J' | 'j'));
        true
    } else if stream.match_regex(&INTEGER, true).is_some() || eat_zero(stream) {
        stream.eat_if(|ch| matches!(ch, 'L' | 'l'));
        true
    } else {
        false
    }
}

fn eat_zero(stream: &mut StringStream<'_>) -> bool {
    let rest = remaining(stream).as_bytes();
    if rest.first() == Some(&b'0') && !matches!(rest.get(1), Some(b'0'..=b'9' | b'x' | b'X')) {
        stream.next();
        true
    } else {
        false
    }
}

fn leading_word(text: &str) -> &str {
    &text[..text.find(|ch| !word(ch)).unwrap_or(text.len())]
}

#[derive(Clone, Copy)]
enum Keyword {
    Do,
    Open,
    Middle,
    End,
    Close,
    Operator,
    Other,
}

fn keyword(text: &str) -> Option<Keyword> {
    static WORDS: LazyLock<HashMap<&str, Keyword>> = LazyLock::new(|| {
        TABLES
            .iter()
            .flat_map(|&(words, kind)| words.iter().map(move |&word| (word, kind)))
            .collect()
    });
    match text.to_ascii_lowercase().as_str() {
        "do" => Some(Keyword::Do),
        "end" => Some(Keyword::End),
        word => WORDS.get(word).copied(),
    }
}

pub(super) fn autocomplete() -> &'static [&'static str] {
    static WORDS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        TABLES
            .iter()
            .flat_map(|&(words, _)| words.iter().copied())
            .collect()
    });
    WORDS.as_slice()
}

const TABLES: &[(&[&str], Keyword)] = &[
    (OPENING, Keyword::Open),
    (MIDDLE, Keyword::Middle),
    (CLOSING, Keyword::Close),
    (OPERATORS, Keyword::Operator),
    (COMMON, Keyword::Other),
    (TYPES, Keyword::Other),
];

const OPENING: &[&str] = &[
    "class",
    "module",
    "sub",
    "enum",
    "select",
    "while",
    "if",
    "function",
    "get",
    "set",
    "property",
    "try",
    "structure",
    "synclock",
    "using",
    "with",
];
const MIDDLE: &[&str] = &["else", "elseif", "case", "catch", "finally"];
const CLOSING: &[&str] = &["next", "loop"];
const OPERATORS: &[&str] = &[
    "and", "andalso", "or", "orelse", "xor", "in", "not", "is", "isnot", "like",
];
const COMMON: &[&str] = &[
    "#const",
    "#else",
    "#elseif",
    "#end",
    "#if",
    "#region",
    "addhandler",
    "addressof",
    "alias",
    "as",
    "byref",
    "byval",
    "cbool",
    "cbyte",
    "cchar",
    "cdate",
    "cdbl",
    "cdec",
    "cint",
    "clng",
    "cobj",
    "compare",
    "const",
    "continue",
    "csbyte",
    "cshort",
    "csng",
    "cstr",
    "cuint",
    "culng",
    "cushort",
    "declare",
    "default",
    "delegate",
    "dim",
    "directcast",
    "each",
    "erase",
    "error",
    "event",
    "exit",
    "explicit",
    "false",
    "for",
    "friend",
    "gettype",
    "goto",
    "handles",
    "implements",
    "imports",
    "infer",
    "inherits",
    "interface",
    "isfalse",
    "istrue",
    "lib",
    "me",
    "mod",
    "mustinherit",
    "mustoverride",
    "my",
    "mybase",
    "myclass",
    "namespace",
    "narrowing",
    "new",
    "nothing",
    "notinheritable",
    "notoverridable",
    "of",
    "off",
    "on",
    "operator",
    "option",
    "optional",
    "out",
    "overloads",
    "overridable",
    "overrides",
    "paramarray",
    "partial",
    "private",
    "protected",
    "public",
    "raiseevent",
    "readonly",
    "redim",
    "removehandler",
    "resume",
    "return",
    "shadows",
    "shared",
    "static",
    "step",
    "stop",
    "strict",
    "then",
    "throw",
    "to",
    "true",
    "trycast",
    "typeof",
    "until",
    "until",
    "when",
    "widening",
    "withevents",
    "writeonly",
];
const TYPES: &[&str] = &[
    "object", "boolean", "char", "string", "byte", "sbyte", "short", "ushort", "int16", "uint16",
    "integer", "uinteger", "int32", "uint32", "long", "ulong", "int64", "uint64", "decimal",
    "single", "double", "float", "date", "datetime", "intptr", "uintptr",
];
