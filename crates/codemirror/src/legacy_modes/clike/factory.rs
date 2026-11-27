use std::{borrow::Cow, collections::HashSet, sync::Arc};

use regex::Regex;

use super::{State, common_data, indent_trigger};
use crate::{
    language::{LanguageData, StreamLanguage},
    stream_parser::{StreamParser, StringStream},
};

/// Case-sensitive word membership for the configurable C-like parser.
#[derive(Clone, Debug)]
pub enum LegacyClikeWords {
    /// Literal words. Autocomplete keeps their order and ignores duplicates
    /// within this list.
    List(Vec<String>),
    /// Test a whole token. Predicates do not contribute autocomplete entries.
    Predicate(fn(&str) -> bool),
}

impl Default for LegacyClikeWords {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

impl LegacyClikeWords {
    const EMPTY: Self = Self::List(Vec::new());

    pub(super) fn contains(&self, word: &str) -> bool {
        match self {
            Self::List(words) => words.iter().any(|candidate| candidate == word),
            Self::Predicate(test) => test(word),
        }
    }

    fn complete(&self) -> impl Iterator<Item = &str> {
        let words = match self {
            Self::List(words) => words.as_slice(),
            Self::Predicate(_) => &[],
        };
        let mut seen = HashSet::new();
        words
            .iter()
            .map(String::as_str)
            .filter(move |word| seen.insert(*word))
    }
}

/// Result of a character hook or a post-token hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyClikeHookResult {
    /// Use the base scanner or keep the existing token style.
    Continue,
    /// Return a token without a style.
    Unstyled,
    /// Return the given style.
    Style(&'static str),
}

impl LegacyClikeHookResult {
    pub(super) fn style(self, fallback: Option<&'static str>) -> Option<&'static str> {
        match self {
            Self::Continue => fallback,
            Self::Unstyled => None,
            Self::Style(style) => Some(style),
        }
    }
}

/// Scan a token after the leading character. Do not change the token start.
/// The cursor resets to its position after that character on `Continue`.
/// A handled token must include the leading character.
pub type LegacyClikeHook = fn(&mut StringStream<'_>) -> LegacyClikeHookResult;

/// Change a token's style after the definition check. As upstream, this does
/// not run for whitespace or tokens already classified as `comment` or `meta`.
pub type LegacyClikeTokenHook =
    fn(&StringStream<'_>, Option<&'static str>) -> LegacyClikeHookResult;

/// Native options for [`clike`]. Defaults match `clike({})`, not the C mode.
/// Pattern overrides use Rust [`Regex`]. Character patterns test one Unicode
/// scalar. A number pattern must match a nonempty prefix at the cursor.
#[derive(Clone, Debug)]
pub struct LegacyClikeConfig {
    /// Parser name. `None` leaves the mode unnamed.
    pub name: Option<String>,
    pub keywords: LegacyClikeWords,
    pub types: LegacyClikeWords,
    pub builtin: LegacyClikeWords,
    pub atoms: LegacyClikeWords,
    /// Words in `keywords` or `builtin` that start a statement context.
    pub block_keywords: LegacyClikeWords,
    /// Words in `keywords` that mark the next variable as a definition.
    pub def_keywords: LegacyClikeWords,
    /// Test otherwise unclassified words for the `builtin` style, before `atoms`.
    pub is_reserved_identifier: Option<fn(&str) -> bool>,
    pub is_punctuation_char: Option<Regex>,
    pub number_start: Option<Regex>,
    pub number: Option<Regex>,
    pub is_operator_char: Option<Regex>,
    pub is_identifier_char: Option<Regex>,
    /// Literal joiner for a qualified identifier. An empty string is ignored.
    pub namespace_separator: Option<String>,
    /// Keep an open single- or double-quoted string across lines even without
    /// a trailing backslash.
    pub multi_line_strings: bool,
    /// Mark a function definition after a type at top scope.
    pub type_first_definitions: bool,
    /// Return `variable` in place of `def`. Default: true (keep `def`).
    pub style_defs: bool,
    /// Preferred width for statements and unaligned call arguments. Zero
    /// uses the current indent unit.
    pub statement_indent_unit: usize,
    /// Use indentation instead of column alignment for call arguments.
    pub dont_align_calls: bool,
    /// Track statement indentation, not only delimiters. Default: true.
    pub indent_statements: bool,
    /// Include `case`, `default`, and `{}` in the `indentOnInput` trigger.
    /// This does not change switch block indentation. Default: true.
    pub indent_switch: bool,
    /// Skip statement contexts whose first token matches this pattern.
    pub dont_indent_statements: Option<Regex>,
    /// Align a line that begins with `{` or `(` to the nearest brace block
    /// or to the document root.
    pub allman_indentation: bool,
    /// Leading-character hooks. The first hook for a character is used.
    pub hooks: Vec<(char, LegacyClikeHook)>,
    pub token_hook: Option<LegacyClikeTokenHook>,
    /// Add or replace entries in the default language data.
    pub language_data: Option<fn(LanguageData) -> LanguageData>,
}

impl Default for LegacyClikeConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl LegacyClikeConfig {
    pub(super) const DEFAULT: Self = Self {
        name: None,
        keywords: LegacyClikeWords::EMPTY,
        types: LegacyClikeWords::EMPTY,
        builtin: LegacyClikeWords::EMPTY,
        atoms: LegacyClikeWords::EMPTY,
        block_keywords: LegacyClikeWords::EMPTY,
        def_keywords: LegacyClikeWords::EMPTY,
        is_reserved_identifier: None,
        is_punctuation_char: None,
        number_start: None,
        number: None,
        is_operator_char: None,
        is_identifier_char: None,
        namespace_separator: None,
        multi_line_strings: false,
        type_first_definitions: false,
        style_defs: true,
        statement_indent_unit: 0,
        dont_align_calls: false,
        indent_statements: true,
        indent_switch: true,
        dont_indent_statements: None,
        allman_indentation: false,
        hooks: Vec::new(),
        token_hook: None,
        language_data: None,
    };
}

/// Build a Rust stream parser based on `@codemirror/legacy-modes/mode/clike`.
/// This does not register a new concrete [`super::LegacyMode`].
///
/// There is no JavaScript coercion, object-key ordering, regex state, or mutable
/// parser state in hooks. Hooks cannot replace the continuation scanner, change
/// punctuation or context, or override the scanner's indent method. Use
/// [`StreamLanguage::with_indent`] to override document indentation. Namespace
/// separators are literals, not patterns. Empty number matches are ignored to
/// keep the scanner moving. Token ranges use UTF-8 bytes, columns count Unicode
/// scalars with tab expansion, and negative indentation is clamped to zero.
pub fn clike(config: LegacyClikeConfig) -> LegacyClike {
    LegacyClike(Arc::new(config))
}

/// A configurable legacy C-like stream parser.
#[derive(Clone, Debug)]
pub struct LegacyClike(Arc<LegacyClikeConfig>);

impl LegacyClike {
    /// Get a stream language with the comment markers, indentation trigger,
    /// and `autocomplete` (`Vec<String>`) from the configured word lists.
    pub fn language(&self) -> StreamLanguage<Self> {
        let config = &self.0;
        let words: Vec<String> = [
            &config.keywords,
            &config.types,
            &config.builtin,
            &config.atoms,
        ]
        .into_iter()
        .flat_map(LegacyClikeWords::complete)
        .map(str::to_owned)
        .collect();
        let data = common_data()
            .with("indentOnInput", indent_trigger(config.indent_switch))
            .with("autocomplete", words);
        let data = if let Some(override_data) = config.language_data {
            override_data(data)
        } else {
            data
        };
        StreamLanguage::new(self.clone()).with_language_data(data)
    }
}

/// A checkpoint in a configurable C-like stream. Use it with its parser.
#[derive(Clone, Debug)]
pub struct LegacyClikeState(State);

impl StreamParser for LegacyClike {
    type State = LegacyClikeState;

    fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }

    fn start_state(&self, indent_unit: usize) -> Self::State {
        LegacyClikeState(State::for_mode(None, indent_unit))
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        state.0.token_with(stream, &self.0).map(Cow::Borrowed)
    }

    fn indent(&self, state: &Self::State, text_after: &str, indent_unit: usize) -> Option<usize> {
        state.0.indent_with(text_after, indent_unit, &self.0)
    }
}
