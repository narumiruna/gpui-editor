use std::{borrow::Cow, sync::LazyLock};

use regex::Regex;

use super::{LegacySqlHookResult, Spec};
use crate::{legacy_modes::js_line_terminator, stream_parser::StringStream};

type Style = Option<&'static str>;

macro_rules! pattern {
    ($name:ident, $source:literal) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($source).unwrap());
    };
}

pattern!(HEX_PREFIX, r"^[xX][0-9a-fA-F]+");
pattern!(HEX_QUOTED, r"^'[0-9a-fA-F]*'");
pattern!(BINARY_PREFIX, r"^b[01]*");
pattern!(BINARY_QUOTED, r"^'[01]+'");
pattern!(DECIMAL, r"^[0-9]*(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?");
pattern!(ZEROLESS, r"^[0-9]+(?:[eE][+-]?[0-9]+)?");
pattern!(DATE, r#"^ +(?:'[^']*'|"[^"]*")"#);
pattern!(ODBC_DATE, r#"^ *(?:d|D|t|T|ts|TS) *(?:'[^']*'|"[^"]*") *}"#);
pattern!(COMMENT_MARKER, r"^[^\r\n\u2028\u2029]*?(?:/\*|\*/)");

fn matches(stream: &mut StringStream<'_>, pattern: &Regex) -> bool {
    stream.match_regex(pattern, true).is_some()
}

fn ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Scanner {
    #[default]
    Base,
    EscapeConstant,
    Literal {
        quote: char,
        backslash_escapes: bool,
    },
    Comment(usize),
}

impl Scanner {
    pub(super) fn is_base(self) -> bool {
        matches!(self, Self::Base)
    }

    pub(super) fn token(&mut self, spec: &Spec, stream: &mut StringStream<'_>) -> Style {
        match *self {
            Self::Base => self.base(spec, stream),
            Self::EscapeConstant => {
                let quote = stream.next()?;
                self.literal(stream, quote, true)
            }
            Self::Literal {
                quote,
                backslash_escapes,
            } => self.literal(stream, quote, backslash_escapes),
            Self::Comment(depth) => self.comment(stream, depth),
        }
    }

    fn base(&mut self, spec: &Spec, stream: &mut StringStream<'_>) -> Style {
        let ch = stream.next()?;
        if let Some((_, hook)) = spec.hooks.iter().find(|&&(key, _)| key == ch) {
            match hook(stream) {
                LegacySqlHookResult::Continue => {}
                LegacySqlHookResult::Unstyled => return None,
                LegacySqlHookResult::Style(style) => return Some(style),
            }
        }

        let support = spec.support;
        if support.hex_number
            && ((ch == '0' && matches(stream, &HEX_PREFIX))
                || (matches!(ch, 'x' | 'X') && matches(stream, &HEX_QUOTED)))
        {
            return Some("number");
        }
        if support.binary_number
            && ((matches!(ch, 'b' | 'B') && matches(stream, &BINARY_QUOTED))
                || (ch == '0' && matches(stream, &BINARY_PREFIX)))
        {
            return Some("number");
        }
        if ch.is_ascii_digit() {
            matches(stream, &DECIMAL);
            if support.decimalless_float && !stream.line()[stream.pos()..].starts_with("..") {
                stream.eat('.');
            }
            return Some("number");
        }
        if ch == '?' && (stream.eat_space() || stream.eol() || stream.eat(';').is_some()) {
            return Some("macroName");
        }
        if ch == '\'' || (ch == '"' && support.double_quote) {
            return self.literal(stream, ch, spec.backslash_string_escapes);
        }
        if ((support.n_char_cast && matches!(ch, 'n' | 'N'))
            || (support.charset_cast && ch == '_' && eat_charset(stream)))
            && matches!(stream.peek(), Some('\'' | '"'))
        {
            return Some("keyword");
        }
        if support.escape_constant
            && matches!(ch, 'e' | 'E')
            && (stream.peek() == Some('\'') || (support.double_quote && stream.peek() == Some('"')))
        {
            *self = Self::EscapeConstant;
            return Some("keyword");
        }
        if (support.comment_slash_slash && ch == '/' && stream.eat('/').is_some())
            || (support.comment_hash && ch == '#')
            || (ch == '-'
                && stream.eat('-').is_some()
                && (!support.comment_space_required || stream.eat(' ').is_some()))
        {
            stream.skip_to_end();
            return Some("comment");
        }
        if ch == '/' && stream.eat('*').is_some() {
            return self.comment(stream, 1);
        }
        if ch == '.' {
            if support.zeroless_float && matches(stream, &ZEROLESS) {
                return Some("number");
            }
            if stream.eat_while(|ch| ch == '.') {
                return None;
            }
            return (support.odbc_dot_table
                && stream.eat_while(|ch| ascii_word(ch) || matches!(ch, '$' | '#')))
            .then_some("type");
        }
        if spec.operator_chars.contains(ch) {
            stream.eat_while(|ch| spec.operator_chars.contains(ch));
            return Some("operator");
        }
        if spec.brackets.contains(ch) {
            return Some("bracket");
        }
        if spec.punctuation.contains(ch) {
            stream.eat_while(|ch| spec.punctuation.contains(ch));
            return Some("punctuation");
        }
        if ch == '{' && matches(stream, &ODBC_DATE) {
            return Some("number");
        }

        stream.eat_while(ascii_word);
        let current = stream.current();
        let word = if current
            .bytes()
            .all(|b| b.is_ascii() && !b.is_ascii_uppercase())
        {
            Cow::Borrowed(current)
        } else {
            Cow::Owned(current.to_lowercase())
        };
        if spec.date_sql.contains(word.as_ref()) && matches(stream, &DATE) {
            Some("number")
        } else if spec.atoms.contains(word.as_ref()) {
            Some("atom")
        } else if spec.builtin.contains(word.as_ref()) {
            Some("type")
        } else if spec.keywords.contains(word.as_ref()) {
            Some("keyword")
        } else if spec.client.contains(word.as_ref()) {
            Some("builtin")
        } else {
            None
        }
    }

    fn literal(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
        backslash_escapes: bool,
    ) -> Style {
        let mut escaped = false;
        for ch in stream {
            if ch == quote && !escaped {
                *self = Self::Base;
                return Some("string");
            }
            escaped = backslash_escapes && !escaped && ch == '\\';
        }
        *self = Self::Literal {
            quote,
            backslash_escapes,
        };
        Some("string")
    }

    fn comment(&mut self, stream: &mut StringStream<'_>, depth: usize) -> Style {
        *self = if let Some(marker) = stream.match_regex(&COMMENT_MARKER, true) {
            if marker.get(0).unwrap().as_str().ends_with("/*") {
                Self::Comment(depth + 1)
            } else if depth > 1 {
                Self::Comment(depth - 1)
            } else {
                Self::Base
            }
        } else {
            stream.skip_to_end();
            Self::Comment(depth)
        };
        Some("comment")
    }
}

fn eat_charset(stream: &mut StringStream<'_>) -> bool {
    if stream.eat_if(|ch| ch.is_ascii_alphabetic()).is_none() {
        return false;
    }
    stream.eat_while(|ch| ch.is_ascii_alphanumeric());
    true
}

pub(super) fn hook_backtick(stream: &mut StringStream<'_>) -> LegacySqlHookResult {
    identifier(stream, '`')
}

pub(super) fn hook_double_quote(stream: &mut StringStream<'_>) -> LegacySqlHookResult {
    identifier(stream, '"')
}

fn identifier(stream: &mut StringStream<'_>, quote: char) -> LegacySqlHookResult {
    let after_open = stream.pos();
    while let Some(ch) = stream.next() {
        if ch == quote && stream.eat(quote).is_none() {
            return LegacySqlHookResult::Style("string.special");
        }
    }
    stream.set_pos(after_open).unwrap();
    if stream.eat_while(ascii_word) {
        LegacySqlHookResult::Style("string.special")
    } else {
        LegacySqlHookResult::Unstyled
    }
}

pub(super) fn hook_var(stream: &mut StringStream<'_>) -> LegacySqlHookResult {
    if stream.eat('@').is_some() {
        for prefix in ["session.", "local.", "global."] {
            stream.match_str(prefix, true, false);
        }
    }
    if let Some(quote) = stream.eat_if(|ch| matches!(ch, '\'' | '"' | '`')) {
        let start = stream.pos();
        let rest = &stream.line()[start..];
        let end = rest.find(js_line_terminator).unwrap_or(rest.len());
        if let Some(close) = rest[..end].rfind(quote) {
            stream.set_pos(start + close + 1).unwrap();
        }
        LegacySqlHookResult::Style("string.special")
    } else if stream.eat_while(|ch| ascii_word(ch) || matches!(ch, '$' | '.')) {
        LegacySqlHookResult::Style("string.special")
    } else {
        LegacySqlHookResult::Unstyled
    }
}

pub(super) fn hook_client(stream: &mut StringStream<'_>) -> LegacySqlHookResult {
    if stream.eat('N').is_some() {
        LegacySqlHookResult::Style("atom")
    } else if stream
        .eat_if(|ch| ch.is_ascii_alphabetic() || matches!(ch, '.' | '#' | '!' | '?'))
        .is_some()
    {
        LegacySqlHookResult::Style("string.special")
    } else {
        LegacySqlHookResult::Unstyled
    }
}
