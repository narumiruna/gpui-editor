use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;

use super::{SPACE, block_end, match_token, scheme_words as words, until_unescaped};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

static BUILTINS: LazyLock<HashSet<&str>> =
    LazyLock::new(|| words::BUILTINS.split_ascii_whitespace().collect());
static BINARY: LazyLock<Regex> = LazyLock::new(|| radix_pattern("[01]"));
static OCTAL: LazyLock<Regex> = LazyLock::new(|| radix_pattern("[0-7]"));
static HEX: LazyLock<Regex> = LazyLock::new(|| radix_pattern("[0-9a-f]"));
static DECIMAL: LazyLock<Regex> = LazyLock::new(|| {
    let digits = "[0-9]+";
    let unsigned = format!(
        r"(?:(?:{digits}#+\.?#*|{digits}\.[0-9]*#*|\.{digits}#*|{digits})(?:[esfdl][+-]?{digits})?|{digits}#*/{digits}#*)"
    );
    number_pattern(&unsigned, &unsigned)
});

fn radix_pattern(digit: &str) -> Regex {
    let unsigned = format!(r"{digit}+#*(?:/{digit}+#*)?");
    number_pattern(&unsigned, &format!("[+-]?{unsigned}"))
}

fn number_pattern(unsigned: &str, real: &str) -> Regex {
    Regex::new(&format!(
        r#"^((?i-u:[+-]i|[+-]{unsigned}i|[+-]?{unsigned}@[+-]?{unsigned}|[+-]?{unsigned}[+-](?:{unsigned})?i|{real}))(?:[(){SPACE};"]|$)"#
    )).unwrap()
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Base,
    String,
    Symbol,
    Comment,
    DatumComment,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    stack: Vec<(char, usize)>,
    indentation: usize,
    scan: Scan,
    comment_depth: Option<usize>,
    quote_depth: Option<usize>,
}

impl State {
    pub(super) fn indent(&self) -> usize {
        self.stack
            .last()
            .map_or(self.indentation, |&(_, indent)| indent)
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if self.stack.is_empty() && stream.sol() {
            self.indentation = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        let style = match self.scan {
            Scan::String => self.until(stream, '"', "string"),
            Scan::Symbol => self.until(stream, '|', "symbol"),
            Scan::Comment => {
                if block_end(stream) {
                    self.scan = Scan::Base;
                }
                Some("comment")
            }
            Scan::DatumComment => {
                self.scan = Scan::Base;
                if is_open(stream.peek()) {
                    self.comment_depth = Some(0);
                    self.base(stream)
                } else {
                    stream.eat_while(|ch| !js_space(ch) && !"()[]".contains(ch));
                    Some("comment")
                }
            }
            Scan::Base => self.base(stream),
        };
        if self.comment_depth.is_some() {
            Some("comment")
        } else if self.quote_depth.is_some() {
            Some("atom")
        } else {
            style
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let start = stream.pos();
        let ch = stream.next()?;
        match ch {
            '"' => {
                self.scan = Scan::String;
                Some("string")
            }
            '\'' => {
                if is_open(stream.peek()) {
                    self.quote_depth.get_or_insert(0);
                } else {
                    stream.eat_while(is_word);
                }
                Some("atom")
            }
            '|' => {
                self.scan = Scan::Symbol;
                Some("symbol")
            }
            '#' => self.hash(stream, start),
            ';' => {
                stream.skip_to_end();
                Some("comment")
            }
            '(' | '[' => {
                self.open(stream, ch);
                Some("bracket")
            }
            ')' | ']' => Some(self.close(ch)),
            _ => {
                if is_number_start(ch) {
                    stream.set_pos(start).unwrap();
                    if match_token(stream, &DECIMAL) {
                        return Some("number");
                    }
                    stream.set_pos(start + 1).unwrap();
                }
                stream.eat_while(is_word);
                Some(if BUILTINS.contains(stream.current()) {
                    "builtin"
                } else {
                    "variable"
                })
            }
        }
    }

    fn hash(&mut self, stream: &mut StringStream<'_>, start: usize) -> Option<&'static str> {
        if stream.eat('|').is_some() {
            self.scan = Scan::Comment;
            return Some("comment");
        }
        if stream
            .eat_if(|ch| matches!(ch, 't' | 'T' | 'f' | 'F'))
            .is_some()
        {
            return Some("atom");
        }
        if stream.eat(';').is_some() {
            self.scan = Scan::DatumComment;
            return Some("comment");
        }

        let has_exactness = exactness(stream);
        if !has_exactness {
            stream.set_pos(start).unwrap();
        }
        let matcher = if stream.match_str("#b", true, true) {
            &BINARY
        } else if stream.match_str("#o", true, true) {
            &OCTAL
        } else if stream.match_str("#x", true, true) {
            &HEX
        } else if stream.match_str("#d", true, true) {
            &DECIMAL
        } else if stream.peek().is_some_and(is_number_start) {
            return match_token(stream, &DECIMAL).then_some("number");
        } else {
            if !has_exactness {
                stream.eat('#');
            }
            return None;
        };
        if !has_exactness && stream.eat('#').is_some() && !exactness(stream) {
            stream.back_up(1).unwrap();
        }
        match_token(stream, matcher).then_some("number")
    }

    fn until(
        &mut self,
        stream: &mut StringStream<'_>,
        end: char,
        style: &'static str,
    ) -> Option<&'static str> {
        if until_unescaped(stream, end) {
            self.scan = Scan::Base;
        }
        Some(style)
    }

    fn open(&mut self, stream: &mut StringStream<'_>, ch: char) {
        let after_open = stream.pos();
        let column = stream.column();
        stream.eat_while(|ch| !js_space(ch) && !"([;)]".contains(ch));
        let head = &stream.line()[after_open..stream.pos()];
        let offset = if words::indent_word(head) {
            2
        } else {
            stream.eat_space();
            if stream.eol() || stream.peek() == Some(';') {
                1
            } else {
                stream.current().chars().count()
            }
        };
        stream.set_pos(after_open).unwrap();
        self.stack.push((ch, column + offset));
        for depth in [&mut self.comment_depth, &mut self.quote_depth]
            .into_iter()
            .flatten()
        {
            *depth += 1;
        }
    }

    fn close(&mut self, ch: char) -> &'static str {
        let expected = if ch == ')' { '(' } else { '[' };
        if self.stack.last().is_none_or(|&(open, _)| open != expected) {
            return "bracket";
        }
        self.stack.pop();
        let comment_closed = close_datum(&mut self.comment_depth);
        let quote_closed = close_datum(&mut self.quote_depth);
        match (comment_closed, quote_closed) {
            (_, true) => "atom",
            (true, false) => "comment",
            _ => "bracket",
        }
    }
}

fn close_datum(depth: &mut Option<usize>) -> bool {
    if let Some(n) = depth {
        *n -= 1;
        if *n == 0 {
            *depth = None;
            return true;
        }
    }
    false
}

fn is_open(ch: Option<char>) -> bool {
    matches!(ch, Some('(' | '['))
}

fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "_-!$%&*+./:<=>?@^~".contains(ch)
}

fn is_number_start(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.')
}

fn exactness(stream: &mut StringStream<'_>) -> bool {
    stream
        .eat_if(|ch| matches!(ch, 'e' | 'E' | 'i' | 'I'))
        .is_some()
}
