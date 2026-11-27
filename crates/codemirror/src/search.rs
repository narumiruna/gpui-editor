//! In-document search based on `@codemirror/search` 6.7.3.
//!
//! All ranges use UTF-8 byte offsets into the original [`Text`]. Use
//! [`extension`] for query state, [`SearchAction`] to run search actions, and
//! [`scroll_to_match`] to change the effect used to reveal a match. Use the
//! highlight functions to create native marks in a viewport.
//!
//! Use [`search_panel_factory`] to attach a custom lifecycle to the registered
//! search panel. The host supplies panel and dialog content, focus, input,
//! scrolling, and announcements. Regexes use a subset of ECMAScript. Literal
//! cursors use fixed NFKD normalization with optional lowercasing. Word tests
//! use native grapheme segmentation and optionally use configured language word characters.

// MIT License
//
// Copyright (C) 2018-2021 by Marijn Haverbeke <marijn@haverbeke.berlin> and others
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

mod actions;
mod commands;
mod filter;
mod goto_line;
mod highlight;
mod literal;
mod panel;
mod query_state;
mod regexp;
mod selection;

use std::{fmt, iter::FusedIterator, ops::Range, sync::Arc};

use crate::{Change, ChangeSet, EditError, EditorState, Text};

pub use crate::word_characters;
pub use actions::{SearchAction, SearchActionResult, search_keymap};
pub use commands::{
    find_next, find_previous, match_announcement, replace_all, replace_next,
    replacement_announcement, replacements_announcement, scroll_to_match,
};
pub use goto_line::{goto_line, goto_line_transaction};
pub use highlight::highlight_search_matches;
pub use literal::SearchCursor;
pub use panel::{SearchPanel, search_panel_factory};
pub use query_state::{
    SearchPanelInput, close_search_panel, extension, get_search_query, open_search_panel,
    search_panel_id, search_panel_open, search_panel_placement, set_search_query,
};
pub use regexp::RegExpCursor;
pub use selection::{
    HighlightSelectionOptions, SCROLL_TO_OCCURRENCE, SelectionMatchConfig,
    highlight_selection_matches, select_matches, select_next_occurrence, select_selection_matches,
    selection_match_decorations, selection_match_extension, selection_match_options,
};

use filter::{MatchFilter, MatchTest};
use literal::LiteralPattern;
use regexp::RegExpPattern;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchError {
    InvalidRange,
    EmptyQuery,
    InvalidRegExp(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => f.write_str("search range is not a valid UTF-8 byte range"),
            Self::EmptyQuery => f.write_str("search query is empty"),
            Self::InvalidRegExp(reason) => write!(f, "invalid or unsupported regex: {reason}"),
        }
    }
}

impl std::error::Error for SearchError {}

/// Options for either cursor. Direct cursors are case-sensitive by default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CursorOptions {
    pub ignore_case: bool,
    pub whole_word: bool,
}

/// A match in the original text, with captures numbered from zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch<'a> {
    pub range: Range<usize>,
    /// False when the search matched only part of a normalized scalar.
    pub precise: bool,
    pub text: &'a str,
    /// Regex captures include the full match at index zero. A literal match
    /// has no captures. An optional group that did not match has value `None`.
    pub captures: Vec<Option<&'a str>>,
}

/// Query settings. By default, use case-insensitive text search and interpret
/// `\n`, `\r`, `\t`, and `\\` in literal queries and replacement text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regexp: bool,
    /// Do not interpret escapes in literal queries or replacement text.
    /// This does not change regex syntax or capture substitution.
    pub literal: bool,
}

/// A reusable search query. Use [`Self::from_input`] to retain invalid input.
#[derive(Clone, Debug)]
pub struct SearchQuery {
    search: Arc<str>,
    replacement: Arc<str>,
    pattern: Result<Pattern, SearchError>,
    options: SearchOptions,
    test: Option<MatchTest>,
}

/// Equality follows the pinned CodeMirror query comparison. It ignores
/// the literal escape flag and compares test callbacks by identity.
impl PartialEq for SearchQuery {
    fn eq(&self, other: &Self) -> bool {
        self.search == other.search
            && self.replacement == other.replacement
            && self.options.case_sensitive == other.options.case_sensitive
            && self.options.whole_word == other.options.whole_word
            && self.options.regexp == other.options.regexp
            && self.test == other.test
    }
}

impl Eq for SearchQuery {}

