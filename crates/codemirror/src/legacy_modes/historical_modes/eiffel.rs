use std::sync::LazyLock;

use regex::Regex;

use super::{QuoteEnd, StringStream, quoted, word};

const KEYWORDS: &str = "note across when variant until unique undefine then strip select retry rescue require rename reference redefine prefix once old obsolete loop local like is inspect infix include if frozen from external export ensure end elseif else do creation create check alias agent separate invariant inherit indexing feature expanded deferred class Void True Result Precursor False Current attached detachable as and implies not or";
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^0(?:[bB][01]+|[cC][0-7]+|[xX][0-9a-fA-F]+)$|^[0-9]+\.|\.[0-9]+$|^[0-9]+$")
        .unwrap()
});

#[derive(Clone, Debug, Default)]
pub(super) struct State(Option<char>);

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if let Some(quote) = self.0 {
            return Some(self.string(stream, quote));
        }
        if stream.eat_space() {
            return None;
        }
        let ch = stream.next()?;
        match ch {
            '"' | '\'' => Some(self.string(stream, ch)),
            '-' if stream.eat('-').is_some() => {
                stream.skip_to_end();
                Some("comment")
            }
            ':' if stream.eat('=').is_some() => Some("operator"),
            '0'..='9' => {
                stream.eat_while(|ch| ch.is_ascii_digit() || "xXbBCc.".contains(ch));
                stream.eat_if(|ch| matches!(ch, '?' | '!'));
                Some(classify_word(stream.current()))
            }
            ch if word(ch) => {
                stream.eat_while(word);
                stream.eat_if(|ch| matches!(ch, '?' | '!'));
                Some(classify_word(stream.current()))
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                Some("operator")
            }
            _ => None,
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.0 = (quoted(stream, quote, '%') != QuoteEnd::Closed).then_some(quote);
        "string"
    }
}

fn operator(ch: char) -> bool {
    "=+-/*^%<>~".contains(ch)
}

fn classify_word(text: &str) -> &'static str {
    if KEYWORDS.split_ascii_whitespace().any(|word| word == text) {
        "keyword"
    } else if text.starts_with(|ch: char| ch.is_ascii_uppercase())
        && text
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        "tag"
    } else if NUMBER.is_match(text) {
        "number"
    } else {
        "variable"
    }
}
