use std::{borrow::Cow, sync::Arc};

use regex::Regex;

use super::{LegacyMode, data, python};
use crate::{
    language::StreamLanguage,
    stream_parser::{StreamParser, StringStream},
};

/// Native options for [`mk_python`]. The default uses Python 3 syntax.
///
/// The lexical overrides use Rust [`Regex`], not JavaScript regular expressions.
/// Only nonempty matches at the cursor count. JavaScript-specific regex syntax,
/// flags, and coercion are not supported. Extra words are literal, case-sensitive
/// strings followed by an ASCII word boundary, not regex fragments. Empty words
/// are ignored.
#[derive(Clone, Debug, Default)]
pub struct LegacyPythonConfig {
    /// Versions 1 and 2 select the Python 2 rules. `None`, zero, and versions
    /// 3 and later select the Python 3 rules.
    pub version: Option<u32>,
    /// Replace the default identifier pattern, including decorator lookahead.
    pub identifiers: Option<Regex>,
    /// Add words to the keyword table.
    pub extra_keywords: Vec<String>,
    /// Add words to the built-in name table.
    pub extra_builtins: Vec<String>,
    /// Replace the standard operator pattern. The four historical patterns
    /// below are tried first, in field order, and produce operator tokens.
    pub operators: Option<Regex>,
    pub single_operators: Option<Regex>,
    pub double_operators: Option<Regex>,
    pub double_delimiters: Option<Regex>,
    pub triple_delimiters: Option<Regex>,
    /// Replace the standard punctuation pattern.
    pub delimiters: Option<Regex>,
    /// A fallback for `delimiters`, used only if it is not set.
    pub single_delimiters: Option<Regex>,
    /// Set bracket indentation. `None` and zero use the current indent unit.
    pub hanging_indent: Option<usize>,
    /// Mark an unterminated single-line string as an error and keep its string
    /// state on the next line, as the upstream factory does.
    pub single_line_string_errors: bool,
}

impl LegacyPythonConfig {
    pub(super) fn python3(&self) -> bool {
        !matches!(self.version, Some(1 | 2))
    }

    pub(super) fn hanging_indent(&self, unit: usize) -> usize {
        self.hanging_indent
            .filter(|&width| width > 0)
            .unwrap_or(unit)
    }
}

/// Build a legacy Python stream parser from `@codemirror/legacy-modes/mode/python`.
/// This is separate from the modern Python parser and from the concrete legacy
/// mode lookup. Tokens use UTF-8 byte ranges and columns count Unicode scalars
/// (with tab expansion). Indentation cannot be negative.
pub fn mk_python(config: LegacyPythonConfig) -> LegacyPython {
    LegacyPython(Arc::new(config))
}

/// A configurable legacy Python stream parser.
#[derive(Clone, Debug)]
pub struct LegacyPython(Arc<LegacyPythonConfig>);

impl LegacyPython {
    /// Get a stream language with the default Python metadata. Custom word
    /// tables affect tokenization, not the language's autocomplete list.
    pub fn language(&self) -> StreamLanguage<Self> {
        StreamLanguage::new(self.clone()).with_language_data(data(LegacyMode::Python))
    }
}

/// A checkpoint in a configurable legacy Python stream.
#[derive(Clone, Debug)]
pub struct LegacyPythonState(python::State);

impl StreamParser for LegacyPython {
    type State = LegacyPythonState;

    fn name(&self) -> Option<&str> {
        Some("python")
    }

    fn start_state(&self, _indent_unit: usize) -> Self::State {
        LegacyPythonState(python::State::new(LegacyMode::Python))
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        state
            .0
            .token_with_config(stream, &self.0)
            .map(Cow::Borrowed)
    }

    fn indent(&self, state: &Self::State, text_after: &str, indent_unit: usize) -> Option<usize> {
        state
            .0
            .indent(text_after, self.0.hanging_indent(indent_unit))
    }
}
