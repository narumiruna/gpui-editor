use super::{super::js_space, word};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    quote: Option<char>,
    contexts: Vec<Context>,
    indent: usize,
}

#[derive(Clone, Debug)]
struct Context {
    kind: Kind,
    indent: usize,
    column: usize,
    align: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Close(char),
    Pattern,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            if let Some(context) = self.contexts.last_mut() {
                context.align.get_or_insert(false);
            }
            self.indent = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        let (style, punct) = self.scan(stream);
        if style != Some("comment")
            && let Some(context) = self.contexts.last_mut()
            && context.kind != Kind::Pattern
        {
            context.align.get_or_insert(true);
        }
        match punct {
            Some('(') => self.push(Kind::Close(')'), stream.column()),
            Some('[') => self.push(Kind::Close(']'), stream.column()),
            Some('{') => self.push(Kind::Close('}'), stream.column()),
            Some(ch @ (']' | '}' | ')')) => {
                while self.in_pattern() {
                    self.pop();
                }
                if self
                    .contexts
                    .last()
                    .is_some_and(|c| c.kind == Kind::Close(ch))
                {
                    self.pop();
                }
            }
            Some('.') if self.in_pattern() => self.pop(),
            _ if matches!(style, Some("atom" | "string" | "variableName.special")) => {
                if let Some(context) = self.contexts.last_mut() {
                    match context.kind {
                        Kind::Close(']' | '}') => self.push(Kind::Pattern, stream.column()),
                        Kind::Pattern if context.align != Some(true) => {
                            context.align = Some(true);
                            context.column = stream.column();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> usize {
        let first = text_after.chars().next();
        let context = self
            .contexts
            .iter()
            .rev()
            .find(|context| !matches!(first, Some(']' | '}')) || context.kind != Kind::Pattern);
        match context {
            None => 0,
            Some(Context {
                kind: Kind::Pattern,
                column,
                ..
            }) => *column,
            Some(Context {
                kind: Kind::Close(ch),
                align,
                column,
                indent,
            }) => {
                let closing = first == Some(*ch);
                if *align == Some(true) {
                    column + usize::from(!closing)
                } else if closing {
                    *indent
                } else {
                    indent + indent_unit
                }
            }
        }
    }

    fn scan(&mut self, stream: &mut StringStream<'_>) -> (Option<&'static str>, Option<char>) {
        if let Some(quote) = self.quote {
            return (Some(self.literal(stream, quote)), None);
        }
        let Some(ch) = stream.next() else {
            return (None, None);
        };
        let style = match ch {
            '<' if !stream.peek().is_some_and(|c| c == '=' || js_space(c)) => {
                stream.eat_while(|c| c != '>' && !js_space(c));
                stream.eat('>');
                "atom"
            }
            '"' | '\'' => self.literal(stream, ch),
            '{' | '}' | '(' | ')' | ',' | '.' | ';' | '[' | ']' => return (None, Some(ch)),
            '#' => {
                stream.skip_to_end();
                "comment"
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                return (None, None);
            }
            ':' => "operator",
            _ => {
                stream.eat_while(word);
                let current = stream.current();
                if stream.peek() == Some(':') {
                    "variableName.special"
                } else if ["@prefix", "@base", "a"]
                    .iter()
                    .any(|k| current.eq_ignore_ascii_case(k))
                {
                    "meta"
                } else if ch.is_ascii_uppercase() {
                    "comment"
                } else {
                    "keyword"
                }
            }
        };
        (Some(style), None)
    }

    fn literal(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.quote = Some(quote);
        let mut escaped = false;
        for ch in stream.by_ref() {
            if ch == quote && !escaped {
                self.quote = None;
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        "string"
    }

    fn in_pattern(&self) -> bool {
        self.contexts
            .last()
            .is_some_and(|c| c.kind == Kind::Pattern)
    }

    fn push(&mut self, kind: Kind, column: usize) {
        self.contexts.push(Context {
            kind,
            indent: self.indent,
            column,
            align: None,
        });
    }

    fn pop(&mut self) {
        self.indent = self.contexts.pop().unwrap().indent;
    }
}

fn operator(ch: char) -> bool {
    matches!(ch, '*' | '+' | '-' | '<' | '>' | '=' | '&' | '|')
}
