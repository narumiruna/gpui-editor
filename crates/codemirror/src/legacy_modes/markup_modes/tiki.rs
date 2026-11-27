use super::eat_str;
use crate::{legacy_modes::js_space, stream_parser::StringStream};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Text,
    Block {
        style: &'static str,
        end: &'static str,
    },
    Plugin,
    Quoted(char),
    Unquoted,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.scan {
            Scan::Text => self.text(stream),
            Scan::Block { style, end } => Some(self.block(stream, style, end, true)),
            Scan::Plugin => self.plugin(stream),
            Scan::Quoted(quote) => Some(self.quoted(stream, quote)),
            Scan::Unquoted => Some(self.unquoted(stream)),
        }
    }

    fn text(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let sol = stream.sol();
        let ch = stream.next()?;
        let (style, end, multiline) = match ch {
            '{' => {
                stream.eat('/');
                stream.eat_space();
                stream.eat_while(|ch| {
                    !js_space(ch) && !matches!(ch, '=' | '"' | '\'' | '/' | '?' | '(' | '}')
                });
                self.scan = Scan::Plugin;
                return Some("tag");
            }
            '_' if stream.eat('_').is_some() => ("strong", "__", false),
            '\'' if stream.eat('\'').is_some() => ("em", "''", false),
            '(' if stream.eat('(').is_some() => ("link", "))", false),
            '[' => ("url", "]", false),
            '|' if stream.eat('|').is_some() => ("comment", "||", true),
            '-' if stream.eat('=').is_some() => ("header string", "=-", false),
            '-' if stream.eat('-').is_some() => ("error tw-deleted", "--", false),
            '=' if eat_str(stream, "==") => ("tw-underline", "===", false),
            ':' if stream.eat(':').is_some() => ("comment", "::", true),
            '^' => ("tw-box", "^", true),
            '~' if eat_str(stream, "np~") => ("meta", "~/np~", true),
            '!' if sol => {
                stream.skip_to_end();
                return Some("header string");
            }
            '*' | '#' | '+' if sol => {
                stream.skip_to_end();
                return Some("tw-listitem bracket");
            }
            _ => return None,
        };
        Some(self.block(stream, style, end, multiline))
    }

    fn block(
        &mut self,
        stream: &mut StringStream<'_>,
        style: &'static str,
        end: &'static str,
        multiline: bool,
    ) -> &'static str {
        if stream.skip_to(end) {
            eat_str(stream, end);
            self.scan = Scan::Text;
        } else {
            stream.skip_to_end();
            self.scan = if multiline {
                Scan::Block { style, end }
            } else {
                Scan::Text
            };
        }
        style
    }

    fn plugin(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match stream.next()? {
            '}' => {
                self.scan = Scan::Text;
                Some("tag")
            }
            '(' | ')' => Some("bracket"),
            '=' => {
                stream.eat('>');
                if !matches!(stream.peek(), Some('"' | '\'')) {
                    self.scan = Scan::Unquoted;
                }
                Some("operator")
            }
            quote @ ('"' | '\'') => Some(self.quoted(stream, quote)),
            _ => {
                stream.eat_while(|ch| !js_space(ch) && !matches!(ch, '=' | '"' | '\'' | '/' | '?'));
                Some("keyword")
            }
        }
    }

    fn quoted(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        let closed = stream.any(|ch| ch == quote);
        self.scan = if closed {
            Scan::Plugin
        } else {
            Scan::Quoted(quote)
        };
        "string"
    }

    fn unquoted(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        while let Some(ch) = stream.next() {
            if matches!(ch, ' ' | ',') || matches!(stream.peek(), Some(' ' | ')' | '}')) {
                self.scan = Scan::Plugin;
                break;
            }
        }
        "string"
    }
}
