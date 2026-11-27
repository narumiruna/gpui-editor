use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use super::{LegacyCssState, Rules, data_with_line_comment, scanner::Hooks};
use crate::{
    language::StreamLanguage,
    stream_parser::{StreamParser, StringStream},
};

/// Options for [`mk_css`]. The default uses the same scanner and keyword tables
/// as the bundled CSS mode, with no name.
#[derive(Clone, Debug)]
pub struct LegacyCssConfig {
    pub name: Option<String>,
    /// Start in declaration context, without an enclosing rule.
    pub inline: bool,
    /// Allow nested rules and property blocks. This does not select SCSS or Less hooks.
    pub allow_nested: bool,
    /// Enable the GSS `@component` block rules.
    pub supports_at_component: bool,
    /// Style known nonstandard properties as `string.special` instead of `property`.
    pub highlight_nonstandard_property_keywords: bool,
    /// Line comment metadata only. Set a token hook to scan line comments.
    pub line_comment: Option<&'static str>,
    pub keywords: LegacyCssKeywordOverrides,
    /// Replace the complete hook map. `None` keeps the default `/* ... */`
    /// hook. An empty map disables all hooks, including block comments.
    pub token_hooks: Option<BTreeMap<char, LegacyCssTokenHook>>,
}

impl Default for LegacyCssConfig {
    fn default() -> Self {
        Self {
            name: None,
            inline: false,
            allow_nested: false,
            supports_at_component: false,
            highlight_nonstandard_property_keywords: true,
            line_comment: None,
            keywords: LegacyCssKeywordOverrides::default(),
            token_hooks: None,
        }
    }
}

/// Replace individual keyword tables. `None` keeps the upstream table; an
/// empty set removes every entry. Keys are literal and are not normalized.
/// The scanner lowercases its lookup words except for `document_types`, which
/// uses the token text as written. These tables do not change autocomplete.
#[derive(Clone, Debug, Default)]
pub struct LegacyCssKeywordOverrides {
    pub document_types: Option<HashSet<String>>,
    pub media_types: Option<HashSet<String>>,
    pub media_features: Option<HashSet<String>>,
    pub media_value_keywords: Option<HashSet<String>>,
    pub property_keywords: Option<HashSet<String>>,
    pub nonstandard_property_keywords: Option<HashSet<String>>,
    pub font_properties: Option<HashSet<String>>,
    pub counter_descriptors: Option<HashSet<String>>,
    pub color_keywords: Option<HashSet<String>>,
    pub value_keywords: Option<HashSet<String>>,
}

/// A native token hook. It receives the stream just after the trigger character.
/// Return `None` to run the base scanner from that point, discarding any cursor
/// movement by the hook. Do not reset the token start.
pub type LegacyCssTokenHook = fn(&mut StringStream<'_>) -> Option<LegacyCssHookToken>;

/// How the CSS parser interprets the token from a hook. The CSS context may
/// replace the token's style based on this kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyCssTokenKind {
    Other,
    Comment,
    /// Classify the token text as an at-rule if it starts with `@`.
    AtRule,
    /// A CSS punctuation token, such as `:` or `{`.
    Mark(char),
    Hash,
    Word,
    Qualifier,
    Meta,
    Variable,
    VariableDefinition,
    Interpolation,
}

/// The style and parser kind returned by a native token hook.
#[derive(Clone, Copy, Debug)]
pub struct LegacyCssHookToken {
    pub kind: LegacyCssTokenKind,
    pub style: Option<&'static str>,
    continuation: Option<fn(&mut StringStream<'_>) -> Self>,
}

impl LegacyCssHookToken {
    pub fn new(kind: LegacyCssTokenKind, style: Option<&'static str>) -> Self {
        Self {
            kind,
            style,
            continuation: None,
        }
    }

    /// Use this scanner function for the next token, before the base scanner,
    /// whitespace handling, or hook lookup. It must consume text or return a
    /// token without a continuation so the next call can use the base scanner.
    /// Return another continuation to keep scanning across tokens or lines.
    pub fn with_continuation(mut self, next: fn(&mut StringStream<'_>) -> Self) -> Self {
        self.continuation = Some(next);
        self
    }

    pub(super) fn continuation(self) -> Option<fn(&mut StringStream<'_>) -> Self> {
        self.continuation
    }
}

/// Build a configurable legacy CSS stream parser from `@codemirror/legacy-modes/mode/css`.
/// This is separate from the modern CSS parser and concrete legacy mode lookup.
///
/// Hooks are Rust functions with static styles. They cannot capture data,
/// mutate private parser state, or inherit the previous token kind by omitting
/// a kind. They can select a continuation function to scan across lines.
/// The factory does not emulate JavaScript object prototypes or coercions.
/// State and token history are local to each stream. Ranges use UTF-8 bytes,
/// and columns count Unicode scalars, not UTF-16 code units.
pub fn mk_css(config: LegacyCssConfig) -> LegacyCss {
    LegacyCss(Arc::new(config))
}

/// The native configurable legacy CSS parser.
#[derive(Clone, Debug)]
pub struct LegacyCss(Arc<LegacyCssConfig>);

impl LegacyCss {
    /// Get a stream language with the factory's comment metadata, indentation
    /// trigger, and unchanged [`super::CSS_KEYWORDS`] autocomplete list.
    pub fn language(&self) -> StreamLanguage<Self> {
        StreamLanguage::new(self.clone())
            .with_language_data(data_with_line_comment(self.0.line_comment))
    }
}

impl StreamParser for LegacyCss {
    type State = LegacyCssState;

    fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }

    fn start_state(&self, _indent_unit: usize) -> Self::State {
        LegacyCssState::new(self.0.inline)
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        let config = &self.0;
        let rules = Rules {
            hooks: config
                .token_hooks
                .as_ref()
                .map_or(Hooks::Css, Hooks::Custom),
            keywords: &config.keywords,
            allow_nested: config.allow_nested,
            supports_at_component: config.supports_at_component,
            highlight_nonstandard: config.highlight_nonstandard_property_keywords,
        };
        state.token(stream, rules).map(Cow::Borrowed)
    }

    fn indent(&self, state: &Self::State, text_after: &str, indent_unit: usize) -> Option<usize> {
        state.indent(text_after, indent_unit)
    }
}
