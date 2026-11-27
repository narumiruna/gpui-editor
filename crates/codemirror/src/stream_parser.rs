//! Line streams and stateful tokenization, based on `@codemirror/streamparser` 6.0.0.
//!
//! Positions and lengths are UTF-8 bytes. Character operations and columns use
//! Unicode scalar values, not UTF-16 code units or grapheme clusters.

use std::{borrow::Cow, cell::Cell, fmt, iter::FusedIterator, num::NonZeroUsize, ops::Range};

use regex::{Captures, Regex};

pub mod utf16;

/// A position fell outside a line or inside a UTF-8 scalar value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidStreamPosition;

impl fmt::Display for InvalidStreamPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("position is not a UTF-8 boundary in the stream line")
    }
}

impl std::error::Error for InvalidStreamPosition {}

/// A cursor for one logical line. The caller supplies the line without its ending.
#[derive(Debug)]
pub struct StringStream<'line> {
    line: &'line str,
    pos: usize,
    start: usize,
    tab_size: NonZeroUsize,
    indent_unit: usize,
    indentation_override: Option<usize>,
    column_cache: Cell<(usize, usize)>,
}

impl<'line> StringStream<'line> {
    pub fn new(line: &'line str, tab_size: NonZeroUsize, indent_unit: usize) -> Self {
        Self {
            line,
            pos: 0,
            start: 0,
            tab_size,
            indent_unit,
            indentation_override: None,
            column_cache: Cell::new((0, 0)),
        }
    }

    /// Use the given column for [`Self::indentation`], including zero.
    pub fn with_indentation_override(mut self, column: usize) -> Self {
        self.indentation_override = Some(column);
        self
    }

    pub fn line(&self) -> &'line str {
        self.line
    }

    pub fn indent_unit(&self) -> usize {
        self.indent_unit
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn start(&self) -> usize {
        self.start
    }

    /// Set the token start to the cursor. [`Tokenizer`] does this for each token.
    pub fn start_token(&mut self) {
        self.start = self.pos;
    }

    /// Move to a line-relative byte offset. On error, do not move.
    pub fn set_pos(&mut self, pos: usize) -> Result<(), InvalidStreamPosition> {
        if !self.line.is_char_boundary(pos) {
            return Err(InvalidStreamPosition);
        }
        self.pos = pos;
        Ok(())
    }

    pub fn eol(&self) -> bool {
        self.pos == self.line.len()
    }

    pub fn sol(&self) -> bool {
        self.pos == 0
    }

    pub fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    pub fn eat(&mut self, expected: char) -> Option<char> {
        self.eat_if(|ch| ch == expected)
    }

    pub fn eat_if(&mut self, predicate: impl FnOnce(char) -> bool) -> Option<char> {
        let ch = self.peek().filter(|&ch| predicate(ch))?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    /// Test the regex against only the next scalar, not the rest of the line.
    pub fn eat_regex(&mut self, pattern: &Regex) -> Option<char> {
        self.eat_if(|ch| scalar_matches(pattern, ch))
    }

    /// Return true if at least one scalar matched and was consumed.
    pub fn eat_while(&mut self, mut predicate: impl FnMut(char) -> bool) -> bool {
        let start = self.pos;
        while self.eat_if(&mut predicate).is_some() {}
        self.pos > start
    }

    pub fn eat_while_regex(&mut self, pattern: &Regex) -> bool {
        self.eat_while(|ch| scalar_matches(pattern, ch))
    }

    /// Consume ECMAScript whitespace, including non-breaking space and BOM.
    pub fn eat_space(&mut self) -> bool {
        self.eat_while(is_stream_space)
    }

    pub fn skip_to_end(&mut self) {
        self.pos = self.line.len();
    }

    /// Move to the start of the first occurrence at or after the cursor.
    /// If the text is not found, return false and do not move.
    pub fn skip_to(&mut self, text: &str) -> bool {
        let Some(offset) = self.remaining().find(text) else {
            return false;
        };
        self.pos += offset;
        true
    }

    /// Move back by the given number of bytes. On error, do not move.
    pub fn back_up(&mut self, bytes: usize) -> Result<(), InvalidStreamPosition> {
        self.set_pos(self.pos.checked_sub(bytes).ok_or(InvalidStreamPosition)?)
    }

    /// The zero-based column at the token start, not at the cursor.
    /// Each scalar counts as one column. Tabs move to the next tab stop.
    pub fn column(&self) -> usize {
        let (pos, column) = self.column_cache.get();
        let (pos, column) = if pos <= self.start {
            (pos, column)
        } else {
            (0, 0)
        };
        let column = count_columns(self.line[pos..self.start].chars(), self.tab_size, column);
        self.column_cache.set((self.start, column));
        column
    }

    /// The column after the leading whitespace, unless an override was set.
    pub fn indentation(&self) -> usize {
        self.indentation_override.unwrap_or_else(|| {
            count_columns(
                self.line.chars().take_while(|&ch| is_stream_space(ch)),
                self.tab_size,
                0,
            )
        })
    }

    /// Match a literal at the cursor. Move past it only when `consume` is true.
    /// Case-insensitive matching lowercases a prefix with the same number of
    /// scalars as the pattern. The byte lengths can differ.
    pub fn match_str(&mut self, pattern: &str, consume: bool, case_insensitive: bool) -> bool {
        let remaining = self.remaining();
        let bytes = if case_insensitive {
            let mut suffix = remaining.chars();
            for _ in pattern.chars() {
                if suffix.next().is_none() {
                    return false;
                }
            }
            let bytes = remaining.len() - suffix.as_str().len();
            if remaining[..bytes].to_lowercase() != pattern.to_lowercase() {
                return false;
            }
            bytes
        } else {
            if !remaining.starts_with(pattern) {
                return false;
            }
            pattern.len()
        };
        if consume {
            self.pos += bytes;
        }
        true
    }

    /// Match a native Rust regex against the remaining line. Reject matches
    /// that start after the cursor. Capture offsets are relative to the cursor
    /// before the match. A zero-length match does not move the cursor.
    pub fn match_regex(&mut self, pattern: &Regex, consume: bool) -> Option<Captures<'line>> {
        let captures = pattern.captures(self.remaining())?;
        let matched = captures.get(0).filter(|matched| matched.start() == 0)?;
        if consume {
            self.pos += matched.end();
        }
        Some(captures)
    }

    /// Text from the token start to the cursor. If the cursor is before the
    /// token start, return an empty string.
    pub fn current(&self) -> &'line str {
        self.line.get(self.start..self.pos).unwrap_or("")
    }

    fn remaining(&self) -> &'line str {
        &self.line[self.pos..]
    }
}