#[derive(Clone, Debug)]
enum Pattern {
    Literal(LiteralPattern),
    RegExp(RegExpPattern),
}

/// Search an editor snapshot using its selection, extensions, and text.
pub struct StateSearch<'a> {
    query: &'a SearchQuery,
    state: &'a EditorState,
}

impl<'a> StateSearch<'a> {
    pub fn cursor(&self, range: Range<usize>) -> Result<SearchMatches<'a>, SearchError> {
        self.query
            .cursor_with(self.state.text(), range, Some(self.state))
    }

    pub fn find_next(
        &self,
        selection: Range<usize>,
    ) -> Result<Option<SearchMatch<'a>>, SearchError> {
        self.query
            .find_next_with(self.state.text(), selection, Some(self.state))
    }

    pub fn find_previous(
        &self,
        selection: Range<usize>,
    ) -> Result<Option<SearchMatch<'a>>, SearchError> {
        self.query
            .find_previous_with(self.state.text(), selection, Some(self.state))
    }

    pub fn replace_all(&self, template: &str) -> Result<ChangeSet, EditError> {
        self.replacements(template).map(|(changes, _)| changes)
    }

    /// Build replacements and count precise matches before combining adjacent edits.
    pub fn replacements(&self, template: &str) -> Result<(ChangeSet, usize), EditError> {
        self.query
            .replace_all_with(self.state.text(), template, Some(self.state))
    }
}

/// The non-overlapping matches for a query in a document range.
pub enum SearchMatches<'a> {
    Literal(SearchCursor<'a>),
    RegExp(RegExpCursor<'a>),
}

impl<'a> Iterator for SearchMatches<'a> {
    type Item = SearchMatch<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Literal(cursor) => cursor.next(),
            Self::RegExp(cursor) => cursor.next(),
        }
    }
}

impl FusedIterator for SearchMatches<'_> {}

impl SearchQuery {
    pub fn new(query: &str, options: SearchOptions) -> Result<Self, SearchError> {
        let query = Self::from_input(query, options);
        query.pattern()?;
        Ok(query)
    }

    /// Keep the input even if it is empty or the native regex engine rejects it.
    pub fn from_input(query: &str, options: SearchOptions) -> Self {
        let pattern = if query.is_empty() {
            Err(SearchError::EmptyQuery)
        } else if options.regexp {
            RegExpPattern::new(query, !options.case_sensitive).map(Pattern::RegExp)
        } else {
            let unquoted = unquote(query, options.literal);
            Ok(Pattern::Literal(LiteralPattern::new(
                &unquoted,
                !options.case_sensitive,
            )))
        };
        Self {
            search: query.into(),
            replacement: "".into(),
            pattern,
            options,
            test: None,
        }
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    pub fn options(&self) -> SearchOptions {
        self.options
    }

    pub fn is_valid(&self) -> bool {
        self.pattern.is_ok()
    }

    /// Store the replacement input with this query. Text-level replacement
    /// helpers take an explicit template; pass [`Self::replacement_text`] to use it.
    pub fn with_replacement(mut self, replacement: impl Into<Arc<str>>) -> Self {
        self.replacement = replacement.into();
        self
    }

    pub fn replacement_text(&self) -> &str {
        &self.replacement
    }

    /// Test each match after applying whole-word rules, before returning or replacing it.
    /// A text-only search supplies a state with that text and default configuration.
    pub fn with_test(
        mut self,
        test: impl Fn(&SearchMatch<'_>, &EditorState) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.test = Some(MatchTest::new(test));
        self
    }

    /// Use the editor's current state to resolve language word characters and match tests.
    pub fn in_state<'a>(&'a self, state: &'a EditorState) -> StateSearch<'a> {
        StateSearch { query: self, state }
    }

    pub fn cursor<'a>(
        &self,
        text: &'a Text,
        range: Range<usize>,
    ) -> Result<SearchMatches<'a>, SearchError> {
        self.cursor_with(text, range, None)
    }

