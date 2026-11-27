use super::{quote_continues, word};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    depth: usize,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Comment,
    Quote,
    Text,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.scan {
            Scan::Base => self.base(stream),
            Scan::Comment => Some(self.comment(stream)),
            Scan::Quote => Some(self.quote(stream)),
            Scan::Text => Some(self.text(stream)),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        self.depth
            .saturating_sub(usize::from(text_after.starts_with('}')))
            .saturating_mul(unit)
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        Some(match stream.next()? {
            '/' if stream.eat('*').is_some() => self.comment(stream),
            '#' => {
                stream.skip_to_end();
                "comment"
            }
            '"' => self.quote(stream),
            ch @ ('(' | '{' | ')' | '}') => {
                self.depth = match ch {
                    '(' => self.depth + 2,
                    '{' => self.depth + 1,
                    ')' => self.depth.saturating_sub(2),
                    _ => self.depth.saturating_sub(1),
                };
                return None;
            }
            ',' | ';' => return None,
            ch if ch.is_ascii_digit() => {
                stream.eat_while(|ch| ch.is_ascii_digit());
                stream.eat_if(|ch| "KkMmGg".contains(ch));
                "number"
            }
            ':' => {
                stream.eat_while(word);
                "operator"
            }
            _ => {
                stream.eat_while(word);
                match stream.current() {
                    "text" if stream.eat(':').is_some() => {
                        self.scan = Scan::Text;
                        "string"
                    }
                    "if" | "elsif" | "else" | "stop" | "require" => "keyword",
                    "true" | "false" | "not" => "atom",
                    _ => return None,
                }
            }
        })
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = Scan::Comment;
        if stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.scan = Scan::Base;
        } else {
            stream.skip_to_end();
        }
        "comment"
    }

    fn quote(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = if quote_continues(stream) {
            Scan::Quote
        } else {
            Scan::Base
        };
        "string"
    }

    fn text(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.sol() {
            if stream.next() == Some('.') && stream.eol() {
                self.scan = Scan::Base;
            }
            "string"
        } else {
            let style = if stream.peek() == Some('#') {
                "comment"
            } else {
                "string"
            };
            stream.skip_to_end();
            style
        }
    }
}