impl Iterator for StringStream<'_> {
    type Item = char;

    /// Read the next scalar and advance by its UTF-8 byte length.
    fn next(&mut self) -> Option<char> {
        self.eat_if(|_| true)
    }
}

fn scalar_matches(pattern: &Regex, ch: char) -> bool {
    pattern.is_match(ch.encode_utf8(&mut [0; 4]))
}

fn is_stream_space(ch: char) -> bool {
    // Rust recognizes NEL as whitespace. ECMAScript excludes it but includes BOM.
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{FEFF}'
}

fn count_columns(chars: impl Iterator<Item = char>, tab_size: NonZeroUsize, start: usize) -> usize {
    chars.fold(start, |column, ch| {
        let width = if ch == '\t' {
            tab_size.get() - column % tab_size.get()
        } else {
            1
        };
        column.saturating_add(width)
    })
}

/// A mode that reads tokens with mutable state shared across lines.
///
/// Styles are opaque strings here. Highlight tags, language data, and syntax
/// trees are the responsibility of a language integration.
pub trait StreamParser {
    type State: Clone;

    fn name(&self) -> Option<&str> {
        None
    }

    fn start_state(&self, indent_unit: usize) -> Self::State;

    /// Read a token. Return `None` for unstyled text. A call can leave the
    /// cursor at the token start to change state before the next attempt.
    fn token(&self, stream: &mut StringStream<'_>, state: &mut Self::State)
    -> Option<Cow<'_, str>>;

    /// Update state on a line with no content bytes.
    fn blank_line(&self, _state: &mut Self::State, _indent_unit: usize) {}

    fn copy_state(&self, state: &Self::State) -> Self::State {
        state.clone()
    }

    fn indent(
        &self,
        _state: &Self::State,
        _text_after: &str,
        _indent_unit: usize,
    ) -> Option<usize> {
        None
    }

    /// A preference for a downstream tree builder. Raw tokenizer output is not merged.
    fn merge_tokens(&self) -> bool {
        true
    }
}

/// A nonempty token with a document-relative UTF-8 byte range.
#[derive(Debug, Eq, PartialEq)]
pub struct Token<'input, 'mode> {
    pub range: Range<usize>,
    pub text: &'input str,
    pub style: Option<Cow<'mode, str>>,
}

/// A mode broke the token contract. The tokenizer stops after it yields the error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenizeError {
    /// The cursor did not move past the token start in ten calls.
    NoProgress { position: usize },
    /// The mode replaced the stream with one for a different line.
    ReplacedLine { position: usize },
}