    fn cursor_with<'a>(
        &self,
        text: &'a Text,
        range: Range<usize>,
        state: Option<&EditorState>,
    ) -> Result<SearchMatches<'a>, SearchError> {
        check_range(text, &range)?;
        Ok(self.cursor_in(text, range, self.pattern()?, state))
    }

    /// Find the first match after the selection, then wrap to the start.
    /// Literal search does not return the same match as the selection.
    pub fn find_next<'a>(
        &self,
        text: &'a Text,
        selection: Range<usize>,
    ) -> Result<Option<SearchMatch<'a>>, SearchError> {
        self.find_next_with(text, selection, None)
    }

    fn find_next_with<'a>(
        &self,
        text: &'a Text,
        selection: Range<usize>,
        state: Option<&EditorState>,
    ) -> Result<Option<SearchMatch<'a>>, SearchError> {
        check_range(text, &selection)?;
        let pattern = self.pattern()?;
        let mut found = self
            .cursor_in(text, selection.end..text.len(), pattern, state)
            .next();
        if found.is_none() {
            let end = match pattern {
                Pattern::Literal(pattern) => {
                    selection.start
                        + bytes_for_units(text.as_str()[selection.start..].chars(), pattern.units)
                }
                Pattern::RegExp(_) => selection.start,
            };
            found = self.cursor_in(text, 0..end, pattern, state).next();
        }
        Ok(self.exclude_selection(found, selection))
    }

    /// Find the last match before the selection, then wrap to the end.
    /// Literal search does not return the same match as the selection.
    pub fn find_previous<'a>(
        &self,
        text: &'a Text,
        selection: Range<usize>,
    ) -> Result<Option<SearchMatch<'a>>, SearchError> {
        self.find_previous_with(text, selection, None)
    }

    fn find_previous_with<'a>(
        &self,
        text: &'a Text,
        selection: Range<usize>,
        state: Option<&EditorState>,
    ) -> Result<Option<SearchMatch<'a>>, SearchError> {
        check_range(text, &selection)?;
        let pattern = self.pattern()?;
        let mut found = self.last(text, 0..selection.start, pattern, state);
        if found.is_none() {
            let start = match pattern {
                Pattern::Literal(pattern) => {
                    selection.end
                        - bytes_for_units(
                            text.as_str()[..selection.end].chars().rev(),
                            pattern.units,
                        )
                }
                Pattern::RegExp(_) => selection.end,
            };
            found = self.last(text, start..text.len(), pattern, state);
        }
        Ok(self.exclude_selection(found, selection))
    }

    /// Expand the replacement template for a match of this query.
    /// Regex templates support `$$`, `$&`, and decimal capture numbers.
    /// The longest valid capture prefix wins. An unmatched group expands to
    /// `undefined`, as in CodeMirror 6.7.3. Other dollar forms remain literal.
    pub fn replacement(&self, result: &SearchMatch<'_>, template: &str) -> String {
        self.expand_template(result, &unquote(template, self.options.literal))
    }

    /// Build non-overlapping replacements against the original text.
    /// Leave partial matches of normalized scalars unchanged. An invalid query
    /// makes no changes. The caller enforces edit permissions.
    pub fn replace_all(&self, text: &Text, template: &str) -> Result<ChangeSet, EditError> {
        self.replace_all_with(text, template, None)
            .map(|(changes, _)| changes)
    }

    fn replace_all_with(
        &self,
        text: &Text,
        template: &str,
        state: Option<&EditorState>,
    ) -> Result<(ChangeSet, usize), EditError> {
        let Ok(pattern) = self.pattern() else {
            return Ok((ChangeSet::default(), 0));
        };
        let template = unquote(template, self.options.literal);
        let changes: Vec<_> = self
            .cursor_in(text, 0..text.len(), pattern, state)
            .filter(|result| result.precise)
            .map(|result| {
                let replacement = self.expand_template(&result, &template);
                Change::new(result.range, replacement)
            })
            .collect();
        // Adjacent deletions can merge, and empty insertions can disappear.
        let count = changes.len();
        Ok((ChangeSet::new(changes)?, count))
    }

    fn expand_template(&self, result: &SearchMatch<'_>, template: &str) -> String {
        if self.options.regexp {
            expand_replacement(template, result)
        } else {
            template.to_owned()
        }
    }

    fn pattern(&self) -> Result<&Pattern, SearchError> {
        self.pattern.as_ref().map_err(Clone::clone)
    }

    fn cursor_in<'a>(
        &self,
        text: &'a Text,
        range: Range<usize>,
        pattern: &Pattern,
        state: Option<&EditorState>,
    ) -> SearchMatches<'a> {
        let filter =
            MatchFilter::for_query(self.options.whole_word, self.test.as_ref(), text, state);
        match pattern {
            Pattern::Literal(pattern) => SearchMatches::Literal(SearchCursor::with_filter(
                text,
                range,
                pattern.clone(),
                filter,
            )),
            Pattern::RegExp(pattern) => SearchMatches::RegExp(RegExpCursor::with_pattern(
                text,
                range,
                pattern.clone(),
                filter,
            )),
        }
    }

    fn last<'a>(
        &self,
        text: &'a Text,
        range: Range<usize>,
        pattern: &Pattern,
        state: Option<&EditorState>,
    ) -> Option<SearchMatch<'a>> {
        const CHUNK: usize = 10_000;
        let source = text.as_str();
        // Carry the extra unit from a split surrogate pair to avoid shifting
        // the next window's cut.
        let retreat = |(from, extra), units: usize| {
            let (bytes, extra) = take_units(source[range.start..from].chars().rev(), units - extra);
            (from - bytes, extra)
        };
        match pattern {
            Pattern::Literal(literal) => {
                let mut end = (range.end, 0);
                loop {
                    let (start, _) = retreat(end, CHUNK.saturating_add(literal.units));
                    let SearchMatches::Literal(mut cursor) =
                        self.cursor_in(text, start..end.0, pattern, state)
                    else {
                        unreachable!("literal query cursor");
                    };
                    let found = std::iter::from_fn(|| cursor.next_overlapping()).last();
                    if found.is_some() || start == range.start {
                        return found;
                    }
                    end = retreat(end, CHUNK);
                }
            }
            Pattern::RegExp(_) => {
                let mut cut = (range.end, 0);
                loop {
                    cut = retreat(cut, CHUNK);
                    let (start, extra) = cut;
                    let found = self
                        .cursor_in(text, start..range.end, pattern, state)
                        .last();
                    // Rescan a wider window if a match may be clipped at its start.
                    if start == range.start
                        || found.as_ref().is_some_and(|matched| {
                            let threshold = 10 + extra;
                            source[start..matched.range.start]
                                .encode_utf16()
                                .take(threshold + 1)
                                .count()
                                > threshold
                        })
                    {
                        return found;
                    }
                }
            }
        }
    }

    fn exclude_selection<'a>(
        &self,
        result: Option<SearchMatch<'a>>,
        selection: Range<usize>,
    ) -> Option<SearchMatch<'a>> {
        result.filter(|found| self.options.regexp || found.range != selection)
    }
}

