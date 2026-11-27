use super::{super::js_space, word};
use crate::stream_parser::StringStream;

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Base,
    Quoted(char),
    Comment(&'static str),
    CharacterClass,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if matches!(self, Self::Base) {
            if let Some(quote) = stream.eat_if(|ch| matches!(ch, '\'' | '"')) {
                *self = Self::Quoted(quote);
            } else if stream.match_str("/*", true, false) {
                *self = Self::Comment("*/");
            } else if stream.match_str("(*", true, false) {
                *self = Self::Comment("*)");
            }
        }
        match *self {
            Self::Quoted(quote) => {
                while let Some(ch) = stream.next() {
                    if ch == quote {
                        *self = Self::Base;
                        break;
                    }
                    if ch == '\\' {
                        stream.next();
                    }
                }
                // The upstream lhs flag is never cleared.
                return Some("property");
            }
            Self::Comment(close) => {
                if stream.skip_to(close) {
                    stream.match_str(close, true, false);
                    *self = Self::Base;
                } else {
                    stream.skip_to_end();
                }
                return Some("comment");
            }
            Self::CharacterClass => {
                stream.eat_while(|ch| !matches!(ch, ']' | '\\'));
                if !stream.eol() {
                    // The delimiter belongs to the base scan. It can take a retry.
                    *self = Self::Base;
                }
                return Some("operator");
            }
            Self::Base => {}
        }

        let style = match stream.peek()? {
            '[' => {
                *self = Self::CharacterClass;
                "bracket"
            }
            ']' | '(' | ')' => "bracket",
            ':' | '|' | ';' => "operator",
            '.' | '*' | '-' | '+' | '^' => "atom",
            '%' => {
                if stream.match_str("%%", true, false) {
                    return Some("header");
                }
                if stream.match_str("%}", true, false) {
                    return Some("bracket");
                }
                return directive(stream);
            }
            // The upstream switch falls through to the atom branch for
            // backslashes and slashes without a directive, including `//`.
            '/' => return directive(stream).or(Some("atom")),
            '\\' => "atom",
            '$' => {
                if stream.match_str("$$", true, false) {
                    return Some("builtin");
                }
                stream.next();
                return stream
                    .eat_while(|ch| ch.is_ascii_digit())
                    .then_some("variableName.special");
            }
            '<' => {
                let rest = &stream.line()[stream.pos()..];
                if let Some(name) = rest.strip_prefix("<<") {
                    let end = name
                        .find(|ch: char| !ch.is_ascii_alphabetic() && ch != '_')
                        .unwrap_or(name.len());
                    if end != 0 && name[end..].starts_with(">>") {
                        stream.set_pos(stream.pos() + end + 4).unwrap();
                        return Some("builtin");
                    }
                }
                stream.next();
                return None;
            }
            ch if ch.is_ascii_alphabetic() || ch == '_' => {
                if stream.match_str("return", true, false) {
                    return Some("operator");
                }
                stream.eat_while(word);
                let after = &stream.line()[stream.pos()..];
                return Some(if after.starts_with(['(', '.']) {
                    "variable"
                } else if after.trim_start_matches(js_space).starts_with([':', '=']) {
                    "def"
                } else {
                    "variableName.special"
                });
            }
            _ => {
                if !stream.eat_space() {
                    stream.next();
                }
                return None;
            }
        };
        stream.next();
        Some(style)
    }
}

fn directive(stream: &mut StringStream<'_>) -> Option<&'static str> {
    stream.next();
    stream
        .eat_while(|ch| ch.is_ascii_alphabetic())
        .then_some("keyword")
}
