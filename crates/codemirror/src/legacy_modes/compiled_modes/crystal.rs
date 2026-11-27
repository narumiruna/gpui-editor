use std::sync::{Arc, LazyLock};

use regex::Regex;

use super::{in_words, js_space, remaining, take, word};
use crate::stream_parser::StringStream;

const KEYWORDS: &str = "abstract alias as asm begin break case class def do else elsif end ensure enum extend for \
    fun if include instance_sizeof lib macro module next of out pointerof private protected rescue return require \
    select sizeof struct super then type typeof uninitialized union unless until when while with yield __DIR__ \
    __END_LINE__ __FILE__ __LINE__";
const ATOMS: &str = "true false nil self";
const BLOCKS: &str = "def fun macro class module struct lib enum union do for";
const EXPRESSIONS: &str = "if unless case while until begin then";
const DEDENT: &str = "end else elsif rescue ensure";

static OPERATORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[-+/%|&^]|\*\*?|[<>]{2})").unwrap());
static CONDITIONAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[=!]~|===|<=>|[<>=!]=?|[|&]{2}|~)").unwrap());
static INDEXING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[\][?=]?").unwrap());
static OTHER_OPERATORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\.(?:\.\.)?|->|[?:])").unwrap());
static NUMBER_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9_]*(?:\.[0-9_]+)?(?:[eE][+-]?[0-9]+)?").unwrap());
static HERE_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<<-(?:'([A-Z][A-Za-z0-9_]*)'|([A-Z][A-Za-z0-9_]*))").unwrap());

#[derive(Clone, Debug)]
enum Scan {
    Nest {
        open: &'static str,
        close: &'static str,
        style: Option<&'static str>,
        started: bool,
    },
    Macro {
        open: &'static str,
        close: &'static str,
        started: bool,
    },
    Follow(Follow),
    Quote {
        end: Option<char>,
        style: &'static str,
        embed: bool,
    },
    HereDoc {
        phrase: Arc<str>,
        embed: bool,
    },
}