impl fmt::Display for TokenizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProgress { position } => {
                write!(f, "stream parser did not advance at byte {position}")
            }
            Self::ReplacedLine { position } => {
                write!(f, "stream parser replaced the line at byte {position}")
            }
        }
    }
}

impl std::error::Error for TokenizeError {}

/// Lazily read tokens with one mode state for the entire input.
///
/// As in [`crate::Text`], LF and CRLF end lines. A lone CR is content.
/// Line endings do not produce tokens. Empty lines (including a final empty
/// line) invoke `blank_line` as the iterator reaches them.
pub struct Tokenizer<'input, 'mode, M: StreamParser + ?Sized> {
    mode: &'mode M,
    state: M::State,
    source: &'input str,
    tab_size: NonZeroUsize,
    indent_unit: usize,
    next_line: Option<usize>,
    line: Option<(usize, StringStream<'input>)>,
}

impl<'input, 'mode, M: StreamParser + ?Sized> Tokenizer<'input, 'mode, M> {
    pub fn new(
        mode: &'mode M,
        source: &'input str,
        tab_size: NonZeroUsize,
        indent_unit: usize,
    ) -> Self {
        Self {
            mode,
            state: mode.start_state(indent_unit),
            source,
            tab_size,
            indent_unit,
            next_line: Some(0),
            line: None,
        }
    }

    /// Mode state after the most recent token, or after the last attempt on error.
    pub fn state(&self) -> &M::State {
        &self.state
    }
}

impl<'input, 'mode, M: StreamParser + ?Sized> Iterator for Tokenizer<'input, 'mode, M> {
    type Item = Result<Token<'input, 'mode>, TokenizeError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((line_start, stream)) = &mut self.line
                && !stream.eol()
            {
                let result = read_token(self.mode, &mut self.state, *line_start, stream);
                if result.is_err() {
                    self.line = None;
                    self.next_line = None;
                }
                return Some(result);
            }

            let from = self.next_line.take()?;
            let remaining = &self.source[from..];
            let line = if let Some(end) = remaining.find('\n') {
                self.next_line = Some(from + end + 1);
                remaining[..end]
                    .strip_suffix('\r')
                    .unwrap_or(&remaining[..end])
            } else {
                remaining
            };
            if line.is_empty() {
                self.mode.blank_line(&mut self.state, self.indent_unit);
            } else {
                self.line = Some((
                    from,
                    StringStream::new(line, self.tab_size, self.indent_unit),
                ));
            }
        }
    }
}

impl<M: StreamParser + ?Sized> FusedIterator for Tokenizer<'_, '_, M> {}

pub(crate) fn read_token<'input, 'mode, M: StreamParser + ?Sized>(
    mode: &'mode M,
    state: &mut M::State,
    line_start: usize,
    stream: &mut StringStream<'input>,
) -> Result<Token<'input, 'mode>, TokenizeError> {
    let line = stream.line();
    let start = stream.pos();
    let position = line_start + start;
    for _ in 0..10 {
        stream.start = start;
        let style = mode.token(stream, state);
        if !std::ptr::eq(line, stream.line()) {
            return Err(TokenizeError::ReplacedLine { position });
        }
        let end = stream.pos();
        if end > start {
            return Ok(Token {
                range: position..line_start + end,
                text: &line[start..end],
                style,
            });
        }
    }
    Err(TokenizeError::NoProgress { position })
}
