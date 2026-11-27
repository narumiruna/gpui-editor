use std::sync::LazyLock;

use regex::Regex;

use super::{eat_text, remaining, word};
use crate::{
    language::LanguageData,
    legacy_modes::{js_line_terminator, js_space},
    stream_parser::StringStream,
};

const ATOMS: &str = "NULL NA Inf NaN NA_integer_ NA_real_ NA_complex_ NA_character_ TRUE FALSE";
const BUILTINS: &str = "list quote bquote eval return call parse deparse";
const KEYWORDS: &str = "if else repeat while function for in next break";

pub(super) fn data(data: LanguageData) -> LanguageData {
    static COMPLETIONS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        [ATOMS, BUILTINS, KEYWORDS]
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .collect()
    });
    data.with("wordChars", ".")
        .with("autocomplete", COMPLETIONS.as_slice())
}

#[derive(Clone, Debug)]
pub(super) struct State {
    quote: Option<char>,
    contexts: Vec<Context>,
    top_indent_unit: usize,
    indentation: usize,
    after_ident: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Top,
    Block,
    Delimiter(char),
}

#[derive(Clone, Copy, Debug)]
struct Context {
    kind: Kind,
    indentation: usize,
    column: usize,
    aligned: Option<bool>,
    braceless: bool,
    args: bool,
}

impl State {
    pub(super) fn new(indent_unit: usize) -> Self {
        Self {
            quote: None,
            contexts: vec![Context {
                kind: Kind::Top,
                indentation: 0,
                column: 0,
                aligned: Some(false),
                braceless: false,
                args: false,
            }],
            top_indent_unit: indent_unit,
            indentation: 0,
            after_ident: false,
        }
    }

    fn context(&mut self) -> &mut Context {
        self.contexts.last_mut().unwrap()
    }

    fn pop(&mut self) {
        if self.contexts.len() > 1 {
            self.indentation = self.contexts.pop().unwrap().indentation;
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.context().aligned.get_or_insert(false);
            if self.context().braceless {
                self.pop();
            }
            self.indentation = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }

        let mut punctuation = None;
        let style = if let Some(quote) = self.quote {
            Some(self.string(stream, quote))
        } else {
            self.base(stream, &mut punctuation)
        };
        let ctx = self.context();
        if style != Some("comment") && ctx.aligned != Some(false) {
            ctx.aligned = Some(true);
            // The upstream flag update also discards the argument-list marker.
            ctx.args = false;
        }
        if matches!(punctuation, Some(Kind::Delimiter(';' | '{' | '}'))) && ctx.kind == Kind::Block
        {
            self.pop();
        }
        let next = match punctuation {
            Some(Kind::Delimiter('{')) => Some(Kind::Delimiter('}')),
            Some(Kind::Delimiter('(')) => Some(Kind::Delimiter(')')),
            Some(Kind::Delimiter('[')) => Some(Kind::Delimiter(']')),
            Some(Kind::Block) => Some(Kind::Block),
            Some(kind) if kind == self.context().kind => {
                self.pop();
                None
            }
            _ => {
                let ctx = self.context();
                if ctx.kind == Kind::Block && style != Some("comment") {
                    ctx.braceless = true;
                    ctx.args = false;
                }
                None
            }
        };
        if let Some(kind) = next {
            self.contexts.push(Context {
                kind,
                indentation: self.indentation,
                column: stream.column(),
                aligned: None,
                braceless: false,
                args: kind == Kind::Delimiter(')') && self.after_ident,
            });
        }
        self.after_ident = matches!(style, Some("variable" | "keyword"));
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        if self.quote.is_some() {
            return 0;
        }
        let first = text_after.chars().next();
        let mut index = self.contexts.len() - 1;
        let closing = matches!(self.contexts[index].kind, Kind::Delimiter(ch) if first == Some(ch));
        if self.contexts[index].braceless {
            index -= 1;
        }
        let ctx = &self.contexts[index];
        match ctx.kind {
            Kind::Top => unit.saturating_sub(self.top_indent_unit),
            Kind::Block => {
                ctx.indentation
                    .saturating_add(if first == Some('{') { 0 } else { unit })
            }
            Kind::Delimiter(_) if ctx.aligned == Some(true) => {
                ctx.column.saturating_add(usize::from(!closing))
            }
            Kind::Delimiter(_) => ctx
                .indentation
                .saturating_add(if closing { 0 } else { unit }),
        }
    }

