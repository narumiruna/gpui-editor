use std::sync::LazyLock;

use super::{StringStream, close_string, word};

const KEYWORDS: &str = "algorithm and annotation assert block break class connect connector constant constrainedby der discrete each else elseif elsewhen encapsulated end enumeration equation expandable extends external false final flow for function if import impure in initial inner input loop model not operator or outer output package parameter partial protected public pure record redeclare replaceable return stream then true type when while within";
const BUILTINS: &str = "abs acos actualStream asin atan atan2 cardinality ceil cos cosh delay div edge exp floor getInstanceName homotopy inStream integer log log10 mod pre reinit rem semiLinear sign sin sinh spatialDistribution sqrt tan tanh";
const ATOMS: &str = "Real Boolean Integer String";

pub(super) fn completions() -> &'static [&'static str] {
    static COMPLETIONS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        [KEYWORDS, BUILTINS, ATOMS]
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .collect()
    });
    &COMPLETIONS
}

#[derive(Clone, Debug)]
pub(super) struct State {
    continuation: Option<Continuation>,
    level: usize,
    first: bool,
}

#[derive(Clone, Copy, Debug)]
enum Continuation {
    Comment,
    String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            continuation: None,
            level: 0,
            first: true,
        }
    }
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.continuation {
            Some(Continuation::Comment) => return Some(self.comment(stream)),
            Some(Continuation::String) => return Some(self.string(stream)),
            None => {}
        }
        if stream.sol() {
            self.first = true;
        }
        if stream.eat_space() {
            return None;
        }
        let ch = stream.next()?;
        let style = match ch {
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            '/' if stream.eat('*').is_some() => self.comment(stream),
            ch if matches!(
                (ch, stream.peek()),
                (':' | '<' | '>' | '=', Some('='))
                    | ('<', Some('>'))
                    | ('.', Some('+' | '-' | '*' | '/' | '^'))
            ) =>
            {
                stream.next();
                "operator"
            }
            ch if ";=(:),{}.*<>+-/^[]".contains(ch) => "operator",
            ch if ch.is_ascii_alphabetic() || ch == '_' => self.identifier(stream),
            '\'' if stream.peek().is_some_and(|ch| ch != '\'') => {
                stream.eat_while(|ch| ch != '\'');
                self.first = false;
                if stream.eat('\'').is_some() {
                    "variable"
                } else {
                    "error"
                }
            }
            '"' => self.string(stream),
            ch if ch.is_ascii_digit() => {
                stream.eat_while(|ch| ch.is_ascii_digit());
                if stream.eat('.').is_some() {
                    stream.eat_while(|ch| ch.is_ascii_digit());
                }
                if stream.eat_if(|ch| matches!(ch, 'e' | 'E')).is_some() {
                    stream.eat_if(|ch| matches!(ch, '+' | '-'));
                    stream.eat_while(|ch| ch.is_ascii_digit());
                }
                self.first = false;
                "number"
            }
            _ => "error",
        };
        Some(style)
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        self.continuation.is_none().then(|| {
            let dedent = [
                "algorithm",
                "equation",
                "initial algorithm",
                "initial equation",
                "end",
            ]
            .into_iter()
            .filter(|word| text_after.contains(word))
            .count();
            self.level.saturating_sub(dedent).saturating_mul(unit)
        })
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.continuation = None;
        } else {
            stream.skip_to_end();
            self.continuation = Some(Continuation::Comment);
        }
        "comment"
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if close_string(stream) {
            self.continuation = None;
            self.first = false;
        } else {
            self.continuation = Some(Continuation::String);
        }
        "string"
    }

    fn identifier(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        stream.eat_while(word);
        let text = stream.current();
        if self.first {
            match text {
                "package" | "model" | "when" | "connector" => self.level += 1,
                "end" => self.level = self.level.saturating_sub(1),
                _ => {}
            }
        }
        self.first = false;
        if KEYWORDS.split_ascii_whitespace().any(|item| item == text) {
            "keyword"
        } else if BUILTINS.split_ascii_whitespace().any(|item| item == text) {
            "builtin"
        } else if ATOMS.split_ascii_whitespace().any(|item| item == text) {
            "atom"
        } else {
            "variable"
        }
    }
}
