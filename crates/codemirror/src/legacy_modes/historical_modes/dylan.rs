use std::sync::LazyLock;

use regex::Regex;

use super::{QuoteEnd, StringStream, block_comment_ended, quoted, word};

static DECIMAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[+-]?(?:[0-9]*\.[0-9]*(?:[esdxESDX][+-]?[0-9]+)?|[0-9]+(?:[esdxESDX][+-]?[0-9]+)?)",
    )
    .unwrap()
});
static HASH_ATOM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u)^(?:f|t|all-keys|include|key|next|rest)").unwrap());
const SYMBOL: &str = "[-_a-zA-Z?!*@<>$%]+";
static SYMBOLS: LazyLock<[(Regex, Option<&str>); 5]> = LazyLock::new(|| {
    [
        (format!("^{SYMBOL}:"), Some("atom")),
        (format!("^<{SYMBOL}>"), Some("tag")),
        (format!(r"^\*{SYMBOL}\*"), Some("variableName.standard")),
        (format!(r"^\${SYMBOL}"), Some("variableName.constant")),
        (format!("^with(?:out)?-{SYMBOL}"), None),
    ]
    .map(|(pattern, style)| (Regex::new(&pattern).unwrap(), style))
});

#[derive(Clone, Debug, Default)]
pub(super) enum State {
    #[default]
    Base,
    Comment,
    String(char),
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match *self {
            Self::Base => self.base(stream),
            Self::Comment => Some(self.comment(stream)),
            Self::String(quote) => Some(self.string(stream, quote)),
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.peek()?;
        match ch {
            '\'' | '"' => {
                stream.next();
                return Some(self.string(stream, ch));
            }
            '/' if stream.match_str("/*", true, false) => return Some(self.comment(stream)),
            '/' if stream.match_str("//", true, false) => {
                stream.skip_to_end();
                return Some("comment");
            }
            '+' | '-' | '.' | '0'..='9' if stream.match_regex(&DECIMAL, true).is_some() => {
                return Some("number");
            }
            '#' => return Some(self.hash(stream)),
            '~' => {
                stream.next();
                if stream.eat('=').is_some() {
                    stream.eat('=');
                }
                return Some("operator");
            }
            ':' => {
                stream.next();
                if stream.eat('=').is_some() {
                    return Some("operator");
                }
                if stream.eat(':').is_some() {
                    return Some("punctuation");
                }
            }
            '[' | ']' | '(' | ')' | '{' | '}' => {
                stream.next();
                return Some("bracket");
            }
            ',' => {
                stream.next();
                return Some("punctuation");
            }
            _ => {}
        }
        if ch != ':' && stream.match_str("end", true, false) {
            return Some("keyword");
        }
        for (pattern, style) in &*SYMBOLS {
            if stream.match_regex(pattern, true).is_some() {
                return *style;
            }
        }
        if stream.eat_if(|ch| "+-*/^=<>&|".contains(ch)).is_some() {
            return Some("operator");
        }
        if stream.match_str("define", true, false) {
            return Some("def");
        }
        stream.eat_while(|ch| word(ch) || ch == '-');
        let current = stream.current();
        if let Some(style) = word_style(current) {
            Some(style)
        } else if current
            .starts_with(|ch: char| ch.is_ascii_alphabetic() || matches!(ch, '_' | '-'))
        {
            Some("variable")
        } else {
            stream.next();
            Some("variableName.standard")
        }
    }

    fn hash(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        stream.next();
        match stream.peek() {
            Some('"') => {
                stream.next();
                self.string(stream, '"')
            }
            Some(base @ ('b' | 'x' | 'o')) => {
                stream.next();
                stream.eat_while(|ch| match base {
                    'b' => matches!(ch, '0' | '1'),
                    'x' => ch.is_ascii_hexdigit(),
                    _ => matches!(ch, '0'..='7'),
                });
                "number"
            }
            Some('#') => {
                stream.next();
                "punctuation"
            }
            Some('[' | '(') => {
                stream.next();
                "bracket"
            }
            _ if stream.match_regex(&HASH_ATOM, true).is_some() => "atom",
            _ => {
                stream.eat_while(|ch| ch.is_ascii_alphabetic() || ch == '-');
                "error"
            }
        }
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        // The nested depth is local to each token, not to the whole comment.
        *self = if block_comment_ended(stream, true) {
            Self::Base
        } else {
            Self::Comment
        };
        "comment"
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        *self = if quoted(stream, quote, '\\') == QuoteEnd::Escaped {
            Self::String(quote)
        } else {
            Self::Base
        };
        "string"
    }
}

fn word_style(word: &str) -> Option<&'static str> {
    let groups = [
        (
            "def",
            "interface module library macro C-struct C-union C-function C-callable-wrapper class C-subtype C-mapped-subtype method function C-variable C-address constant variable generic domain C-pointer-type table",
        ),
        ("builtin", "signal error cerror break check-type abort"),
        (
            "keyword",
            "if block begin case for select when unless until while iterate profiling dynamic-bind finally exception cleanup else elseif afterwards above below by from handler in instance let local otherwise slot subclass then to keyed-by virtual",
        ),
    ];
    groups.into_iter().find_map(|(style, words)| {
        words
            .split_ascii_whitespace()
            .any(|entry| entry == word)
            .then_some(style)
    })
}
