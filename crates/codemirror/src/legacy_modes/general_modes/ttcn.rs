use std::sync::LazyLock;

use super::{StringStream, closes_comment, digit, until_unescaped, word};

mod words;

pub(super) fn autocomplete() -> &'static [&'static str] {
    static WORDS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        [
            words::KEYWORDS,
            words::BUILTIN,
            words::TIMER_OPS,
            words::PORT_OPS,
        ]
        .into_iter()
        .flat_map(str::split_ascii_whitespace)
        .collect()
    });
    &WORDS
}

// The upstream exports maintain a syntax context but do not expose an indent
// hook, and the context does not affect tokenization.
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Base,
    String(char),
    Comment,
}

impl State {
    pub(super) fn token(
        &mut self,
        stream: &mut StringStream<'_>,
        config: bool,
    ) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        Some(match *self {
            Self::String(quote) => self.string(stream, quote),
            Self::Comment => self.comment(stream),
            Self::Base => match stream.next()? {
                quote @ ('"' | '\'') => self.string(stream, quote),
                '#' => {
                    stream.skip_to_end();
                    if config { "comment" } else { "atom" }
                }
                ch if digit(ch) => {
                    stream.eat_while(|ch| word(ch) || ch == '.');
                    "number"
                }
                ch if config => config_token(stream, ch),
                '%' => {
                    // The upstream word-boundary test is on one character at a time.
                    stream.eat_while(word);
                    "atom"
                }
                ch if "[]{}(),;\\:?.".contains(ch) => "punctuation",
                '/' if stream.eat('*').is_some() => self.comment(stream),
                '/' if stream.eat('/').is_some() => {
                    stream.skip_to_end();
                    "comment"
                }
                '@' if ["try", "catch", "lazy"]
                    .iter()
                    .any(|s| stream.match_str(s, true, false)) =>
                {
                    "keyword"
                }
                ch if operator(ch) => {
                    stream.eat_while(operator);
                    "operator"
                }
                _ => {
                    stream.eat_while(|ch| word(ch) || ch == '$' || ch >= '\u{a1}');
                    classify(stream.current(), words::TTCN)
                }
            },
        })
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        *self = if until_unescaped(stream, quote) {
            stream.eat_if(|ch| matches!(ch.to_ascii_lowercase(), 'b' | 'h' | 'o'));
            Self::Base
        } else {
            Self::String(quote)
        };
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        *self = if closes_comment(stream) {
            Self::Base
        } else {
            Self::Comment
        };
        "comment"
    }
}

fn operator(ch: char) -> bool {
    "+-*&@=<>!/".contains(ch)
}

fn config_token(stream: &mut StringStream<'_>, ch: char) -> &'static str {
    match ch {
        ':' | '=' => "punctuation",
        '|' => {
            stream.eat_while(|ch| ch == '|');
            "operator"
        }
        '[' => {
            stream.eat_while(|ch| word(ch) || ch == ']');
            "number"
        }
        _ => {
            stream.eat_while(|ch| word(ch) || ch == '$');
            classify(stream.current(), words::CONFIG)
        }
    }
}

fn classify(word: &str, groups: &[(&str, &'static str)]) -> &'static str {
    groups
        .iter()
        .find_map(|(words, style)| {
            words
                .split_ascii_whitespace()
                .any(|item| item == word)
                .then_some(*style)
        })
        .unwrap_or("variable")
}
