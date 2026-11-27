use std::sync::LazyLock;

use regex::Regex;

use crate::stream_parser::StringStream;

static DATETIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{4}[0-9:.T-]*Z").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-?[0-9]+(?:\.[0-9]+)?").unwrap());

#[derive(Clone, Debug)]
pub(super) struct State {
    quote: Option<&'static str>,
    lhs: bool,
    array_depth: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            quote: None,
            lhs: true,
            array_depth: 0,
        }
    }
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if self.quote.is_none() {
            self.quote = ["'''", "\"\"\"", "'", "\""]
                .into_iter()
                .find(|quote| stream.match_str(quote, true, false));
        }
        if stream.sol() && self.quote.is_none() && self.array_depth == 0 {
            self.lhs = true;
        }
        if let Some(quote) = self.quote {
            while !stream.eol() {
                if stream.match_str(quote, true, false) {
                    self.quote = None;
                    break;
                }
                if stream.next() == Some('\\') {
                    stream.next();
                }
            }
            return Some(if self.lhs { "property" } else { "string" });
        }
        if self.array_depth != 0 && stream.eat(']').is_some() {
            self.array_depth -= 1;
            return Some("bracket");
        }
        if self.lhs && stream.peek() == Some('[') && stream.skip_to("]") {
            stream.next();
            stream.eat(']');
            return Some("atom");
        }
        if stream.peek() == Some('#') {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.eat_space() {
            return None;
        }
        if self.lhs {
            if stream.eat_while(|ch| ch != '=' && ch != ' ') {
                return Some("property");
            }
            stream.eat('=');
            self.lhs = false;
        } else if stream.match_regex(&DATETIME, true).is_some()
            || stream.match_str("true", true, false)
            || stream.match_str("false", true, false)
        {
            return Some("atom");
        } else if stream.eat('[').is_some() {
            self.array_depth += 1;
            return Some("bracket");
        } else if stream.match_regex(&NUMBER, true).is_some() {
            return Some("number");
        } else {
            stream.next();
        }
        None
    }
}
