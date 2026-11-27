use std::sync::LazyLock;

use regex::Regex;

use super::{QuoteEnd, StringStream, block_comment_ended, js_space, leading_word, quoted};

const BODIED: &str = "Assert BackQuote D Defun Deriv For ForEach FromFile FromString Function Integrate InverseTaylor Limit LocalSymbols Macro MacroRule MacroRulePattern NIntegrate Rule RulePattern Subst TD TExplicitSum TSum Taylor Taylor1 Taylor2 Taylor3 ToFile ToStdout ToString TraceRule Until While";
const IDENTIFIER: &str = "[a-zA-Z$'][a-zA-Z0-9$']*";
static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]+ *#").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\.[0-9]+|[0-9]+\.[0-9]*|[0-9]+)(?:[eE][+-]?[0-9]+)?").unwrap()
});
static PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^(?:{IDENTIFIER})?_{IDENTIFIER}")).unwrap());
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("^{IDENTIFIER}")).unwrap());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Base,
    Comment,
    String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Bodied,
    Delimiter(char),
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    scopes: Vec<Scope>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        Some(match self.scan {
            Scan::Base => self.base(stream)?,
            Scan::Comment => self.comment(stream),
            Scan::String => self.string(stream),
        })
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.peek()?;
        if stream.eat('"').is_some() {
            return Some(self.string(stream));
        }
        if stream.match_str("/*", true, false) {
            return Some(self.comment(stream));
        }
        if stream.match_str("//", true, false) {
            stream.skip_to_end();
            return Some("comment");
        }
        self.update_scopes(&stream.line()[stream.pos()..], ch);
        if stream.match_regex(&RULE, true).is_some() {
            Some("qualifier")
        } else if stream.match_regex(&NUMBER, true).is_some() {
            Some("number")
        } else if stream.match_regex(&PATTERN, true).is_some() {
            Some("variableName.special")
        } else if "[]{}()".contains(ch) {
            stream.next();
            Some("bracket")
        } else if stream.match_regex(&NAME, true).is_some() {
            let tail = &stream.line()[stream.pos()..];
            let after_space = tail.trim_start_matches(js_space);
            if after_space.starts_with('(') {
                stream
                    .set_pos(stream.line().len() - after_space.len())
                    .unwrap();
                Some("variableName.function")
            } else {
                Some("variable")
            }
        } else if "\\+-*/,;.:@~=><&|_`'^?!%#".contains(ch) {
            stream.next();
            Some("operator")
        } else {
            stream.next();
            Some("error")
        }
    }

    fn update_scopes(&mut self, tail: &str, ch: char) {
        let name = leading_word(tail);
        if BODIED.split_ascii_whitespace().any(|word| word == name)
            && tail[name.len()..]
                .trim_start_matches(js_space)
                .starts_with('(')
        {
            self.scopes.push(Scope::Bodied);
        }
        match ch {
            '[' | '{' | '(' => {
                if ch == '[' && self.scopes.last() == Some(&Scope::Bodied) {
                    self.scopes.pop();
                }
                let close = match ch {
                    '[' => ']',
                    '{' => '}',
                    _ => ')',
                };
                self.scopes.push(Scope::Delimiter(close));
            }
            ']' | '}' | ')' if self.scopes.last() == Some(&Scope::Delimiter(ch)) => {
                self.scopes.pop();
            }
            ';' => {
                while self.scopes.last() == Some(&Scope::Bodied) {
                    self.scopes.pop();
                }
            }
            _ => {}
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = if quoted(stream, '"', '\\') == QuoteEnd::Closed {
            Scan::Base
        } else {
            Scan::String
        };
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = if block_comment_ended(stream, false) {
            Scan::Base
        } else {
            Scan::Comment
        };
        "comment"
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        let closing = matches!(text_after, "]" | "];" | "}" | "};" | ");");
        (self.scan == Scan::Base).then(|| {
            self.scopes
                .len()
                .saturating_sub(usize::from(closing))
                .saturating_mul(unit)
        })
    }
}

pub(super) fn electric_input(input: &str) -> bool {
    input.contains(['{', '}', '[', ']', '(', ')', ';'])
}
