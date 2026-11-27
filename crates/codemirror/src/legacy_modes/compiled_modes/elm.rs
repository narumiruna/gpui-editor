use super::{in_words, take, word};
use crate::stream_parser::StringStream;

const KEYWORDS: &str =
    "case of as if then else let in type alias module where import exposing port";
const SYMBOLS: &str = "-&*+.\\/<>=?^|:";

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Normal,
    Comment(usize),
    MultiString,
    Glsl,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = match *self {
            Self::Normal => self.normal(stream),
            Self::Comment(depth) => Some(self.comment(stream, depth)),
            Self::MultiString => Some(self.until(stream, "\"\"\"")),
            Self::Glsl => Some(self.until(stream, "|]")),
        };
        if in_words(KEYWORDS, stream.current()) {
            Some("keyword")
        } else {
            style
        }
    }

    fn normal(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_while(|ch| matches!(ch, ' ' | '\x0b' | '\x0c')) {
            return None;
        }
        Some(match stream.next()? {
            '{' if stream.eat('-').is_some() => self.comment(stream, 1),
            '[' if take(stream, "glsl|") => {
                *self = Self::Glsl;
                self.until(stream, "|]")
            }
            '(' | ')' | ',' | '[' | ']' | '{' | '}' => "builtin",
            '\'' => quoted(stream, "\\'", "'"),
            '"' if stream.eat('"').is_some() => {
                if stream.eat('"').is_some() {
                    *self = Self::MultiString;
                    self.until(stream, "\"\"\"")
                } else {
                    "string"
                }
            }
            '"' => quoted(stream, "\\\"", "\""),
            'A'..='Z' => {
                stream.eat_while(word);
                "type"
            }
            'a'..='z' => {
                stream.eat_while(word);
                if stream.start() == 0 {
                    "def"
                } else {
                    "variable"
                }
            }
            ch @ '0'..='9' => number(stream, ch),
            '-' if stream.eat('-').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            ch if SYMBOLS.contains(ch) => {
                stream.eat_while(|ch| SYMBOLS.contains(ch));
                "keyword"
            }
            '_' => "keyword",
            _ => "error",
        })
    }

    fn comment(&mut self, stream: &mut StringStream<'_>, mut depth: usize) -> &'static str {
        while let Some(ch) = stream.next() {
            if ch == '{' && stream.eat('-').is_some() {
                depth += 1;
            } else if ch == '-' && stream.eat('}').is_some() {
                depth -= 1;
                if depth == 0 {
                    *self = Self::Normal;
                    return "comment";
                }
            }
        }
        *self = Self::Comment(depth);
        "comment"
    }

    fn until(&mut self, stream: &mut StringStream<'_>, end: &str) -> &'static str {
        if stream.skip_to(end) {
            take(stream, end);
            *self = Self::Normal;
        } else {
            stream.skip_to_end();
        }
        "string"
    }
}

fn quoted(stream: &mut StringStream<'_>, escape: &str, end: &str) -> &'static str {
    // The upstream scanner searches for every escaped delimiter before it
    // searches for a closing delimiter, even across an earlier closing quote.
    while stream.skip_to(escape) {
        take(stream, escape);
    }
    if stream.skip_to(end) {
        take(stream, end);
        "string"
    } else {
        stream.skip_to_end();
        "error"
    }
}

fn number(stream: &mut StringStream<'_>, first: char) -> &'static str {
    if first == '0' {
        if stream.eat_if(|ch| matches!(ch, 'x' | 'X')).is_some() {
            stream.eat_while(|ch| ch.is_ascii_hexdigit());
            return "number";
        }
    } else {
        stream.eat_while(|ch| ch.is_ascii_digit());
    }
    if stream.eat('.').is_some() {
        stream.eat_while(|ch| ch.is_ascii_digit());
    }
    if stream.eat_if(|ch| matches!(ch, 'e' | 'E')).is_some() {
        stream.eat_if(|ch| matches!(ch, '-' | '+'));
        stream.eat_while(|ch| ch.is_ascii_digit());
    }
    "number"
}
