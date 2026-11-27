use std::sync::LazyLock;

use regex::Regex;

use super::{Contexts, Kind, Resume, block_end, until_unescaped};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

// Match prefixes in upstream order. For example, `1.2` starts with a `1` token.
static NUMBERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:[+-]?(?:[0-9]+|[0-9]*\.[0-9]+)(?:[efd][+-]?[0-9]+)?|[+-]?[0-9]+(?:/[+-]?[0-9]+)?|#b[+-]?[01]+|#o[+-]?[0-7]+|#x[+-]?[0-9a-f]+)",
    ).unwrap()
});

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    contexts: Contexts,
    resume: Resume,
    after_open: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        self.contexts.start_line(stream);
        let (kind, style) = match self.resume {
            Resume::Base => self.base(stream),
            Resume::String => self.string(stream),
            Resume::Comment => self.comment(stream),
        };
        if kind != Kind::Space {
            let body = kind == Kind::Symbol && assume_body(stream.current());
            self.contexts.align(stream, true, body);
            self.after_open = kind == Kind::Open;
        }
        self.contexts.bracket(stream, kind);
        style
    }

    pub(super) fn indent(&self) -> usize {
        self.contexts.indent()
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        if stream.eat_space() {
            return (Kind::Space, None);
        }
        if stream
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || "+-.#".contains(ch))
            && stream.match_regex(&NUMBERS, true).is_some()
        {
            return (Kind::Other, Some("number"));
        }
        let mut ch = stream.next();
        if ch == Some('\\') {
            ch = stream.next();
        }
        match ch {
            Some('"') => self.string(stream),
            Some('(') => (Kind::Open, Some("bracket")),
            Some(')') => (Kind::Close, Some("bracket")),
            Some(';') => {
                stream.skip_to_end();
                (Kind::Space, Some("comment"))
            }
            Some('\'' | '`' | ',' | '@') => (Kind::Other, None),
            Some('|') => {
                if stream.skip_to("|") {
                    stream.next();
                    (Kind::Other, Some("variableName"))
                } else {
                    stream.skip_to_end();
                    (Kind::Other, Some("error"))
                }
            }
            Some('#') => self.dispatch(stream),
            _ => {
                let name = read_symbol(stream);
                let style = if name == "." {
                    return (Kind::Other, None);
                } else if matches!(name, "nil" | "t") || name.starts_with(':') {
                    "atom"
                } else if self.after_open && (is_special(name) || assume_body(name)) {
                    "keyword"
                } else if name.starts_with('&') {
                    "variableName.special"
                } else {
                    "variableName"
                };
                (Kind::Symbol, Some(style))
            }
        }
    }

    fn dispatch(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        match stream.next() {
            Some('(') => (Kind::Open, Some("bracket")),
            Some('+' | '-' | '=' | '.' | '\'') => (Kind::Other, None),
            Some('|') => self.comment(stream),
            Some(':') => {
                read_symbol(stream);
                (Kind::Other, Some("meta"))
            }
            Some('\\') => {
                stream.next();
                read_symbol(stream);
                (Kind::Other, Some("string.special"))
            }
            Some(ch) if ch.is_ascii_digit() => {
                let rest = &stream.line()[stream.pos()..];
                let count = rest.bytes().take_while(u8::is_ascii_digit).count();
                if rest.as_bytes().get(count) == Some(&b'#') {
                    stream.set_pos(stream.pos() + count + 1).unwrap();
                    (Kind::Other, None)
                } else {
                    (Kind::Other, Some("error"))
                }
            }
            _ => (Kind::Other, Some("error")),
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        self.resume = if until_unescaped(stream, '"') {
            Resume::Base
        } else {
            Resume::String
        };
        (Kind::Other, Some("string"))
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        self.resume = if block_end(stream) {
            Resume::Base
        } else {
            Resume::Comment
        };
        (Kind::Space, Some("comment"))
    }
}

fn read_symbol<'line>(stream: &mut StringStream<'line>) -> &'line str {
    while let Some(ch) = stream.eat_if(|ch| !js_space(ch) && !"'`,@()[]\";".contains(ch)) {
        if ch == '\\' {
            stream.next();
        }
    }
    stream.current()
}

fn assume_body(name: &str) -> bool {
    name == "cond"
        || ["with", "def", "do", "prog"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || ["case", "bind", "when", "unless"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn is_special(name: &str) -> bool {
    // `let*` upstream is a regex alternative: it matches `le` and `lett`, not `let*`.
    name.strip_prefix("le")
        .is_some_and(|rest| rest.bytes().all(|ch| ch == b't'))
        || matches!(
            name,
            "block"
                | "return-from"
                | "catch"
                | "load-time-value"
                | "setq"
                | "eval-when"
                | "locally"
                | "symbol-macrolet"
                | "flet"
                | "macrolet"
                | "tagbody"
                | "function"
                | "multiple-value-call"
                | "the"
                | "go"
                | "multiple-value-prog1"
                | "throw"
                | "if"
                | "progn"
                | "unwind-protect"
                | "labels"
                | "progv"
                | "quote"
        )
}