#[derive(Clone, Copy, Debug)]
enum Follow {
    Ident,
    Type,
    Macro,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Previous {
    #[default]
    Other,
    Dot,
    Escape,
    Abstract,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    Lib,
    Other,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scans: Vec<Scan>,
    current_indent: isize,
    last_token: Previous,
    last_style: Option<&'static str>,
    blocks: Vec<Block>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = self.read(stream);
        if style.is_some() && style != Some("comment") {
            self.last_style = style;
            self.last_token = match stream.current() {
                "." => Previous::Dot,
                "\\" => Previous::Escape,
                "abstract" => Previous::Abstract,
                _ => Previous::Other,
            };
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let text = text_after.trim_matches(js_space);
        let text = text
            .strip_prefix("{%")
            .unwrap_or(text)
            .trim_start_matches(js_space);
        let text = text
            .strip_suffix("%}")
            .unwrap_or(text)
            .trim_end_matches(js_space);
        let dedent = matches!(text, ")" | "}" | "]") || in_words(DEDENT, word_prefix(text));
        let level = self.current_indent.saturating_sub(isize::from(dedent));
        unit.saturating_mul(level.max(0).cast_unsigned())
    }

    fn read(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.scans.last().cloned() {
            None => self.base(stream),
            Some(Scan::Nest {
                open,
                close,
                style,
                started,
            }) => {
                if !started && take(stream, open) {
                    *self.scans.last_mut().unwrap() = Scan::Nest {
                        open,
                        close,
                        style,
                        started: true,
                    };
                    self.current_indent += 1;
                    return style;
                }
                let next = self.base(stream);
                if stream.current() == close {
                    self.scans.pop();
                    self.current_indent -= 1;
                    style
                } else {
                    next
                }
            }
            Some(Scan::Macro {
                open,
                close,
                started,
            }) => {
                if !started && take(stream, open) {
                    *self.scans.last_mut().unwrap() = Scan::Macro {
                        open,
                        close,
                        started: true,
                    };
                    self.current_indent += 1;
                    Some("meta")
                } else if take(stream, close) {
                    self.scans.pop();
                    self.current_indent -= 1;
                    Some("meta")
                } else {
                    self.base(stream)
                }
            }
            Some(Scan::Follow(kind)) => self.follow(stream, kind),
            Some(Scan::Quote { end, style, embed }) => Some(self.quoted(stream, end, style, embed)),
            Some(Scan::HereDoc { phrase, embed }) => {
                if stream.sol() {
                    stream.eat_space();
                    if take(stream, &phrase) {
                        self.scans.pop();
                        return Some("string");
                    }
                }
                Some(self.quoted(stream, None, "string", embed))
            }
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        if self.last_token != Previous::Escape
            && let Some(scan) = macro_at(remaining(stream))
        {
            return self.chain(stream, scan);
        }
        if stream.peek() == Some('#') {
            stream.skip_to_end();
            return Some("comment");
        }
        if ident(stream, false) {
            stream.eat_if(|ch| matches!(ch, '?' | '!'));
            let matched = stream.current();
            return Some(if stream.eat(':').is_some() {
                "atom"
            } else if self.last_token == Previous::Dot {
                "property"
            } else if in_words(KEYWORDS, word_prefix(matched)) {
                self.keyword(matched);
                "keyword"
            } else if in_words(ATOMS, word_prefix(matched)) {
                "atom"
            } else {
                "variable"
            });
        }
        if stream.eat('@').is_some() {
            if stream.peek() == Some('[') {
                return self.chain(stream, nest("[", "]", Some("meta")));
            }
            stream.eat('@');
            let _ = ident(stream, false) || ident(stream, true);
            return Some("propertyName");
        }
        if ident(stream, true) {
            return Some("tag");
        }
        if stream.eat(':').is_some() {
            if stream.eat('"').is_some() {
                return self.chain(stream, quote(Some('"'), "atom", false));
            }
            if ident(stream, false) || ident(stream, true) || symbol_op(stream) {
                return Some("atom");
            }
            stream.eat(':');
            return Some("operator");
        }
        if stream.eat('"').is_some() {
            return self.chain(stream, quote(Some('"'), "string", true));
        }
        if stream.peek() == Some('%') {
            return self.percent(stream);
        }
        if let Some(matched) = stream.match_regex(&HERE_START, true) {
            let (phrase, embed) = if let Some(quoted) = matched.get(1) {
                (quoted.as_str(), false)
            } else {
                (matched.get(2).unwrap().as_str(), true)
            };
            return self.chain(
                stream,
                Scan::HereDoc {
                    phrase: phrase.into(),
                    embed,
                },
            );
        }
        if stream.eat('\'').is_some() {
            // The first upstream regexp branch accepts every non-quote,
            // including a backslash, before trying any escape patterns.
            stream.eat_if(|ch| ch != '\'');
            stream.eat('\'');
            return Some("atom");
        }
        if number(stream) {
            return Some("number");
        }
        if stream.match_regex(&OPERATORS, true).is_some() {
            stream.eat('=');
            return Some("operator");
        }
        if stream.match_regex(&CONDITIONAL, true).is_some()
            || stream.match_regex(&OTHER_OPERATORS, true).is_some()
        {
            return Some("operator");
        }
        match stream.peek() {
            Some('(') => return self.chain(stream, nest("(", ")", None)),
            Some('{') => return self.chain(stream, nest("{", "}", None)),
            Some('[') => return self.chain(stream, nest("[", "]", None)),
            Some('\\') => {
                stream.next();
                stream.next();
                return Some("meta");
            }
            _ => {}
        }
        stream.next();
        None
    }

    fn keyword(&mut self, matched: &str) {
        let prefix = word_prefix(matched);
        if in_words(BLOCKS, prefix) {
            let declaration = (matched == "fun" && self.blocks.contains(&Block::Lib))
                || (matched == "def" && self.last_token == Previous::Abstract);
            if !declaration {
                self.push_block(matched);
            }
        } else if matches!(self.last_style, None | Some("operator"))
            && in_words(EXPRESSIONS, prefix)
        {
            self.push_block(matched);
        } else if matched == "end" {
            self.blocks.pop();
            self.current_indent -= 1;
        }
        let follow = match matched {
            "def" | "fun" => Follow::Ident,
            "macro" => Follow::Macro,
            "class" | "module" | "struct" | "lib" | "enum" | "union" => Follow::Type,
            _ => return,
        };
        self.scans.push(Scan::Follow(follow));
    }

    fn push_block(&mut self, matched: &str) {
        self.blocks.push(if matched == "lib" {
            Block::Lib
        } else {
            Block::Other
        });
        self.current_indent += 1;
    }

    fn follow(&mut self, stream: &mut StringStream<'_>, kind: Follow) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match kind {
            Follow::Type => {
                ident(stream, true);
            }
            Follow::Macro if ident(stream, false) => {
                if stream.current() == "def" {
                    return Some("keyword");
                }
                stream.eat_if(|ch| matches!(ch, '?' | '!'));
            }
            Follow::Ident => {
                if ident(stream, false) {
                    stream.eat_if(|ch| matches!(ch, '?' | '!'));
                } else {
                    symbol_op(stream);
                }
            }
            Follow::Macro => {}
        }
        self.scans.pop();
        Some("def")
    }

    fn percent(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let (delim, style, embed) = if take(stream, "%r") {
            (stream.next(), "string.special", true)
        } else if take(stream, "%w") || take(stream, "%q") {
            (stream.next(), "string", false)
        } else {
            stream.next();
            if let Some(ch) = stream.eat_if(|ch| !word(ch) && !js_space(ch) && ch != '=') {
                (Some(ch), "string", true)
            } else if stream
                .eat_if(|ch| ch.is_ascii_alphabetic() || ch == '_' || ch >= '\u{9f}')
                .is_some()
            {
                stream.eat_while(ident_tail);
                return Some("meta");
            } else {
                return Some("operator");
            }
        };
        let end = delim.map(|ch| match ch {
            '[' => ']',
            '{' => '}',
            '(' => ')',
            '<' => '>',
            _ => ch,
        });
        self.chain(stream, quote(end, style, embed))
    }

    fn quoted(
        &mut self,
        stream: &mut StringStream<'_>,
        end: Option<char>,
        style: &'static str,
        embed: bool,
    ) -> &'static str {
        while let Some(ch) = stream.peek() {
            if let Some(scan) = macro_at(remaining(stream)) {
                self.scans.push(scan);
                break;
            }
            if embed && remaining(stream).starts_with("#{") {
                self.scans.push(nest("#{", "}", Some("meta")));
                break;
            }
            stream.next();
            if Some(ch) == end {
                self.scans.pop();
                break;
            }
            if embed && ch == '\\' {
                stream.next();
            }
        }
        style
    }

