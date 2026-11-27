use std::{collections::HashSet, sync::LazyLock};

use super::{StringStream, word};

static KEYWORDS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    include_str!("nginx_keywords.txt")
        .split_ascii_whitespace()
        .collect()
});

const CONTROL: &[&str] = &[
    "http",
    "mail",
    "events",
    "server",
    "types",
    "location",
    "upstream",
    "charset_map",
    "limit_except",
    "if",
    "geo",
    "map",
    "include",
    "root",
    "server_name",
    "listen",
    "internal",
    "proxy_pass",
    "memcached_pass",
    "fastcgi_pass",
    "try_files",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Context {
    Block,
    Rule,
    Media,
    MediaBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Other,
    Open,
    Close,
    Semicolon,
    Media,
    Comment,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scanner {
    #[default]
    Base,
    CComment,
    SgmlComment,
    String(char),
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scanner: Scanner,
    stack: Vec<Context>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        let (style, kind) = match self.scanner {
            Scanner::Base => self.base(stream),
            Scanner::CComment => self.c_comment(stream),
            Scanner::SgmlComment => self.sgml_comment(stream),
            Scanner::String(quote) => self.string(stream, quote),
        };
        let context = self.stack.last().copied();
        let style = match (style, context) {
            (Some("variable"), Some(Context::Rule)) => Some("number"),
            (Some("variable"), None | Some(Context::MediaBlock)) => Some("tag"),
            _ => style,
        };
        if context == Some(Context::Rule)
            && matches!(kind, Kind::Open | Kind::Close | Kind::Semicolon)
        {
            self.stack.pop();
        }
        match kind {
            Kind::Open if context == Some(Context::Media) => {
                *self.stack.last_mut().unwrap() = Context::MediaBlock;
            }
            Kind::Open => self.stack.push(Context::Block),
            Kind::Close => {
                self.stack.pop();
            }
            Kind::Media => self.stack.push(Context::Media),
            Kind::Comment => {}
            _ if context == Some(Context::Block) => self.stack.push(Context::Rule),
            _ => {}
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let levels = if text_after.starts_with('}') {
            let close = if self.stack.last() == Some(&Context::Rule) {
                2
            } else {
                1
            };
            // The Rust indentation API cannot represent a negative column.
            self.stack.len().saturating_sub(close)
        } else {
            self.stack.len()
        };
        levels.saturating_mul(unit)
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (Option<&'static str>, Kind) {
        // Upstream scans the word prefix before reading the next character.
        // For an unknown word, that character belongs to the same token.
        stream.eat_while(|ch| word(ch) || ch == '$');
        let current = stream.current();
        if KEYWORDS.contains(current) {
            return (Some("keyword"), Kind::Other);
        }
        if CONTROL.contains(&current) {
            return (Some("controlKeyword"), Kind::Other);
        }
        match stream.next() {
            Some('@') => {
                stream.eat_while(word_tail);
                let kind = if stream.current() == "@media" {
                    Kind::Media
                } else {
                    Kind::Other
                };
                (Some("meta"), kind)
            }
            Some('/') if stream.eat('*').is_some() => self.c_comment(stream),
            Some('<') if stream.eat('!').is_some() => self.sgml_comment(stream),
            Some('~' | '|') if stream.eat('=').is_some() => (None, Kind::Other),
            Some(quote @ ('\'' | '"')) => self.string(stream, quote),
            Some('#') => {
                stream.skip_to_end();
                (Some("comment"), Kind::Comment)
            }
            Some('!') => {
                stream.eat_space();
                stream.eat_while(word);
                (Some("keyword"), Kind::Other)
            }
            Some(ch) if ch.is_ascii_digit() => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '.' | '%'));
                (Some("number"), Kind::Other)
            }
            Some('=' | ',' | '.' | '+' | '>' | '*' | '/' | ':' | '[' | ']') => (None, Kind::Other),
            Some(';') => (None, Kind::Semicolon),
            Some('{') => (None, Kind::Open),
            Some('}') => (None, Kind::Close),
            _ => {
                stream.eat_while(word_tail);
                (Some("variable"), Kind::Other)
            }
        }
    }

    fn c_comment(&mut self, stream: &mut StringStream<'_>) -> (Option<&'static str>, Kind) {
        self.scanner = Scanner::CComment;
        let mut star = false;
        for ch in stream {
            if star && ch == '/' {
                self.scanner = Scanner::Base;
                break;
            }
            star = ch == '*';
        }
        (Some("comment"), Kind::Comment)
    }

    fn sgml_comment(&mut self, stream: &mut StringStream<'_>) -> (Option<&'static str>, Kind) {
        self.scanner = Scanner::SgmlComment;
        let mut dashes = 0;
        for ch in stream {
            if dashes >= 2 && ch == '>' {
                self.scanner = Scanner::Base;
                break;
            }
            dashes = if ch == '-' { dashes + 1 } else { 0 };
        }
        (Some("comment"), Kind::Comment)
    }

    fn string(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
    ) -> (Option<&'static str>, Kind) {
        let mut escaped = false;
        for ch in stream {
            if ch == quote && !escaped {
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        self.scanner = if escaped {
            Scanner::String(quote)
        } else {
            Scanner::Base
        };
        (Some("string"), Kind::Other)
    }
}

fn word_tail(ch: char) -> bool {
    word(ch) || matches!(ch, '\\' | '-')
}
