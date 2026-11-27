use std::sync::LazyLock;

use regex::Regex;

use super::super::{CSS_KEYWORDS, js_space};
use crate::stream_parser::StringStream;

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-?[0-9.]+").unwrap());
static UNIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:^(?:px|em|in)\b)").unwrap());
static KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:true|false|null|auto)").unwrap());
static OPERATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[()=><+\-]|!=|[/*%]|and|or|not|[;{}:])").unwrap());
static VENDOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:^-\w+-)").unwrap());
static MIXIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:^[=+][\w-]+)").unwrap());
static DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@(?:else if|if|media|else|for|each|while|mixin|function)").unwrap()
});
static PSEUDO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:^::?[a-zA-Z_][\w-]*)").unwrap());
static PROPERTY_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?-u:^ *: *[\w+$#!("'-])"#).unwrap());
static COLOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[0-9a-fA-F]{6}|[0-9a-fA-F]{3})").unwrap());

#[derive(Clone, Copy, Debug)]
struct Quoted {
    end: char,
    greedy: bool,
}

#[derive(Clone, Copy, Debug)]
enum Scanner {
    Base,
    Url,
    Comment { indentation: usize, block: bool },
    String(Quoted),
    Interpolation(Option<Quoted>),
}

#[derive(Clone, Debug)]
pub(in crate::legacy_modes) struct State {
    scanner: Scanner,
    scopes: Vec<usize>,
    previous_property: String,
    indented: bool,
    in_value: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scanner: Scanner::Base,
            scopes: vec![0],
            previous_property: String::new(),
            indented: false,
            in_value: false,
        }
    }
}

impl State {
    pub(super) fn indent(&self) -> usize {
        *self.scopes.last().unwrap()
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.indented = false;
        }
        let style = match self.scanner {
            Scanner::Base => self.base(stream),
            Scanner::Url => self.url(stream),
            Scanner::Comment { indentation, block } => self.comment(stream, indentation, block),
            Scanner::String(quoted) => self.string(stream, quoted),
            Scanner::Interpolation(quoted) => {
                if stream.eat('}').is_some() {
                    self.scanner = quoted.map_or(Scanner::Base, Scanner::String);
                    Some("operator")
                } else {
                    self.base(stream)
                }
            }
        };
        if matches!(stream.current(), "@return" | "}") {
            self.dedent();
        }
        if style.is_some() {
            // Upstream compares the raw UTF-16 offset, without expanding tabs.
            let start = stream.line()[..stream.start()].encode_utf16().count();
            let offset = start + usize::from(self.indented) * stream.indent_unit();
            while self.indent() > offset {
                self.scopes.pop();
            }
        }
        style
    }

    fn url(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat(')').is_some() {
            self.scanner = Scanner::Base;
            Some("operator")
        } else if stream.eat('(').is_some() {
            stream.eat_space();
            Some("operator")
        } else {
            // For an unquoted URL, this call changes the scanner but does not advance.
            let quote = stream.eat_if(is_quote);
            self.scanner = Scanner::String(Quoted {
                end: quote.unwrap_or(')'),
                greedy: quote.is_some(),
            });
            Some("string")
        }
    }

    fn comment(
        &mut self,
        stream: &mut StringStream<'_>,
        indentation: usize,
        block: bool,
    ) -> Option<&'static str> {
        if stream.sol() && stream.indentation() <= indentation {
            self.scanner = Scanner::Base;
            return self.base(stream);
        }
        if block && stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.scanner = Scanner::Base;
        } else {
            stream.skip_to_end();
        }
        Some("comment")
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quoted: Quoted) -> Option<&'static str> {
        let escaped = stream.line()[..stream.pos()].ends_with('\\');
        let ch = stream.next();
        let peek = stream.peek();
        let end = Some(quoted.end);
        if ch != Some('\\') && peek == end || ch == end && !escaped {
            if ch != end && quoted.greedy {
                stream.next();
            }
            self.finish_value(stream);
            self.scanner = Scanner::Base;
            Some("string")
        } else if ch == Some('#') && stream.eat('{').is_some() {
            self.scanner = Scanner::Interpolation(Some(quoted));
            Some("operator")
        } else {
            Some("string")
        }
    }

    fn push_indent(&mut self, stream: &StringStream<'_>) {
        if !self.indented {
            self.scopes.push(self.indent() + stream.indent_unit());
            self.indented = true;
        }
    }

    fn dedent(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.peek();
        if let Some(block) = [("/*", true), ("//", false)]
            .into_iter()
            .find_map(|(prefix, block)| stream.match_str(prefix, true, false).then_some(block))
        {
            let indentation = stream.indentation();
            self.scanner = Scanner::Comment { indentation, block };
            return self.comment(stream, indentation, block);
        }
        if stream.match_str("#{", true, false) {
            self.scanner = Scanner::Interpolation(None);
            return Some("operator");
        }
        if let Some(end) = stream.eat_if(is_quote) {
            self.scanner = Scanner::String(Quoted { end, greedy: true });
            return Some("string");
        }
        let in_value = self.in_value;
        let style = if in_value {
            self.value(stream, ch)
        } else {
            self.selector(stream, ch)
        };
        if style.is_some() || in_value && end_of_line(stream) {
            return style;
        }
        if stream.match_regex(&OPERATOR, true).is_some() {
            return Some("operator");
        }
        stream.next();
        None
    }

    fn selector(
        &mut self,
        stream: &mut StringStream<'_>,
        ch: Option<char>,
    ) -> Option<&'static str> {
        if ch == Some('-') && stream.match_regex(&VENDOR, true).is_some() {
            return Some("meta");
        }
        if let Some(prefix @ ('.' | '#')) = ch {
            stream.next();
            if stream.eat_while(word_hyphen) {
                self.push_indent(stream);
                return Some(if prefix == '.' {
                    "qualifier"
                } else {
                    "builtin"
                });
            }
            if stream.peek() == Some('#') {
                self.push_indent(stream);
                return Some("tag");
            }
        }
        if ch == Some('$') {
            stream.next();
            stream.eat_while(word_hyphen);
            return Some("variable-2");
        }
        if let Some(style) = self.literal(stream) {
            return Some(style);
        }
        if matches!(ch, Some('=' | '+')) && stream.match_regex(&MIXIN, true).is_some() {
            if ch == Some('=') {
                self.push_indent(stream);
            }
            return Some("meta");
        }
        if ch == Some('@') && stream.match_str("@extend", true, false) {
            let tail = remainder(stream).trim_start_matches(js_space);
            if tail.chars().next().is_some_and(word) {
                stream
                    .set_pos(stream.line().len() - tail.len() + 1)
                    .unwrap();
            } else {
                self.dedent();
            }
        }
        if stream.match_regex(&DIRECTIVE, true).is_some() {
            self.push_indent(stream);
            return Some("def");
        }
        if ch == Some('@') {
            stream.next();
            stream.eat_while(word_hyphen);
            return Some("def");
        }
        if stream.eat_while(word_hyphen) {
            let word = stream.current().to_lowercase();
            if stream.match_regex(&PROPERTY_VALUE, false).is_some() {
                let nested = format!("{}-{word}", self.previous_property);
                return Some(if CSS_KEYWORDS.properties.contains(&nested.as_str()) {
                    "property"
                } else if CSS_KEYWORDS.properties.contains(&word.as_str()) {
                    self.previous_property = word;
                    "property"
                } else if CSS_KEYWORDS.fonts.contains(&word.as_str()) {
                    "property"
                } else {
                    "tag"
                });
            }
            let tail = remainder(stream).trim_start_matches(' ');
            if tail.starts_with(':') {
                self.push_indent(stream);
                self.in_value = true;
                self.previous_property = word;
                return Some("property");
            }
            if !tail.starts_with(',') {
                self.push_indent(stream);
            }
            return Some("tag");
        }
        if ch == Some(':') {
            if stream.match_regex(&PSEUDO, true).is_some() {
                return Some("type");
            }
            stream.next();
            self.in_value = true;
            return Some("operator");
        }
        None
    }

    fn value(&mut self, stream: &mut StringStream<'_>, ch: Option<char>) -> Option<&'static str> {
        let style = if ch == Some('#') && {
            stream.next();
            stream.match_regex(&COLOR, true).is_some()
        } {
            "number"
        } else if let Some(style) = self.literal(stream) {
            style
        } else if ch == Some('$') {
            stream.next();
            stream.eat_while(word_hyphen);
            "variable-2"
        } else if ch == Some('!') {
            stream.next();
            self.in_value = false;
            return Some(if stream.eat_while(word) {
                "keyword"
            } else {
                "operator"
            });
        } else if stream.match_regex(&OPERATOR, true).is_some() {
            "operator"
        } else if stream.eat_while(word_hyphen) {
            let word = stream.current().to_lowercase();
            if CSS_KEYWORDS.values.contains(&word.as_str()) {
                "atom"
            } else if CSS_KEYWORDS.colors.contains(&word.as_str()) {
                "keyword"
            } else if CSS_KEYWORDS.properties.contains(&word.as_str()) {
                self.previous_property = word;
                "property"
            } else {
                "tag"
            }
        } else {
            self.finish_value(stream);
            return None;
        };
        self.finish_value(stream);
        Some(style)
    }

    fn literal(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        for (pattern, style) in [
            (&*NUMBER, "number"),
            (&*UNIT, "unit"),
            (&*KEYWORD, "keyword"),
        ] {
            if stream.match_regex(pattern, true).is_some() {
                return Some(style);
            }
        }
        if stream.match_str("url", true, false) && stream.peek() == Some('(') {
            self.scanner = Scanner::Url;
            return Some("atom");
        }
        None
    }

    fn finish_value(&mut self, stream: &StringStream<'_>) {
        if end_of_line(stream) {
            self.in_value = false;
        }
    }
}

fn remainder<'a>(stream: &StringStream<'a>) -> &'a str {
    &stream.line()[stream.pos()..]
}

fn end_of_line(stream: &StringStream<'_>) -> bool {
    remainder(stream).chars().all(js_space)
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn word_hyphen(ch: char) -> bool {
    word(ch) || ch == '-'
}

fn is_quote(ch: char) -> bool {
    matches!(ch, '"' | '\'')
}
