use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, in_words, js_space, remaining, take, word};
use crate::legacy_modes::js_line_terminator;

const KEYWORDS: &str = "abstract alignof as async await box break continue const crate do dyn else \
    enum extern fn for final if impl in loop macro match mod move offsetof override priv proc \
    pub pure ref return self sizeof static struct super trait type typeof union unsafe unsized \
    use virtual where while yield";
const ATOMS: &str =
    "Self isize usize char bool u8 u16 u32 u64 f16 f32 f64 i8 i16 i32 i64 str Option";
const BUILTINS: &str = "true false Some None Ok Err";

static CHARACTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"^(?:'(?:[\x{0}-\x{ffff}&&[^'\\]]|\\(?:[nrt0'"]|x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{6}\}))'"#,
        r"|b'(?:[\x{0}-\x{ffff}&&[^']]|\\(?:['\\nrt0]|x[0-9a-fA-F]{2}))')",
    ))
    .unwrap()
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:[0-9][0-9_]*(?:[Ee][+-]?[0-9_]+|\.[0-9_]+(?:[Ee][+-]?[0-9_]+)?)(?:f32|f64)?",
        r"|(?:0(?:b[01_]+|o[0-7_]+|x[0-9a-fA-F_]+)|[0-9][0-9_]*)",
        r"(?:u8|u16|u32|u64|i8|i16|i32|i64|isize|usize)?)",
    ))
    .unwrap()
});
static DEFINITION: LazyLock<Regex> = LazyLock::new(|| {
    // ECMAScript whitespace excludes NEL and includes BOM.
    const SPACE: &str = r"[\x09-\x0d\x20\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";
    Regex::new(&format!(
        "^(let(?:{SPACE}+mut)?|fn|enum|mod|struct|type|union)({SPACE}+)([a-zA-Z_][a-zA-Z0-9_]*)"
    ))
    .unwrap()
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Phase {
    #[default]
    Start,
    String,
    Raw,
    RawHash,
    Comment,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    phase: Phase,
    pending: Option<Definition>,
    indents: Vec<usize>,
}

#[derive(Clone, Debug)]
struct Definition {
    space: usize,
    name: usize,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if let Some(pending) = &mut self.pending {
            if pending.space != 0 {
                advance(stream, std::mem::take(&mut pending.space));
                return None;
            }
            advance(stream, pending.name);
            self.pending = None;
            return Some("def");
        }
        match self.phase {
            Phase::Start => self.start(stream),
            Phase::String => Some(self.string(stream)),
            Phase::Raw | Phase::RawHash => Some(self.raw(stream)),
            Phase::Comment => Some(self.comment(stream)),
        }
    }

    pub(super) fn indent(&self, text_after: &str) -> Option<usize> {
        if self.phase == Phase::Comment {
            return None;
        }
        let dedents = if self.phase == Phase::Start {
            text_after.chars().take_while(|&ch| closing(ch)).count()
        } else {
            0
        };
        Some(
            self.indents[..self.indents.len().saturating_sub(dedents)]
                .last()
                .copied()
                .unwrap_or(0),
        )
    }

    fn start(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if take(stream, "\"") || take(stream, "b\"") {
            self.phase = Phase::String;
            return Some("string");
        }
        let text = remaining(stream);
        if let Some(raw) = text.strip_prefix('r').or_else(|| text.strip_prefix("br")) {
            let suffix = raw.trim_start_matches('#');
            if suffix.starts_with('"') {
                self.phase = if suffix.len() == raw.len() {
                    Phase::Raw
                } else {
                    Phase::RawHash
                };
                advance(stream, text.len() - suffix.len() + 1);
                return Some("string");
            }
        }
        if stream.match_regex(&CHARACTER, true).is_some() {
            return Some("string special");
        }
        if stream.match_regex(&NUMBER, true).is_some() {
            return Some("number");
        }
        if let Some(captures) = stream.match_regex(&DEFINITION, false) {
            advance(stream, captures[1].len());
            self.pending = Some(Definition {
                space: captures[2].len(),
                name: captures[3].len(),
            });
            return Some("keyword");
        }
        if let Some(attribute) = text.strip_prefix("#[").or_else(|| text.strip_prefix("#!["))
            && let Some(end) = line_prefix(attribute).rfind(']')
        {
            advance(stream, text.len() - attribute.len() + end + 1);
            return Some("meta");
        }
        if take(stream, "//") {
            advance(stream, line_prefix(remaining(stream)).len());
            return Some("comment");
        }
        if take(stream, "/*") {
            self.phase = Phase::Comment;
            return Some("comment");
        }
        if stream.eat_while(|ch| "-+/*=<>!".contains(ch)) {
            return Some("operator");
        }
        if stream
            .eat_if(|ch| ch.is_ascii_alphabetic() || ch == '_')
            .is_some()
        {
            stream.eat_while(word);
            let name = stream.current();
            return Some(if in_words(KEYWORDS, name) {
                "keyword"
            } else if in_words(ATOMS, name) {
                "atom"
            } else if in_words(BUILTINS, name) {
                "builtin"
            } else if stream.eat('!').is_some() {
                "macroName"
            } else {
                "variable"
            });
        }
        match stream.next() {
            Some('{' | '[' | '(') => self
                .indents
                .push(stream.indentation().saturating_add(stream.indent_unit())),
            Some(ch) if closing(ch) => {
                self.indents.pop();
            }
            _ => {}
        }
        None
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.eat('"').is_some() {
            self.phase = Phase::Start;
        } else {
            let start = stream.pos();
            while let Some(ch) = stream.eat_if(|ch| ch != '"') {
                if ch == '\\' {
                    if stream.peek().is_some_and(js_line_terminator) {
                        // The upstream body rule cannot consume a backslash before a
                        // line separator. Consume it alone if no earlier text matched.
                        if stream.pos() != start + 1 {
                            stream.back_up(1).unwrap();
                        }
                        break;
                    }
                    stream.next();
                }
            }
        }
        "string"
    }

    fn raw(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let hash = self.phase == Phase::RawHash;
        let close = if hash { "\"#" } else { "\"" };
        if take(stream, close) {
            // The upstream mode accepts any positive number of closing hashes.
            if hash {
                stream.eat_while(|ch| ch == '#');
            }
            self.phase = Phase::Start;
        } else if !stream.skip_to(close) {
            stream.skip_to_end();
        }
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let text = line_prefix(remaining(stream));
        if let Some(end) = text.find("*/") {
            advance(stream, end + 2);
            self.phase = Phase::Start;
        } else if text.is_empty() {
            // The upstream dot rule stalls at a line separator inside the stream.
            stream.next();
        } else {
            advance(stream, text.len());
        }
        "comment"
    }
}

fn advance(stream: &mut StringStream<'_>, bytes: usize) {
    stream.set_pos(stream.pos() + bytes).unwrap();
}

fn line_prefix(text: &str) -> &str {
    text.split(js_line_terminator).next().unwrap_or_default()
}

fn closing(ch: char) -> bool {
    matches!(ch, '}' | ']' | ')')
}

pub(super) fn indent_on_input(text: &str) -> bool {
    text.trim_start_matches(js_space) == "}"
}
