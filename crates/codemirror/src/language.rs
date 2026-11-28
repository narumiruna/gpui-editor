//! Stream language services based on `@codemirror/language` 6.13.1.
//! See `docs/licenses/codemirror-language.txt` for the upstream MIT notice.
//!
//! This adapter works on one native [`Text`] document. Positions are UTF-8
//! byte offsets. Logical lines do not depend on display wrapping.

use std::{any::Any, collections::HashMap, fmt, num::NonZeroUsize, ops::Range, sync::Arc};

use crate::{
    Assoc, ChangeSet, EditError, Text,
    stream_parser::{StreamParser, StringStream, TokenizeError, read_token},
};

pub mod angular;
mod bidi;
pub mod brackets;
pub mod cpp;
pub mod css;
mod data_at;
pub mod fold;
pub mod go;
mod highlight;
pub mod html;
mod indent;
mod indent_service;
pub mod java;
pub mod javascript;
pub mod jinja;
pub mod json;
pub mod less;
pub mod lezer;
pub mod lezer_grammar;
pub(crate) mod line_projection;
pub mod liquid;
pub mod markdown;
pub mod parse_service;
pub mod php;
pub mod python;
pub mod rust;
pub mod sass;
pub mod sql;
pub mod vue;
pub mod wast;
pub mod xml;
pub mod yaml;

pub use bidi::{BidiIsolateOptions, bidi_isolates, configured_bidi_isolates};
pub use data_at::language_data_provider;
pub use highlight::{
    Color, CustomTagModifier, DEFAULT_HIGHLIGHT_STYLE, HighlightContext, HighlightLayer,
    HighlightProperties, HighlightResets, HighlightRule, HighlightSelector, HighlightStyle,
    HighlightTag, HighlightTheme, TagModifier, TextDecoration, TokenTable, highlighting_for,
    highlighting_for_context, highlighting_for_native,
};
pub use indent::{IndentContext, IndentOptions};
pub(crate) use indent_service::style as indent_style;
pub use indent_service::{
    INDENT_UNIT, IndentDecision, IndentStyle, MarkdownCodeIndent, MarkdownWastIndent,
    get_indent_unit, get_indentation, indent_range, indent_service, indent_string,
};
pub use parse_service::{
    ParseJob, ParseRequest, ParseService, ParseSkip, ParseSnapshot, ParseStep, whole_document,
};

use highlight::Style;
use indent::indent_text;

/// Default data attached to a stream mode for the entire document.
#[derive(Clone, Default)]
pub struct LanguageData(HashMap<String, Arc<dyn Any + Send + Sync>>);

impl LanguageData {
    /// Set a named value, replacing the previous value for that name.
    pub fn with(mut self, name: impl Into<String>, value: impl Any + Send + Sync) -> Self {
        self.0.insert(name.into(), Arc::new(value));
        self
    }

    /// Get the value if it exists and has the requested type.
    pub fn get<T: Any>(&self, name: &str) -> Option<&T> {
        self.0.get(name)?.downcast_ref()
    }
}

/// Tab size and indent unit width in columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamSettings {
    pub tab_size: NonZeroUsize,
    pub indent_unit: usize,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            tab_size: NonZeroUsize::new(4).unwrap(),
            indent_unit: 2,
        }
    }
}

/// A parser and its document-wide language data and style definitions.
pub struct StreamLanguage<M: StreamParser> {
    mode: M,
    data: LanguageData,
    token_table: TokenTable,
    indent: Option<IndentFn<M>>,
}

