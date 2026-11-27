//! SQL streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use std::{borrow::Cow, collections::HashSet, sync::Arc};

use super::LegacyMode;
use crate::{
    commands::CommentTokens,
    language::{LanguageData, StreamLanguage},
    stream_parser::{StreamParser, StringStream},
};

mod scanner;
mod specs;

use scanner::Scanner;

/// Optional syntax supported by the legacy SQL factory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LegacySqlSupport {
    pub odbc_dot_table: bool,
    pub decimalless_float: bool,
    pub zeroless_float: bool,
    pub binary_number: bool,
    pub hex_number: bool,
    pub double_quote: bool,
    pub n_char_cast: bool,
    pub charset_cast: bool,
    pub escape_constant: bool,
    pub comment_hash: bool,
    pub comment_slash_slash: bool,
    pub comment_space_required: bool,
}

/// Result of a leading-character hook. A hook receives the stream after the
/// leading character and can consume more input, including when it falls through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacySqlHookResult {
    Continue,
    Unstyled,
    Style(&'static str),
}

/// A native hook for the base scanner. It cannot replace the scanner state.
pub type LegacySqlHook = fn(&mut StringStream<'_>) -> LegacySqlHookResult;

/// Native options for [`sql`]. Word tables are space-separated and are matched
/// against the lowercased input. An empty table disables that word class.
/// Operator, bracket, and punctuation tables specify literal characters, not
/// JavaScript regular expressions. The first hook for a character is used.
#[derive(Clone, Debug)]
pub struct LegacySqlConfig {
    pub client: Cow<'static, str>,
    pub atoms: Cow<'static, str>,
    pub builtin: Cow<'static, str>,
    pub keywords: Cow<'static, str>,
    pub operator_chars: Cow<'static, str>,
    pub support: LegacySqlSupport,
    pub hooks: Vec<(char, LegacySqlHook)>,
    pub date_sql: Cow<'static, str>,
    pub backslash_string_escapes: bool,
    pub brackets: Cow<'static, str>,
    pub punctuation: Cow<'static, str>,
}

impl Default for LegacySqlConfig {
    fn default() -> Self {
        Self {
            client: "".into(),
            atoms: "false true null".into(),
            builtin: specs::DEFAULT_BUILTIN.into(),
            keywords: specs::DEFAULT_KEYWORDS.into(),
            operator_chars: "*+-%<>!=&|~^/".into(),
            support: LegacySqlSupport::default(),
            hooks: Vec::new(),
            date_sql: "date time timestamp".into(),
            backslash_string_escapes: true,
            brackets: "{}()[]".into(),
            punctuation: ";.,:".into(),
        }
    }
}

/// Build a configurable legacy SQL stream parser. This is separate from the
/// modern `@codemirror/lang-sql` parser.
pub fn sql(config: LegacySqlConfig) -> LegacySql {
    LegacySql(Arc::new(Spec {
        client: words(config.client),
        atoms: words(config.atoms),
        builtin: words(config.builtin),
        keywords: words(config.keywords),
        operator_chars: config.operator_chars,
        support: config.support,
        hooks: config.hooks,
        date_sql: words(config.date_sql),
        backslash_string_escapes: config.backslash_string_escapes,
        brackets: config.brackets,
        punctuation: config.punctuation,
    }))
}

type Words = HashSet<Cow<'static, str>>;

fn words(list: Cow<'static, str>) -> Words {
    match list {
        Cow::Borrowed(list) => list
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(Cow::Borrowed)
            .collect(),
        Cow::Owned(list) => list
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(|s| Cow::Owned(s.to_owned()))
            .collect(),
    }
}

#[derive(Debug)]
struct Spec {
    client: Words,
    atoms: Words,
    builtin: Words,
    keywords: Words,
    operator_chars: Cow<'static, str>,
    support: LegacySqlSupport,
    hooks: Vec<(char, LegacySqlHook)>,
    date_sql: Words,
    backslash_string_escapes: bool,
    brackets: Cow<'static, str>,
    punctuation: Cow<'static, str>,
}

/// The native configurable legacy SQL parser.
#[derive(Clone, Debug)]
pub struct LegacySql(Arc<Spec>);

impl LegacySql {
    /// Get a stream language with this parser's comment and bracket data.
    pub fn language(&self) -> StreamLanguage<Self> {
        StreamLanguage::new(self.clone()).with_language_data(self.0.data())
    }
}

impl Spec {
    fn data(&self) -> LanguageData {
        let line = if self.support.comment_slash_slash {
            "//"
        } else if self.support.comment_hash {
            "#"
        } else {
            "--"
        };
        LanguageData::default()
            .with(
                "commentTokens",
                CommentTokens {
                    line: Some(line),
                    block: Some(("/*", "*/")),
                },
            )
            .with("closeBrackets", &["(", "[", "{", "'", "\"", "`"][..])
    }
}

/// A checkpoint in a legacy SQL stream.
#[derive(Clone, Debug, Default)]
pub struct LegacySqlState {
    scanner: Scanner,
    contexts: Vec<Context>,
}

#[derive(Clone, Copy, Debug)]
struct Context {
    closing: &'static str,
    indent: usize,
    column: usize,
    align: Option<bool>,
}

impl LegacySqlState {
    fn token(&mut self, spec: &Spec, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol()
            && let Some(context) = self.contexts.last_mut()
        {
            context.align.get_or_insert(false);
        }
        if self.scanner.is_base() && stream.eat_space() {
            return None;
        }
        let style = self.scanner.token(spec, stream);
        if style == Some("comment") {
            return style;
        }
        if let Some(context) = self.contexts.last_mut() {
            context.align.get_or_insert(true);
        }
        if let Some(closing) = match stream.current() {
            "(" => Some(")"),
            "[" => Some("]"),
            _ => None,
        } {
            self.contexts.push(Context {
                closing,
                indent: stream.indentation(),
                column: stream.column(),
                align: None,
            });
        } else if self
            .contexts
            .last()
            .is_some_and(|context| stream.current() == context.closing)
        {
            self.contexts.pop();
        }
        style
    }

    fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        let context = self.contexts.last()?;
        let closing = text_after.starts_with(context.closing);
        if context.align == Some(true) {
            Some(context.column.saturating_add(usize::from(!closing)))
        } else {
            Some(
                context
                    .indent
                    .saturating_add(if closing { 0 } else { indent_unit }),
            )
        }
    }
}

impl StreamParser for LegacySql {
    type State = LegacySqlState;

    fn name(&self) -> Option<&str> {
        Some("sql")
    }

    fn start_state(&self, _indent_unit: usize) -> Self::State {
        LegacySqlState::default()
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        state.token(&self.0, stream).map(Cow::Borrowed)
    }

    fn indent(&self, state: &Self::State, text_after: &str, indent_unit: usize) -> Option<usize> {
        state.indent(text_after, indent_unit)
    }
}

pub(super) fn name(_mode: LegacyMode) -> Option<&'static str> {
    Some("sql")
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    specs::parser(mode).0.data()
}

#[derive(Clone, Debug)]
pub(super) struct State {
    mode: LegacyMode,
    inner: LegacySqlState,
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self {
            mode,
            inner: LegacySqlState::default(),
        }
    }

    pub(super) fn mode(&self) -> LegacyMode {
        self.mode
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        self.inner.token(&specs::parser(self.mode).0, stream)
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        self.inner.indent(text_after, indent_unit)
    }
}
