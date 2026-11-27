use super::{StringStream, digit, word};
use crate::legacy_modes::js_line_terminator;

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Base,
    String(char),
    CharacterClass,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if matches!(self, Self::Base)
            && let Some(quote @ ('"' | '\'')) = stream.peek()
        {
            stream.next();
            *self = Self::String(quote);
        }
        Some(match *self {
            Self::String(quote) => {
                while let Some(ch) = stream.next() {
                    if ch == quote {
                        *self = Self::Base;
                        break;
                    }
                    if ch == '\\' {
                        stream.next();
                    }
                }
                "string"
            }
            Self::CharacterClass => {
                while !stream.eol() {
                    if stream.eat_while(|ch| !matches!(ch, ']' | '\\')) {
                        continue;
                    }
                    let mut remaining = stream.line()[stream.pos()..].chars();
                    if remaining.next() == Some('\\')
                        && remaining.next().is_some_and(|ch| !js_line_terminator(ch))
                    {
                        stream.next();
                        stream.next();
                        continue;
                    }
                    // Let the next call scan the closing bracket or unpaired escape.
                    *self = Self::Base;
                    break;
                }
                "operator"
            }
            Self::Base => self.base(stream)?,
        })
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.peek()?;
        Some(match ch {
            '[' => {
                stream.next();
                *self = Self::CharacterClass;
                "bracket"
            }
            '\\' => {
                stream.next();
                if stream.eat_while(|ch| ch.is_ascii_lowercase()) {
                    "string.special"
                } else {
                    "atom"
                }
            }
            ch if digit(ch) => {
                stream.eat_while(digit);
                if stream.eat_while(word) {
                    "error"
                } else {
                    "number"
                }
            }
            ch if ch.is_ascii_alphabetic() || ch == '_' => {
                stream.eat_while(word);
                if matches!(stream.peek(), Some('(' | '.')) {
                    "keyword"
                } else {
                    "variable"
                }
            }
            ch => {
                if !stream.eat_space() {
                    stream.next();
                }
                match ch {
                    ':' => "operator",
                    '.' | ',' | ';' | '*' | '-' | '+' | '^' | '<' | '/' | '=' => "atom",
                    '$' => "builtin",
                    ']' | '(' | ')' | '{' | '}' => "bracket",
                    _ => return None,
                }
            }
        })
    }
}