    fn chain(&mut self, stream: &mut StringStream<'_>, scan: Scan) -> Option<&'static str> {
        self.scans.push(scan);
        self.read(stream)
    }
}

fn word_prefix(text: &str) -> &str {
    text.split(|ch| !word(ch)).next().unwrap_or_default()
}

fn ident_tail(ch: char) -> bool {
    word(ch) || ch >= '\u{9f}'
}

fn ident(stream: &mut StringStream<'_>, upper: bool) -> bool {
    let ascii = if upper { 'A'..='Z' } else { 'a'..='z' };
    if stream
        .eat_if(|ch| ascii.contains(&ch) || ch == '_' || ch >= '\u{9f}')
        .is_none()
    {
        return false;
    }
    stream.eat_while(ident_tail);
    true
}

fn symbol_op(stream: &mut StringStream<'_>) -> bool {
    stream.match_regex(&OPERATORS, true).is_some()
        || stream.match_regex(&CONDITIONAL, true).is_some()
        || stream.match_regex(&INDEXING, true).is_some()
}

fn number(stream: &mut StringStream<'_>) -> bool {
    if stream.eat('0').is_some() {
        match stream.peek() {
            Some('x') => {
                stream.next();
                stream.eat_while(|ch| ch.is_ascii_hexdigit() || ch == '_');
            }
            Some('o') => {
                stream.next();
                stream.eat_while(|ch| matches!(ch, '0'..='7' | '_'));
            }
            Some('b') => {
                stream.next();
                stream.eat_while(|ch| matches!(ch, '0' | '1' | '_'));
            }
            _ => {}
        }
    } else if stream.eat_if(|ch| ch.is_ascii_digit()).is_some() {
        stream.match_regex(&NUMBER_TAIL, true);
    } else {
        return false;
    }
    true
}

fn nest(open: &'static str, close: &'static str, style: Option<&'static str>) -> Scan {
    Scan::Nest {
        open,
        close,
        style,
        started: false,
    }
}

fn quote(end: Option<char>, style: &'static str, embed: bool) -> Scan {
    Scan::Quote { end, style, embed }
}

fn macro_at(text: &str) -> Option<Scan> {
    let (open, close) = if text.starts_with("{%") {
        ("{%", "%}")
    } else if text.starts_with("{{") {
        ("{{", "}}")
    } else {
        return None;
    };
    Some(Scan::Macro {
        open,
        close,
        started: false,
    })
}

pub(super) fn indent_on_input(text: &str) -> bool {
    text.ends_with([')', '}', ']'])
        || DEDENT
            .split_ascii_whitespace()
            .any(|word| text.ends_with(word))
}
