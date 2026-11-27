use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;

use super::{Contexts, Kind, Resume, SPACE, clojure_words as words, match_token, until_unescaped};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

static NUMBERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r##"^([+-]?[0-9]+(?:N|[eE][+-]?[0-9]+|\.?[0-9]*(?:M|[eE][+-]?[0-9]+)?|/[0-9]+|[xX][0-9a-fA-F]+|r[0-9a-zA-Z]+)?)(?:[\\\[\]{SPACE}"#'(),;@^`{{}}~]|$)"##
    )).unwrap()
});
static CHARACTERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"^(\\(?:backspace|formfeed|newline|return|space|tab|o[0-7]{{3}}|[ux][0-9A-Fa-f]{{4}}|[^\r\n\u{{2028}}\u{{2029}}\u{{10000}}-\u{{10FFFF}}])?)(?:[\\\[\]{SPACE}"(),;@^`{{}}~]|$)"#
    )).unwrap()
});
static SYMBOLS: LazyLock<Regex> = LazyLock::new(|| {
    let delimiter = format!(r#"\\\[\]{SPACE}"(),;@^`{{}}~"#);
    let first = format!(r##"{delimiter}/0-9#'"##);
    let namespace = format!(r"[^{first}.][^{delimiter}./]*");
    Regex::new(&format!(
        r"^((?:{namespace}(?:\.{namespace})*/)?(?:/|[^{first}][^{delimiter}]*)*)(?:[{delimiter}]|$)"
    ))
    .unwrap()
});

static AUTOCOMPLETE: LazyLock<Vec<&str>> =
    LazyLock::new(|| words::COMPLETIONS.split_ascii_whitespace().collect());
static CORE: LazyLock<HashSet<&str>> = LazyLock::new(|| AUTOCOMPLETE.iter().copied().collect());

pub(super) fn autocomplete() -> &'static [&'static str] {
    &AUTOCOMPLETE
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    contexts: Contexts,
    resume: Resume,
    after_paren: bool,
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
            let body = kind == Kind::Symbol && words::has_body(stream.current());
            self.contexts.align(stream, self.after_paren, body);
            self.after_paren = stream.current() == "(";
        }
        self.contexts.bracket(stream, kind);
        style
    }

    pub(super) fn indent(&self) -> usize {
        self.contexts.indent()
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        if stream.eat_space() || stream.eat(',').is_some() {
            return (Kind::Space, None);
        }
        if stream
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '-'))
            && match_token(stream, &NUMBERS)
        {
            return (Kind::Other, Some("number"));
        }
        match stream.peek() {
            Some('\\') if match_token(stream, &CHARACTERS) => {
                return (Kind::Other, Some("string.special"));
            }
            Some('"') => {
                stream.next();
                return self.string(stream);
            }
            Some('(' | '[' | '{') => {
                stream.next();
                return (Kind::Open, Some("bracket"));
            }
            Some(')' | ']' | '}') => {
                stream.next();
                return (Kind::Close, Some("bracket"));
            }
            Some(';') => {
                stream.skip_to_end();
                return (Kind::Space, Some("comment"));
            }
            Some('#' | '\'' | '@' | '^' | '`' | '~') => {
                stream.next();
                return (Kind::Other, Some("meta"));
            }
            _ => {}
        }
        if !match_token(stream, &SYMBOLS) {
            stream.next();
            stream.eat_while(|ch| !js_space(ch) && !"\\[]\"(),;@^`{}~".contains(ch));
            return (Kind::Other, Some("error"));
        }
        let symbol = stream.current();
        let style = if symbol == "comment" && self.after_paren {
            return self.comment(stream);
        } else if matches!(symbol, "false" | "nil" | "true") || symbol.starts_with(':') {
            "atom"
        } else if CORE.contains(symbol) {
            "keyword"
        } else if self.after_paren {
            "builtin"
        } else {
            "variable"
        };
        (Kind::Symbol, Some(style))
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
        // Upstream starts at depth one for each continuation line.
        self.resume = Resume::Comment;
        let mut depth = 1;
        while let Some(ch) = stream.next() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        stream.back_up(1).unwrap();
                        self.resume = Resume::Base;
                        break;
                    }
                }
                _ => {}
            }
        }
        (Kind::Space, Some("comment"))
    }
}