fn check_range(text: &Text, range: &Range<usize>) -> Result<(), SearchError> {
    text.slice(range.clone())
        .map(|_| ())
        .ok_or(SearchError::InvalidRange)
}

// Window lengths use UTF-16 units. Include the entire scalar when a cut
// would split it in the native buffer.
fn bytes_for_units(chars: impl Iterator<Item = char>, units: usize) -> usize {
    take_units(chars, units).0
}

fn take_units(chars: impl Iterator<Item = char>, mut units: usize) -> (usize, usize) {
    let mut bytes = 0;
    for ch in chars {
        if units == 0 {
            break;
        }
        bytes += ch.len_utf8();
        let size = ch.len_utf16();
        if size > units {
            return (bytes, size - units);
        }
        units -= size;
    }
    (bytes, 0)
}

fn unquote(text: &str, literal: bool) -> std::borrow::Cow<'_, str> {
    if literal || !text.contains('\\') {
        return text.into();
    }
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let unquoted = match chars.peek() {
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('\\') => '\\',
                _ => {
                    result.push(ch);
                    continue;
                }
            };
            chars.next();
            result.push(unquoted);
        } else {
            result.push(ch);
        }
    }
    result.into()
}

fn expand_replacement(template: &str, result: &SearchMatch<'_>) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find('$') {
        output.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        if let Some(tail) = rest.strip_prefix('$') {
            output.push('$');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix('&') {
            output.push_str(result.text);
            rest = tail;
        } else if let Some((length, capture)) = capture_prefix(rest, result.captures.len()) {
            output.push_str(result.captures[capture].unwrap_or("undefined"));
            rest = &rest[length..];
        } else {
            output.push('$');
        }
    }
    output.push_str(rest);
    output
}

fn capture_prefix(rest: &str, count: usize) -> Option<(usize, usize)> {
    let mut prefix = None;
    let mut number = 0usize;
    for (index, byte) in rest.bytes().take_while(u8::is_ascii_digit).enumerate() {
        // A larger number (or an overflow) cannot become a capture number by
        // adding more digits. Keep the longest prefix that was in range.
        number = number
            .saturating_mul(10)
            .saturating_add((byte - b'0') as usize);
        if number >= count {
            break;
        }
        if number != 0 {
            prefix = Some((index + 1, number));
        }
    }
    prefix
}
