use std::sync::LazyLock;

use regex::Regex;

use super::{advance, eat_str, remaining, word};
use crate::{legacy_modes::js_line_terminator, stream_parser::StringStream};

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Text,
    Code {
        block: bool,
    },
    Phrase {
        end: [char; 2],
        style: &'static str,
    },
    Macro,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match *self {
            Self::Text => self.text(stream),
            Self::Code { block } => self.code(stream, block),
            Self::Phrase { end, style } => self.phrase(stream, end, style),
            Self::Macro => self.macro_token(stream),
        }
    }

    fn text(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let sol = stream.sol();
        if sol {
            let style = match stream.line() {
                "{{{" => {
                    *self = Self::Code { block: true };
                    Some("comment")
                }
                "<<<" => Some("quote"),
                "/***" | "***/" | "//{{{" | "//}}}" | "<!--{{{-->" | "<!--}}}-->" => {
                    Some("comment")
                }
                line if line.starts_with("----") && line.bytes().all(|ch| ch == b'-') => {
                    Some("contentSeparator")
                }
                _ => None,
            };
            if style.is_some() {
                stream.skip_to_end();
                return style;
            }
        }

        let ch = stream.next()?;
        if sol {
            match ch {
                '!' => {
                    stream.skip_to_end();
                    return Some("header");
                }
                '*' | '#' | ';' | ':' | '>' => {
                    stream.eat_while(|next| next == ch);
                    return Some(if ch == '>' { "quote" } else { "comment" });
                }
                '|' => return Some("header"),
                _ => {}
            }
        }

        static URL: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?i-u:ttps?|tp|ile)://[-A-Za-z0-9+&@#/%?=~_|$!:,.;]*[A-Za-z0-9+&@#/%=~_|$]",
            )
            .unwrap()
        });
        match ch {
            '{' if eat_str(stream, "{{") => {
                *self = Self::Code { block: false };
                return self.code(stream, false);
            }
            'h' | 'H' | 'f' | 'F' if stream.match_regex(&URL, true).is_some() => {
                return Some("link");
            }
            '"' => return Some("string"),
            '~' => return Some("brace"),
            '[' | ']' if stream.eat(ch).is_some() => return Some("brace"),
            '@' => {
                stream.eat_while(|ch| word(ch) || ch == '-');
                return Some("link");
            }
            '0'..='9' => {
                stream.eat_while(|ch| ch.is_ascii_digit());
                return Some("number");
            }
            '/' if stream.eat('%').is_some() => {
                return self.open_phrase(stream, ['%', '/'], "comment");
            }
            '/' if stream.eat('/').is_some() => {
                return self.open_phrase(stream, ['/', '/'], "emphasis");
            }
            '_' if stream.eat('_').is_some() => {
                return self.open_phrase(stream, ['_', '_'], "link");
            }
            '-' if stream.eat('-').is_some() => {
                if stream.peek() == Some(' ') {
                    return Some("brace");
                }
                return self.open_phrase(stream, ['-', '-'], "deleted");
            }
            '\'' if stream.eat('\'').is_some() => {
                return self.open_phrase(stream, ['\'', '\''], "strong");
            }
            '<' if stream.eat('<').is_some() => {
                *self = Self::Macro;
                return Some("meta");
            }
            _ => {}
        }
        stream.eat_while(macro_word);
        None
    }

    fn open_phrase(
        &mut self,
        stream: &mut StringStream<'_>,
        end: [char; 2],
        style: &'static str,
    ) -> Option<&'static str> {
        *self = Self::Phrase { end, style };
        self.phrase(stream, end, style)
    }

    fn phrase(
        &mut self,
        stream: &mut StringStream<'_>,
        [first, last]: [char; 2],
        style: &'static str,
    ) -> Option<&'static str> {
        let mut maybe_end = false;
        for ch in stream {
            if ch == last && maybe_end {
                *self = Self::Text;
                break;
            }
            maybe_end = ch == first;
        }
        Some(style)
    }

    fn code(&mut self, stream: &mut StringStream<'_>, block: bool) -> Option<&'static str> {
        if block {
            if stream.sol() && stream.line() == "}}}" {
                stream.skip_to_end();
                *self = Self::Text;
            } else {
                stream.next();
            }
        } else if let Some(end) = remaining(stream)
            .split(js_line_terminator)
            .next()
            .and_then(|text| text.find("}}}"))
        {
            advance(stream, end + 3);
            *self = Self::Text;
        } else {
            stream.next();
        }
        Some("comment")
    }

    fn macro_token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let Some(ch) = stream.next() else {
            *self = Self::Text;
            return None;
        };
        if ch == '>' && stream.eat('>').is_some() {
            *self = Self::Text;
            return Some("meta");
        }
        stream.eat_while(macro_word);
        matches!(
            stream.current(),
            "allTags"
                | "closeAll"
                | "list"
                | "newJournal"
                | "newTiddler"
                | "permaview"
                | "saveChanges"
                | "search"
                | "slider"
                | "tabs"
                | "tag"
                | "tagging"
                | "tags"
                | "tiddler"
                | "timeline"
                | "today"
                | "version"
                | "option"
                | "with"
                | "filter"
        )
        .then_some("keyword")
    }
}

fn macro_word(ch: char) -> bool {
    word(ch) || ch == '$'
}
