use super::{js_state, prefix, web_modes};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    quote: Option<char>,
    comment: bool,
    class: bool,
    braces: isize,
    script: Option<web_modes::State>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if self.quote.is_none() && !self.comment {
            self.quote = stream.eat_if(|ch| matches!(ch, '"' | '\''));
            if self.quote.is_none() && stream.match_str("/*", true, false) {
                self.comment = true;
            }
        }

        if let Some(quote) = self.quote {
            while let Some(ch) = stream.next() {
                if ch == quote {
                    self.quote = None;
                    break;
                }
                if ch == '\\' {
                    stream.next();
                }
            }
            // Upstream leaves `lhs` true throughout, including on the right-hand side.
            return Some("property string");
        }
        if self.comment {
            while !stream.eol() {
                if stream.match_str("*/", true, false) {
                    self.comment = false;
                    break;
                }
                stream.next();
            }
            return Some("comment");
        }
        if self.class {
            while !stream.eol() {
                if !stream.eat_while(|ch| ch != ']' && ch != '\\') && !prefix(stream, re!(r"^\\."))
                {
                    self.class = false;
                    break;
                }
            }
            // This may only change state. The next call reads the closing bracket.
            return None;
        }
        if stream.eat('[').is_some() {
            self.class = true;
            return Some("bracket");
        }
        if stream.match_str("//", true, false) {
            stream.skip_to_end();
            return Some("comment");
        }
        if self.braces != 0 || stream.peek() == Some('{') {
            let script = self.script.get_or_insert_with(|| js_state(0));
            let style = script.token(stream);
            if style.is_none() {
                for ch in stream.current().chars() {
                    self.braces += match ch {
                        '{' => 1,
                        '}' => -1,
                        _ => 0,
                    };
                }
            }
            return style;
        }
        if prefix(stream, re!(r"^[a-zA-Z_][a-zA-Z0-9_]*")) {
            return Some(if stream.peek() == Some(':') {
                "variable"
            } else {
                "variable-2"
            });
        }
        if stream.eat_if(|ch| matches!(ch, ']' | '(' | ')')).is_some() {
            return Some("bracket");
        }
        if !stream.eat_space() {
            stream.next();
        }
        None
    }
}
