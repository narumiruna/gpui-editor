//! Lisp streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use regex::Regex;

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod clojure;
mod clojure_words;
mod common_lisp;
mod scheme;
mod scheme_words;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Clojure => "clojure",
        LegacyMode::CommonLisp => "commonlisp",
        LegacyMode::Scheme => "scheme",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let block = (mode == LegacyMode::CommonLisp).then_some(("#|", "|#"));
    let data = LanguageData::default()
        .with("closeBrackets", &["(", "[", "{", "\""][..])
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(";;"),
                block,
            },
        );
    if mode == LegacyMode::Clojure {
        data.with("autocomplete", clojure::autocomplete())
    } else {
        data
    }
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Clojure(clojure::State),
    CommonLisp(common_lisp::State),
    Scheme(scheme::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, _indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Clojure => Scan::Clojure(clojure::State::default()),
            LegacyMode::CommonLisp => Scan::CommonLisp(common_lisp::State::default()),
            LegacyMode::Scheme => Scan::Scheme(scheme::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Clojure(_) => LegacyMode::Clojure,
            Scan::CommonLisp(_) => LegacyMode::CommonLisp,
            Scan::Scheme(_) => LegacyMode::Scheme,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Clojure(state) => state.token(stream),
            Scan::CommonLisp(state) => state.token(stream),
            Scan::Scheme(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, _text_after: &str, _indent_unit: usize) -> Option<usize> {
        Some(match &self.0 {
            Scan::Clojure(state) => state.indent(),
            Scan::CommonLisp(state) => state.indent(),
            Scan::Scheme(state) => state.indent(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Space,
    Open,
    Close,
    Symbol,
    Other,
}

#[derive(Clone, Copy, Debug, Default)]
enum Resume {
    #[default]
    Base,
    String,
    Comment,
}

#[derive(Clone, Debug, Default)]
struct Contexts(Vec<Context>);

#[derive(Clone, Copy, Debug)]
struct Context {
    start: usize,
    indent: Align,
}

#[derive(Clone, Copy, Debug)]
enum Align {
    Unset,
    Next,
    Column(usize),
}

impl Contexts {
    fn start_line(&mut self, stream: &StringStream<'_>) {
        if stream.sol()
            && let Some(ctx) = self.0.last_mut()
            && !matches!(ctx.indent, Align::Column(_))
        {
            ctx.indent = Align::Column(ctx.start + 1);
        }
    }

    fn align(&mut self, stream: &StringStream<'_>, first: bool, body: bool) {
        if let Some(ctx) = self.0.last_mut() {
            ctx.indent = match ctx.indent {
                Align::Unset if first && body => {
                    Align::Column(ctx.start.saturating_add(stream.indent_unit()))
                }
                Align::Unset if first => Align::Next,
                Align::Next => Align::Column(stream.column()),
                indent => indent,
            };
        }
    }

    fn bracket(&mut self, stream: &StringStream<'_>, kind: Kind) {
        match kind {
            Kind::Open => self.0.push(Context {
                start: stream.column(),
                indent: Align::Unset,
            }),
            Kind::Close => {
                self.0.pop();
            }
            _ => {}
        }
    }

    fn indent(&self) -> usize {
        self.0.last().map_or(0, |ctx| match ctx.indent {
            Align::Column(column) => column,
            _ => ctx.start + 1,
        })
    }
}

fn until_unescaped(stream: &mut StringStream<'_>, end: char) -> bool {
    let mut escaped = false;
    for ch in stream {
        if ch == end && !escaped {
            return true;
        }
        escaped = !escaped && ch == '\\';
    }
    false
}

fn block_end(stream: &mut StringStream<'_>) -> bool {
    let mut bar = false;
    for ch in stream {
        if ch == '#' && bar {
            return true;
        }
        bar = ch == '|';
    }
    false
}

// ECMAScript whitespace for use inside regular expression character classes.
const SPACE: &str =
    r"\x09-\x0D\x20\u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}";

// Capture the required delimiter separately. Do not advance past it.
fn match_token(stream: &mut StringStream<'_>, pattern: &Regex) -> bool {
    let Some(captures) = stream.match_regex(pattern, false) else {
        return false;
    };
    let end = captures.get(1).unwrap().end();
    if end == 0 {
        return false;
    }
    stream.set_pos(stream.pos() + end).unwrap();
    true
}
