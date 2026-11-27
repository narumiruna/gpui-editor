use super::{StringStream, close_quote, word};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    depth: usize,
    // Upstream keeps this at module scope. Keep it with the scan so that
    // copying a state and parsing a different document cannot change it.
    last: Kind,
}

#[derive(Clone, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Comment,
    String(char),
    Meta,
}

#[derive(Clone, Copy, Debug, Default)]
enum Kind {
    #[default]
    Unset,
    Declaration,
    Tag,
    OpenBracket,
    CloseBracket,
    CloseTag,
    Other,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        let style = match self.scan {
            Scan::Base => self.base(stream),
            Scan::Comment => self.comment(stream),
            Scan::String(quote) => self.string(stream, quote),
            Scan::Meta => {
                if stream.skip_to("?>") {
                    stream.match_str("?>", true, false);
                    self.scan = Scan::Base;
                } else {
                    stream.skip_to_end();
                }
                Some("meta")
            }
        };
        if stream.current() == "[" || matches!(self.last, Kind::Declaration | Kind::OpenBracket) {
            self.depth += 1;
        } else if stream.current() == "]"
            || matches!(self.last, Kind::CloseBracket | Kind::CloseTag)
        {
            self.depth = self.depth.saturating_sub(1);
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> usize {
        let depth = self.depth;
        let less = depth.saturating_sub(1);
        let next = if text_after.starts_with(']') {
            less
        } else if text_after.ends_with('>') {
            let next = if text_after.starts_with('<') {
                depth
            } else if text_after == ">" {
                match self.last {
                    Kind::Tag if depth == 0 => 1,
                    Kind::Declaration | Kind::Tag | Kind::CloseTag => less,
                    _ => depth,
                }
            } else if matches!(self.last, Kind::Declaration | Kind::Tag | Kind::CloseTag) {
                depth
            } else {
                less
            };
            if matches!(self.last, Kind::Unset | Kind::CloseBracket) {
                next.saturating_sub(1)
            } else {
                next
            }
        } else {
            depth
        };
        next.saturating_mul(indent_unit)
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let (style, kind) = match stream.next()? {
            '<' if stream.eat('!').is_some() => {
                if stream.eat_while(|ch| ch == '-') {
                    self.scan = Scan::Comment;
                    return self.comment(stream);
                }
                if !stream.eat_while(word) {
                    // Bare `<!` does not change the previous kind.
                    return None;
                }
                (Some("keyword"), Kind::Declaration)
            }
            '<' if stream.eat('?').is_some() => {
                self.scan = Scan::Meta;
                (Some("meta"), Kind::Other)
            }
            '#' if stream.eat_while(word) => (Some("atom"), Kind::Tag),
            '|' => (Some("keyword"), Kind::Other),
            '[' => (None, Kind::OpenBracket),
            ']' => (None, Kind::CloseBracket),
            '>' => (None, Kind::CloseTag),
            '(' | ')' | '-' | '.' | ',' | '+' | '?' => (None, Kind::Other),
            quote @ ('"' | '\'') => {
                self.scan = Scan::String(quote);
                return self.string(stream, quote);
            }
            _ if stream.eat_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '?' | '+')) => {
                // The punctuation at the end starts a separate token.
                if stream.current().ends_with(['?', '+']) {
                    stream.back_up(1).expect("one ASCII byte");
                }
                (Some("tag"), Kind::Tag)
            }
            '%' | '*' => (Some("number"), Kind::Other),
            _ => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '\\' | '-' | '%' | '.' | '{' | ','));
                (None, Kind::Unset)
            }
        };
        self.last = kind;
        style
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let mut dashes = 0;
        for ch in stream {
            if dashes >= 2 && ch == '>' {
                self.scan = Scan::Base;
                break;
            }
            dashes = if ch == '-' { dashes + 1 } else { 0 };
        }
        self.last = Kind::Other;
        Some("comment")
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> Option<&'static str> {
        if close_quote(stream, quote) {
            self.scan = Scan::Base;
        }
        self.last = Kind::Tag;
        Some("string")
    }
}
