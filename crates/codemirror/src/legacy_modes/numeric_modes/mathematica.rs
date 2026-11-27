use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, close_string};

static PATTERNS: LazyLock<[(&str, Regex); 10]> = LazyLock::new(|| {
    let identifier = r"[a-zA-Z$][a-zA-Z0-9$]*";
    let float = r"(?:\.[0-9]+|[0-9]+\.[0-9]*|[0-9]+)";
    let base_float = r"(?-u:\.\w+|\w+\.\w*|\w+)";
    let precision = format!(r"(?:`(?:`?{float})?)?");
    let exponent = r"(?:\*\^[+-]?[0-9]+)?";
    let message = format!(r"{identifier}(?:`[a-zA-Z0-9$]+)*::");
    let space = r"[\x09-\x0d\x20\u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]";
    [
        ("number", format!(r"^[0-9]+\^\^{base_float}{precision}{exponent}")),
        ("number", format!(r"^{float}{precision}{exponent}")),
        ("atom", r"^(?:In|Out)\[[0-9]*\]".into()),
        ("meta", format!(r"^{message}usage")),
        ("string.special", format!(r"^{message}{identifier}:?")),
        (
            "variableName.special",
            format!(r"^{identifier}{space}*:(?:{identifier}|[^:=>~@^&*)\[\]'?,|])[^\r\n\u{{2028}}\u{{2029}}]*"),
        ),
        ("variableName.special", format!(r"^(?:{identifier}_+{identifier}|{identifier}_+|_+{identifier})")),
        ("character", format!(r"^\\\[{identifier}\]")),
        ("variableName.constant", format!(r"^(?:#{identifier}|#+[0-9]?)")),
        ("keyword", format!(r"^`?{identifier}(?:`{identifier})*`?")),
    ]
    .map(|(style, pattern)| (style, Regex::new(&pattern).expect("valid Mathematica pattern")))
});

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    string: bool,
    comment_level: usize,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() || stream.eol() {
            return None;
        }
        if self.string || (self.comment_level == 0 && stream.eat('"').is_some()) {
            self.string = !close_string(stream);
            return Some("string");
        }
        if self.comment_level > 0 || stream.match_str("(*", true, false) {
            if self.comment_level == 0 {
                self.comment_level = 1;
            }
            let mut previous = None;
            for ch in stream {
                match (previous, ch) {
                    (Some('('), '*') => self.comment_level += 1,
                    (Some('*'), ')') => self.comment_level -= 1,
                    _ => {}
                }
                if self.comment_level == 0 {
                    break;
                }
                previous = Some(ch);
            }
            return Some("comment");
        }
        for (style, pattern) in PATTERNS.iter() {
            if stream.match_regex(pattern, true).is_some() {
                return Some(style);
            }
        }
        Some(match stream.next().expect("non-empty token") {
            '[' | ']' | '{' | '}' | '(' | ')' => "bracket",
            ch if "\\+-*/,;.:@~=><&|_`'^?!%".contains(ch) => "operator",
            _ => "error",
        })
    }
}