type IndentFn<M> = Box<
    dyn Fn(&M, &<M as StreamParser>::State, &str, &IndentContext<'_>) -> Option<usize>
        + Send
        + Sync,
>;

impl<M: StreamParser> StreamLanguage<M> {
    pub fn new(mode: M) -> Self {
        Self {
            mode,
            data: LanguageData::default(),
            token_table: TokenTable::default(),
            indent: None,
        }
    }

    /// Set the parser's default language data.
    pub fn with_language_data(mut self, data: LanguageData) -> Self {
        self.data = data;
        self
    }

    /// Set custom tag names for the parser's token styles.
    pub fn with_token_table(mut self, table: TokenTable) -> Self {
        self.token_table = table;
        self
    }

    /// Set an indent function that can inspect the full indentation context.
    /// If absent, use [`StreamParser::indent`].
    pub fn with_indent(
        mut self,
        indent: impl Fn(&M, &M::State, &str, &IndentContext<'_>) -> Option<usize>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.indent = Some(Box::new(indent));
        self
    }

    pub fn name(&self) -> &str {
        self.mode.name().unwrap_or("")
    }

    /// Return the configured document-wide language data without parsing text.
    pub fn language_data(&self) -> &LanguageData {
        &self.data
    }

    /// Parse a complete document with one mode state. For consistent replay,
    /// modes must derive results from their state, stream, and settings.
    /// [`StreamParser::copy_state`] must copy the parts of state the mode changes.
    pub fn parse(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<StreamDocument<'_, M>, TokenizeError> {
        let state = self.mode.start_state(settings.indent_unit);
        let parsed = self.parse_suffix(&text, 0, state, settings)?;
        Ok(StreamDocument {
            language: self,
            text,
            settings,
            parsed,
        })
    }

    fn parse_suffix(
        &self,
        text: &Text,
        start: usize,
        mut state: M::State,
        settings: StreamSettings,
    ) -> Result<Parsed<M::State>, TokenizeError> {
        let mut parsed = Parsed {
            lines: Vec::new(),
            tokens: Vec::new(),
        };
        let mut styles = HashMap::new();
        let mut from = start;
        for segment in text.as_str()[start..].split_inclusive('\n') {
            parsed.lines.push(Checkpoint {
                start: from,
                token_index: parsed.tokens.len(),
                state: self.mode.copy_state(&state),
            });
            let line = line_content(segment);
            if line.is_empty() {
                self.mode.blank_line(&mut state, settings.indent_unit);
            } else {
                let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
                while !stream.eol() {
                    let token = read_token(&self.mode, &mut state, from, &mut stream)?;
                    let Some(raw_style) = token.style.filter(|style| !style.is_empty()) else {
                        continue;
                    };
                    let style = styles.entry(raw_style).or_insert_with_key(|raw| {
                        Style::resolve(raw, &self.token_table).map(Arc::new)
                    });
                    let Some(style) = style else { continue };
                    if self.mode.merge_tokens()
                        && let Some(last) = parsed.tokens.last_mut()
                        && last.range.end == token.range.start
                        && last.style == *style
                    {
                        last.range.end = token.range.end;
                    } else {
                        parsed.tokens.push(CachedToken {
                            range: token.range,
                            style: Arc::clone(style),
                        });
                    }
                }
            }
            from += segment.len();
        }
        // The parser stops at EOF, but indentation still needs the state at
        // the start of an empty document or a final empty logical line.
        if start == text.len() || text.as_str().ends_with('\n') {
            parsed.lines.push(Checkpoint {
                start: from,
                token_index: parsed.tokens.len(),
                state,
            });
        }
        Ok(parsed)
    }
}

struct Parsed<S> {
    lines: Vec<Checkpoint<S>>,
    tokens: Vec<CachedToken>,
}

struct Checkpoint<S> {
    start: usize,
    token_index: usize,
    state: S,
}

struct CachedToken {
    range: Range<usize>,
    style: Arc<Style>,
}

/// A styled token, clipped to the requested UTF-8 byte range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HighlightSpan<'a> {
    pub range: Range<usize>,
    pub tags: &'a [HighlightTag],
}

/// Failure to apply and parse an edit. The document remains unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamUpdateError {
    Edit(EditError),
    Tokenize(TokenizeError),
}

impl fmt::Display for StreamUpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit(error) => write!(f, "invalid stream document edit: {error:?}"),
            Self::Tokenize(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StreamUpdateError {}

/// The parsed language services for a document.
pub struct StreamDocument<'a, M: StreamParser> {
    language: &'a StreamLanguage<M>,
    text: Text,
    settings: StreamSettings,
    parsed: Parsed<M::State>,
}

