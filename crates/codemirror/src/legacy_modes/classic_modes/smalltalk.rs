use super::{js_space, word};
use crate::stream_parser::StringStream;

const SPECIAL: &str = "+-/\\*~<>=@%|&?!.,:;^";
const KEYWORDS: &[&str] = &["true", "false", "nil", "self", "super", "thisContext"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Base,
    Comment,
    String,
    Symbol,
    Temporaries,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    scan: Scan,
    expect_variable: bool,
    indentation: usize,
    user_indentation_delta: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scan: Scan::Base,
            expect_variable: true,
            indentation: 0,
            user_indentation_delta: 0.0,
        }
    }
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let line_indent = stream.indentation();
        self.user_indentation_delta = if line_indent > 0 {
            line_indent as f64 / stream.indent_unit() as f64 - self.indentation as f64
        } else {
            0.0
        };
        if stream.eat_space() || stream.eol() {
            return None;
        }
        let expect_variable = self.expect_variable;
        self.expect_variable = false;
        match self.scan {
            Scan::Base => self.base(stream, expect_variable),
            Scan::Comment => Some(self.quoted(stream, '"', "comment")),
            Scan::String => Some(self.quoted(stream, '\'', "string")),
            Scan::Symbol => Some(self.quoted(stream, '\'', "string.special")),
            Scan::Temporaries => {
                if stream.eat('|').is_some() {
                    self.scan = Scan::Base;
                    self.expect_variable = true;
                    None
                } else {
                    stream.eat_while(|ch| ch != '|');
                    Some("variable")
                }
            }
        }
    }

    pub(super) fn blank_line(&mut self) {
        self.user_indentation_delta = 0.0;
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> usize {
        let delta = if self.scan == Scan::Base && text_after.starts_with(']') {
            -1.0
        } else {
            self.user_indentation_delta
        };
        ((self.indentation as f64 + delta) * indent_unit as f64) as usize
    }

    fn base(
        &mut self,
        stream: &mut StringStream<'_>,
        expect_variable: bool,
    ) -> Option<&'static str> {
        let ch = stream.next()?;
        Some(match ch {
            '"' => {
                self.scan = Scan::Comment;
                self.quoted(stream, '"', "comment")
            }
            '\'' => {
                self.scan = Scan::String;
                self.quoted(stream, '\'', "string")
            }
            '#' if stream.eat('\'').is_some() => {
                self.scan = Scan::Symbol;
                self.quoted(stream, '\'', "string.special")
            }
            '#' => {
                if stream.eat_while(|next| !js_space(next) && !".{}[]()".contains(next)) {
                    "string.special"
                } else {
                    "meta"
                }
            }
            '$' => {
                if stream.next() == Some('<') {
                    stream.eat_while(|next| !js_space(next) && next != '>');
                    stream.next();
                }
                "string.special"
            }
            '|' if expect_variable => {
                self.scan = Scan::Temporaries;
                return None;
            }
            '[' | ']' | '{' | '}' | '(' | ')' => {
                self.expect_variable = matches!(ch, '[' | '{' | '(');
                if ch == '[' {
                    self.indentation += 1;
                } else if ch == ']' {
                    self.indentation = self.indentation.saturating_sub(1);
                }
                "bracket"
            }
            ch if SPECIAL.contains(ch) => {
                stream.eat_while(|next| SPECIAL.contains(next));
                self.expect_variable = ch != ';';
                "operator"
            }
            '0'..='9' => {
                stream.eat_while(word);
                "number"
            }
            ch if word(ch) => {
                stream.eat_while(word);
                if !expect_variable {
                    return None;
                }
                if KEYWORDS.iter().any(|word| stream.current().contains(word)) {
                    "keyword"
                } else {
                    "variable"
                }
            }
            _ => {
                self.expect_variable = expect_variable;
                return None;
            }
        })
    }

    fn quoted(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
        style: &'static str,
    ) -> &'static str {
        stream.eat_while(|ch| ch != quote);
        if stream.eat(quote).is_some() {
            self.scan = Scan::Base;
        }
        self.expect_variable = quote == '"';
        style
    }
}
