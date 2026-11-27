use crate::{legacy_modes::js_space, stream_parser::StringStream};

macro_rules! pattern {
    ($source:literal) => {
        pattern!($source, "")
    };
    ($source:literal, $flags:literal) => {{
        static REGEX: std::sync::LazyLock<regress::Regex> =
            std::sync::LazyLock::new(|| regress::Regex::with_flags($source, $flags).unwrap());
        &*REGEX
    }};
}

mod parser;
mod scanner;
mod words;

pub(super) use words::AUTOCOMPLETE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Block,
    Parens,
    VendorPrefixes,
    Pseudo,
    AtBlock,
    AtBlockParens,
    Keyframes,
    Interpolation,
    Extend,
    VariableName,
}

#[derive(Clone, Debug)]
struct Context {
    phase: Phase,
    indent: usize,
    line_indent: usize,
    first_word: String,
}

impl Context {
    fn new(phase: Phase, indent: usize) -> Self {
        Self {
            phase,
            indent,
            line_indent: 0,
            first_word: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::legacy_modes) struct State {
    scanner: scanner::Scanner,
    phase: Phase,
    contexts: Vec<Context>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scanner: scanner::Scanner::Base,
            phase: Phase::Block,
            contexts: vec![Context::new(Phase::Block, 0)],
        }
    }
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if self.scanner == scanner::Scanner::Base && stream.eat_space() {
            return None;
        }
        let scanner::Token(kind, mut style) = self.scan(stream);
        self.phase = self.parse(self.phase, kind, stream, &mut style);
        style
    }

    pub(super) fn indent(&self, text: &str, unit: usize) -> usize {
        let mut contexts = self.contexts.iter().rev();
        let cx = contexts.next().unwrap();
        let previous = contexts.next();
        let ch = text.chars().next();
        let word = first_word(text);
        let line_indent = cx.line_indent;
        let prev_word = previous.map_or("", |cx| cx.first_word.as_str());
        let prev_indent = previous.map_or(line_indent, |cx| cx.line_indent);
        let selector = |ch| ".#:[*&>~+/".contains(ch);
        let prev_relative_indent = if line_indent <= prev_indent {
            prev_indent
        } else {
            prev_indent + unit
        };

        if previous.is_some()
            && matches!(
                (ch, cx.phase),
                (Some('}'), Phase::Block | Phase::AtBlock | Phase::Keyframes)
                    | (Some(')'), Phase::Parens | Phase::AtBlockParens)
            )
        {
            return cx.indent.saturating_sub(unit);
        }
        if ch == Some('}') {
            return cx.indent;
        }
        let trimmed = text.trim_start_matches(js_space);
        if ch.is_some_and(|ch| matches!(ch, '@' | '$' | '{') || ch.is_ascii_digit())
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || prev_word.trim_start_matches(js_space).starts_with("/*")
            || pattern!(r#"^\s*[\w.\[\]'"-]+\s*[?:+]?="#)
                .find(text)
                .is_some()
            || pattern!(r"^[+-]?[a-z][\w-]*\(", "i").find(text).is_some()
            || text.starts_with("return")
            || is_block(word)
        {
            return line_indent;
        }
        if ch.is_some_and(selector) || is_tag(word) {
            return if prev_word.trim_end_matches(js_space).ends_with(',') {
                prev_indent
            } else if prev_word.contains(selector) || is_tag(prev_word) {
                prev_relative_indent
            } else {
                line_indent
            };
        }
        if !text.trim_end_matches(js_space).ends_with(',') && (is_vendor(word) || is_property(word))
        {
            return if is_block(prev_word) {
                prev_relative_indent
            } else if prev_word.starts_with('{') {
                if line_indent <= prev_indent {
                    line_indent
                } else {
                    prev_indent + unit
                }
            } else if is_vendor(prev_word) || is_property(prev_word) {
                prev_indent.min(line_indent)
            } else if prev_word
                .starts_with(['.', '#', ':', '[', '*', '&', '@', '+', '-', '>', '~', '/'])
                || prev_word.trim_end_matches(js_space).ends_with('=')
                || is_tag(prev_word)
                || pattern!(r#"^\$[\w.\[\]'"-]"#).find(prev_word).is_some()
            {
                prev_indent + unit
            } else {
                line_indent
            };
        }
        cx.indent
    }

    fn context(&self) -> &Context {
        self.contexts.last().unwrap()
    }

    fn push(&mut self, phase: Phase, stream: &StringStream<'_>, indent: usize) -> Phase {
        self.contexts
            .push(Context::new(phase, stream.indentation() + indent));
        phase
    }

    fn pop(&mut self) -> Phase {
        if self.contexts.len() > 1 {
            self.contexts.pop();
        }
        self.context().phase
    }
}

fn rest<'a>(stream: &StringStream<'a>) -> &'a str {
    &stream.line()[stream.pos()..]
}

fn match_pattern(stream: &mut StringStream<'_>, regex: &regress::Regex) -> bool {
    let Some(matched) = regex
        .find(rest(stream))
        .filter(|matched| matched.start() == 0)
    else {
        return false;
    };
    stream.set_pos(stream.pos() + matched.end()).unwrap();
    true
}

fn first_word(line: &str) -> &str {
    pattern!(r"^\s*[-_]*[a-z0-9]+[\w-]*", "i")
        .find(line)
        .map_or("", |matched| {
            line[matched.range()].trim_start_matches(js_space)
        })
}

fn start_of_line(stream: &StringStream<'_>) -> bool {
    let mut line = stream.line();
    loop {
        if line.starts_with(stream.current()) {
            return true;
        }
        let Some(ch) = line.chars().next().filter(|&ch| js_space(ch)) else {
            return false;
        };
        line = &line[ch.len_utf8()..];
    }
}

fn end_of_line(stream: &StringStream<'_>) -> bool {
    rest(stream).chars().all(js_space)
}

fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

// The upstream `in` checks accept these inherited keys after lowercasing.
fn inherited_word(word: &str) -> bool {
    word.eq_ignore_ascii_case("constructor") || word == "__proto__"
}

fn is_tag(word: &str) -> bool {
    inherited_word(word) || words::TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(word))
}

fn is_property(word: &str) -> bool {
    inherited_word(word)
        || words::PROPERTIES
            .iter()
            .chain(words::FONTS)
            .any(|property| property.eq_ignore_ascii_case(word))
}

fn is_block(word: &str) -> bool {
    inherited_word(word)
        || words::BLOCK
            .iter()
            .any(|name| name.eq_ignore_ascii_case(word))
}

fn is_vendor(word: &str) -> bool {
    vendor_pattern().find(word).is_some()
}

fn vendor_pattern() -> &'static regress::Regex {
    pattern!(r"^-(?:moz|ms|o|webkit)-", "i")
}