impl<M: StreamParser> StreamDocument<'_, M> {
    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Query styled tokens in a byte range. The parse behind the query includes
    /// earlier lines and the full text of each affected line, not just the range.
    /// Return `None` for a reversed or invalid UTF-8 byte range.
    pub fn tokens(&self, range: Range<usize>) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let tokens = &self.parsed.tokens;
        let start = tokens.partition_point(|token| token.range.end <= range.start);
        let end = if range.is_empty() {
            start
        } else {
            tokens.partition_point(|token| token.range.start < range.end)
        };
        Some(tokens[start..end].iter().map(move |token| HighlightSpan {
            range: token.range.start.max(range.start)..token.range.end.min(range.end),
            tags: &token.style.tags,
        }))
    }

    pub(crate) fn token_range_at(&self, position: usize, side: Assoc) -> Option<Range<usize>> {
        let tokens = &self.parsed.tokens;
        let index = tokens.partition_point(|token| {
            token.range.start < position || side == Assoc::After && token.range.start == position
        });
        let token = tokens.get(index.checked_sub(1)?)?;
        (position < token.range.end || side == Assoc::Before && position == token.range.end)
            .then(|| token.range.clone())
    }

    /// Get parser data at a valid UTF-8 byte position, including the end.
    /// The same data applies at every position in a stream document.
    pub fn language_data_at(&self, position: usize) -> Option<&LanguageData> {
        self.text
            .as_str()
            .is_char_boundary(position)
            .then_some(&self.language.data)
    }

    /// Get the mode's indentation at a byte position, or `None` if the mode
    /// gives no indentation or the position is invalid.
    pub fn indentation(
        &self,
        position: usize,
        options: IndentOptions<'_>,
    ) -> Result<Option<usize>, TokenizeError> {
        let Some(context) = IndentContext::new(&self.text, position, self.settings, options) else {
            return Ok(None);
        };
        let mode = &self.language.mode;
        let line_index = self.line_index(position);
        // An override can affect state on any earlier line. Do not reuse
        // checkpoints made with the document's original indentation.
        let first = if options.override_indentation.is_some() {
            0
        } else {
            line_index
        };
        let mut state = mode.copy_state(&self.parsed.lines[first].state);
        for (index, checkpoint) in self.parsed.lines[first..=line_index].iter().enumerate() {
            let start = checkpoint.start;
            if start == position {
                break;
            }
            let end = self
                .parsed
                .lines
                .get(first + index + 1)
                .map_or(self.text.len(), |next| next.start);
            let line = line_content(&self.text.as_str()[start..end]);
            if line.is_empty() {
                mode.blank_line(&mut state, self.settings.indent_unit);
            } else {
                let mut stream =
                    StringStream::new(line, self.settings.tab_size, self.settings.indent_unit);
                if let Some(column) = options.override_indentation.and_then(|get| get(start)) {
                    stream = stream.with_indentation_override(column);
                }
                let limit = (position - start).min(line.len());
                while stream.pos() < limit {
                    read_token(mode, &mut state, start, &mut stream)?;
                }
            }
        }
        let text = indent_text(context.line_at(position, Assoc::After).unwrap().text);
        Ok(match &self.language.indent {
            Some(indent) => indent(mode, &state, text, &context),
            None => mode.indent(&state, text, self.settings.indent_unit),
        })
    }

    /// Apply changes in old-document coordinates. Keep complete lines before
    /// the first change and parse the remainder with the saved mode state.
    pub fn apply_changes(&mut self, changes: &ChangeSet) -> Result<(), StreamUpdateError> {
        let desc = changes
            .desc(self.text.len())
            .map_err(StreamUpdateError::Edit)?;
        let Some((old, _)) = desc.changed_ranges().next() else {
            return Ok(());
        };
        let mut text = self.text.clone();
        changes
            .apply_to(&mut text)
            .map_err(StreamUpdateError::Edit)?;
        let index = self.line_index(old.start);
        let checkpoint = &self.parsed.lines[index];
        let state = self.language.mode.copy_state(&checkpoint.state);
        let new = self
            .language
            .parse_suffix(&text, checkpoint.start, state, self.settings)
            .map_err(StreamUpdateError::Tokenize)?;
        let token_index = checkpoint.token_index;
        self.parsed.tokens.truncate(token_index);
        self.parsed.tokens.extend(new.tokens);
        self.parsed.lines.truncate(index);
        self.parsed
            .lines
            .extend(new.lines.into_iter().map(|mut line| {
                line.token_index += token_index;
                line
            }));
        self.text = text;
        Ok(())
    }

    fn line_index(&self, position: usize) -> usize {
        self.parsed
            .lines
            .partition_point(|line| line.start <= position)
            - 1
    }
}

fn line_content(segment: &str) -> &str {
    segment
        .strip_suffix('\n')
        .map_or(segment, |line| line.strip_suffix('\r').unwrap_or(line))
}