    fn base(
        &mut self,
        stream: &mut StringStream<'_>,
        punctuation: &mut Option<Kind>,
    ) -> Option<&'static str> {
        static FRACTION: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^[0-9]*(?:e[+-]?[0-9]+)?").unwrap());
        static NUMBER: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^[0-9]*(?:\.[0-9]+)?(?:e[+-][0-9]+)?L?").unwrap());
        let ch = stream.next()?;
        let style = match ch {
            '#' => {
                stream.skip_to_end();
                "comment"
            }
            '0' if stream.eat('x').is_some() => {
                stream.eat_while(|ch| ch.is_ascii_hexdigit());
                "number"
            }
            '.' if stream.eat_if(|ch| ch.is_ascii_digit()).is_some() => {
                stream.match_regex(&FRACTION, true);
                "number"
            }
            '0'..='9' => {
                stream.match_regex(&NUMBER, true);
                "number"
            }
            '\'' | '"' => {
                self.quote = Some(ch);
                "string"
            }
            '`' => {
                if stream.peek() != Some('`') && stream.skip_to("`") {
                    stream.next();
                }
                "string.special"
            }
            '.' if dot_keyword(stream) => "keyword",
            '.' | 'a'..='z' | 'A'..='Z' => {
                stream.eat_while(|ch| word(ch) || ch == '.');
                let current = stream.current();
                if ATOMS.split_ascii_whitespace().any(|item| item == current) {
                    "atom"
                } else if KEYWORDS
                    .split_ascii_whitespace()
                    .any(|item| item == current)
                {
                    let followed_by_if = remaining(stream)
                        .trim_start_matches(js_space)
                        .strip_prefix("if")
                        .is_some_and(|suffix| suffix.chars().next().is_none_or(js_space));
                    if matches!(
                        current,
                        "if" | "else" | "repeat" | "while" | "function" | "for"
                    ) && !followed_by_if
                    {
                        *punctuation = Some(Kind::Block);
                    }
                    "keyword"
                } else if BUILTINS
                    .split_ascii_whitespace()
                    .any(|item| item == current)
                {
                    "builtin"
                } else {
                    "variable"
                }
            }
            '%' => {
                if stream.skip_to("%") {
                    stream.next();
                }
                "variableName.special"
            }
            '<' if stream.eat('-').is_some() || eat_text(stream, "<-") => "operator",
            '-' if stream.eat('>').is_some() => {
                stream.eat('>');
                "operator"
            }
            '=' if self.context().args => "operator",
            '$' => "operator",
            ch if operator(ch) => {
                stream.eat_while(operator);
                "operator"
            }
            '(' | ')' | '{' | '}' | '[' | ']' | ';' => {
                *punctuation = Some(Kind::Delimiter(ch));
                return (ch == ';').then_some("punctuation");
            }
            _ => return None,
        };
        Some(style)
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        if stream.eat('\\').is_some() {
            let escaped = stream.next();
            if matches!(escaped, Some('u' | 'U'))
                && stream.eat('{').is_some()
                && stream.skip_to("}")
            {
                stream.next();
            } else {
                let count = match escaped {
                    Some('x') => 2,
                    Some('u') => 4,
                    Some('U') => 8,
                    _ => 0,
                };
                if count > 0
                    && remaining(stream)
                        .bytes()
                        .take_while(u8::is_ascii_hexdigit)
                        .take(count)
                        .count()
                        == count
                {
                    stream.set_pos(stream.pos() + count).unwrap();
                } else if matches!(escaped, Some('0'..='7')) {
                    for _ in 0..2 {
                        stream.eat_if(|ch| matches!(ch, '0'..='7'));
                    }
                }
            }
            "string.special"
        } else {
            stream.eat_while(|ch| ch != quote && ch != '\\');
            if stream.eat(quote).is_some() {
                self.quote = None;
            }
            "string"
        }
    }
}

fn operator(ch: char) -> bool {
    "+-*/^<>=!&|~$:".contains(ch)
}

fn dot_keyword(stream: &mut StringStream<'_>) -> bool {
    let mut chars = remaining(stream).chars();
    if let Some(ch) = chars
        .next()
        .filter(|&ch| ch.len_utf16() == 1 && !js_line_terminator(ch))
        && let Some(next) = chars.next().filter(|&ch| ch == '.' || ch.is_ascii_digit())
    {
        stream
            .set_pos(stream.pos() + ch.len_utf8() + next.len_utf8())
            .unwrap();
        if next.is_ascii_digit() {
            stream.eat_while(|ch| ch.is_ascii_digit());
        }
        true
    } else {
        false
    }
}
