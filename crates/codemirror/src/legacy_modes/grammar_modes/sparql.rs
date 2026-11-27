use super::{StringStream, close_quote, js_space, word};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    quote: Option<char>,
    indentation: usize,
    contexts: Vec<Context>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Group(char),
    Pattern,
}

#[derive(Clone, Copy, Debug)]
struct Context {
    kind: Kind,
    indentation: usize,
    column: usize,
    aligned: Option<bool>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            if let Some(context) = self.contexts.last_mut() {
                context.aligned.get_or_insert(false);
            }
            self.indentation = stream.indentation();
        }
        if stream.eat_space() || stream.eol() {
            return None;
        }
        let (style, punctuation) = if let Some(quote) = self.quote {
            (self.string(stream, quote), None)
        } else {
            self.base(stream)
        };

        if let Some(context) = self.contexts.last_mut()
            && style != "comment"
            && context.kind != Kind::Pattern
        {
            context.aligned.get_or_insert(true);
        }
        match punctuation {
            Some('(') => self.push(Kind::Group(')'), stream.column()),
            Some('[') => self.push(Kind::Group(']'), stream.column()),
            Some('{') => self.push(Kind::Group('}'), stream.column()),
            Some(close @ (')' | ']' | '}')) => {
                while self.context_kind() == Some(Kind::Pattern) {
                    self.pop();
                }
                if self.context_kind() == Some(Kind::Group(close)) {
                    self.pop();
                    if close == '}' && self.context_kind() == Some(Kind::Pattern) {
                        self.pop();
                    }
                }
            }
            Some('.') if self.context_kind() == Some(Kind::Pattern) => self.pop(),
            _ if matches!(style, "atom" | "string" | "variable" | "variableName.local") => {
                match self.context_kind() {
                    Some(Kind::Group(']' | '}')) => self.push(Kind::Pattern, stream.column()),
                    Some(Kind::Pattern) => {
                        let context = self.contexts.last_mut().expect("pattern context");
                        if context.aligned != Some(true) {
                            context.aligned = Some(true);
                            context.column = stream.column();
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Some(style)
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> usize {
        let first = text_after.chars().next();
        let mut contexts = self.contexts.iter().rev();
        let context = if matches!(first, Some(']' | '}')) {
            contexts.find(|context| context.kind != Kind::Pattern)
        } else {
            contexts.next()
        };
        let Some(context) = context else {
            return 0;
        };
        if context.kind == Kind::Pattern {
            return context.column;
        }
        let closing = first.is_some_and(|ch| context.kind == Kind::Group(ch));
        if context.aligned == Some(true) {
            context.column.saturating_add(usize::from(!closing))
        } else if closing {
            context.indentation
        } else {
            context.indentation.saturating_add(indent_unit)
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (&'static str, Option<char>) {
        let ch = stream.next().expect("token starts before end of line");
        let style = match ch {
            '?' if stream.peek().is_some_and(js_space) => "operator",
            '$' | '?' => {
                if stream.eat_if(variable_start).is_some() {
                    stream.eat_while(variable_continue);
                }
                "variableName.local"
            }
            '<' if !stream.peek().is_some_and(|ch| js_space(ch) || ch == '=') => {
                stream.eat_while(|ch| !js_space(ch) && ch != '>');
                stream.eat('>');
                "atom"
            }
            quote @ ('"' | '\'') => self.string(stream, quote),
            '{' | '}' | '(' | ')' | ',' | '.' | ';' | '[' | ']' => return ("bracket", Some(ch)),
            '#' => {
                stream.skip_to_end();
                "comment"
            }
            '*' | '+' | '-' | '<' | '>' | '=' | '&' | '|' | '^' | '/' | '!' => "operator",
            ':' => {
                eat_local(stream);
                "atom"
            }
            '@' => {
                stream.eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '-');
                "meta"
            }
            _ if ch.is_ascii_alphabetic() && eat_prefix(stream) => {
                eat_local(stream);
                "atom"
            }
            _ => {
                stream.eat_while(word);
                word_style(stream.current())
            }
        };
        (style, None)
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.quote = (!close_quote(stream, quote)).then_some(quote);
        "string"
    }

    fn context_kind(&self) -> Option<Kind> {
        self.contexts.last().map(|context| context.kind)
    }

    fn push(&mut self, kind: Kind, column: usize) {
        self.contexts.push(Context {
            kind,
            indentation: self.indentation,
            column,
            aligned: None,
        });
    }

    fn pop(&mut self) {
        self.indentation = self.contexts.pop().expect("existing context").indentation;
    }
}

fn variable_start(ch: char) -> bool {
    word(ch)
        || matches!(ch,
            '\u{00c0}'..='\u{00d6}' | '\u{00d8}'..='\u{00f6}' | '\u{00f8}'..='\u{02ff}'
            | '\u{0370}'..='\u{037d}' | '\u{037f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
            | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
            | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}'
        )
}

fn variable_continue(ch: char) -> bool {
    variable_start(ch)
        || matches!(ch, '\u{00b7}' | '\u{0300}'..='\u{036f}' | '\u{203f}'..='\u{2040}')
}

// A prefix remainder can be empty, but cannot end in a period.
fn eat_prefix(stream: &mut StringStream<'_>) -> bool {
    let rest = &stream.line()[stream.pos()..];
    let prefix = rest.trim_start_matches(|ch| word(ch) || matches!(ch, '-' | '.'));
    let len = rest.len() - prefix.len();
    if !prefix.starts_with(':') || rest[..len].ends_with('.') {
        return false;
    }
    stream
        .set_pos(stream.pos() + len + 1)
        .expect("ASCII prefix");
    true
}

// A period needs a matching next byte. Escapes must match in full.
fn eat_local(stream: &mut StringStream<'_>) {
    let rest = &stream.line().as_bytes()[stream.pos()..];
    let mut tail = rest;
    loop {
        let size = match tail {
            [b'.', next, ..] if next.is_ascii_alphanumeric() || b"_-\\%".contains(next) => 1,
            [ch, ..] if ch.is_ascii_alphanumeric() || b":_-".contains(ch) => 1,
            [b'\\', next, ..] if b"-\\_~.!$&'()*+,;=/?#@%".contains(next) => 2,
            [b'%', a, b, ..] if a.is_ascii_hexdigit() && b.is_ascii_hexdigit() => 3,
            _ => break,
        };
        tail = &tail[size..];
    }
    stream
        .set_pos(stream.pos() + rest.len() - tail.len())
        .expect("ASCII local name");
}

fn word_style(text: &str) -> &'static str {
    match text.to_ascii_lowercase().as_str() {
        "str" | "lang" | "langmatches" | "datatype" | "bound" | "sameterm" | "isiri" | "isuri"
        | "iri" | "uri" | "bnode" | "count" | "sum" | "min" | "max" | "avg" | "sample"
        | "group_concat" | "rand" | "abs" | "ceil" | "floor" | "round" | "concat" | "substr"
        | "strlen" | "replace" | "ucase" | "lcase" | "encode_for_uri" | "contains"
        | "strstarts" | "strends" | "strbefore" | "strafter" | "year" | "month" | "day"
        | "hours" | "minutes" | "seconds" | "timezone" | "tz" | "now" | "uuid" | "struuid"
        | "md5" | "sha1" | "sha256" | "sha384" | "sha512" | "coalesce" | "if" | "strlang"
        | "strdt" | "isnumeric" | "regex" | "exists" | "isblank" | "isliteral" | "a" | "bind" => {
            "builtin"
        }
        "base" | "prefix" | "select" | "distinct" | "reduced" | "construct" | "describe"
        | "ask" | "from" | "named" | "where" | "order" | "limit" | "offset" | "filter"
        | "optional" | "graph" | "by" | "asc" | "desc" | "as" | "having" | "undef" | "values"
        | "group" | "minus" | "in" | "not" | "service" | "silent" | "using" | "insert"
        | "delete" | "union" | "true" | "false" | "with" | "data" | "copy" | "to" | "move"
        | "add" | "create" | "drop" | "clear" | "load" | "into" => "keyword",
        _ => "variable",
    }
}
