use std::{borrow::Cow, ops::Range, sync::LazyLock};

use super::{SearchCursor, SearchQuery, literal::LiteralPattern, query_state::stored_query};
use crate::{
    Assoc, CharCategorizer, ConfigurationError, EditError, EditorSelection, EditorState, Extension,
    Facet, MapMode, RangeSet, SelectionRange, StateEffectType, StateField, Transaction,
    language::line_projection::{splits_crlf, uses_default_editor_lines},
    view::{MarkDecoration, SCROLL_INTO_VIEW, ScrollTarget, Viewport},
};

const MATCH_LIMIT: usize = 1000;

/// A scroll request for the newly added occurrence. The byte position is in
/// the resulting document. New hosts can use [`SCROLL_INTO_VIEW`] instead.
pub static SCROLL_TO_OCCURRENCE: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|pos, changes| {
        changes.map_pos(*pos, Assoc::Before, MapMode::Simple)
    })
});

/// Select all matches of the stored query, using the first match as primary.
/// Return `None` when the search extension is absent, the query is invalid,
/// or the number of matches is outside 1 through 1000.
/// This command does not change the text, even in read-only state.
pub fn select_matches(state: &EditorState) -> Option<Transaction> {
    stored_query(state)?.select_matches(state)
}

impl SearchQuery {
    /// Select all matches of this query without installing the search extension.
    /// The first match is primary. Return `None` for an invalid query or a match
    /// count outside 1 through 1000.
    pub fn select_matches(&self, state: &EditorState) -> Option<Transaction> {
        let text = state.text();
        let matches = self.in_state(state).cursor(0..text.len()).ok()?;
        select_all_matches(matches.map(|found| found.range), MATCH_LIMIT, None)
    }
}

/// Select all non-overlapping matches of the sole nonempty selection.
/// Matching is case sensitive and does not interpret escapes. Keep the match
/// at the original start as primary, or use the first match if none starts there.
/// Like upstream, this command accepts one more match than [`select_matches`].
pub fn select_selection_matches(state: &EditorState) -> Option<Transaction> {
    let selection = state.selection();
    let range = selection.main().range();
    if selection.ranges().len() != 1 || range.is_empty() {
        return None;
    }
    let text = state.text();
    let logical = uses_logical_selection_lines(state);
    let selected = selection_text(&text.as_str()[range.clone()], logical);
    select_all_matches(
        SearchCursor::selected_text(
            text,
            0..text.len(),
            LiteralPattern::new(&selected, false),
            logical,
        )
        .map(|found| found.range),
        MATCH_LIMIT + 1,
        Some(range.start),
    )
}

// A native caret can be inside CRLF, which has no corresponding CodeMirror
// position. Keep raw matching for selections with such an endpoint.
fn uses_logical_selection_lines(state: &EditorState) -> bool {
    uses_default_editor_lines(&state.context()).expect("valid line policy")
        && state.selection().ranges().iter().all(|range| {
            let source = state.text().as_str();
            !splits_crlf(source, range.anchor()) && !splits_crlf(source, range.head())
        })
}

fn selection_text(source: &str, logical: bool) -> Cow<'_, str> {
    if logical && source.as_bytes().contains(&b'\r') {
        Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(source)
    }
}

fn select_all_matches(
    matches: impl Iterator<Item = Range<usize>>,
    limit: usize,
    primary_start: Option<usize>,
) -> Option<Transaction> {
    let mut ranges = Vec::new();
    let mut primary = 0;
    for found in matches {
        if ranges.len() == limit {
            return None;
        }
        if primary_start == Some(found.start) {
            primary = ranges.len();
        }
        ranges.push(forward(found));
    }
    let selection = EditorSelection::new(ranges, primary).ok()?;
    Some(
        Transaction::default()
            .with_selection(selection)
            .with_user_event("select.search.matches"),
    )
}

/// If any range is empty, select the word at each range's head. Otherwise,
/// add the next occurrence after the last range, wrapping once if necessary.
/// All ranges must contain the same text to add an occurrence. Keep the
/// primary range and its direction when adding a disjoint range.
pub fn select_next_occurrence(state: &EditorState) -> Option<Transaction> {
    let selection = state.selection();
    let ranges = selection.ranges();
    let source = state.text().as_str();
    if ranges.iter().any(|range| range.range().is_empty()) {
        let expanded = ranges.iter().map(|range| {
            let at = range.head();
            state
                .word_range_at(at)
                .map_or_else(|| SelectionRange::new(at, at), forward)
        });
        let next = EditorSelection::new(expanded.collect(), selection.main_index()).ok()?;
        return (&next != selection).then(|| Transaction::default().with_selection(next));
    }

    let logical = uses_logical_selection_lines(state);
    let query = selection_text(&source[ranges[0].range()], logical);
    if ranges[1..]
        .iter()
        .any(|range| selection_text(&source[range.range()], logical) != query)
    {
        return None;
    }
    let main = selection.main();
    let whole_word = state.word_range_at(main.head()) == Some(main.range());
    let last = ranges.last()?.range();
    let pattern = LiteralPattern::new(&query, false);
    let search = |range| SearchCursor::selected_text(state.text(), range, pattern.clone(), logical);
    let is_match = |range: &Range<usize>| {
        !whole_word || state.word_range_at(range.start).as_ref() == Some(range)
    };
    let found = search(last.end..source.len())
        .find(|found| is_match(&found.range))
        .or_else(|| {
            // Upstream stops one UTF-16 unit before the last range's start.
            // Round a cut inside an astral scalar down to its start.
            let end = source[..last.start]
                .char_indices()
                .next_back()
                .map_or(0, |(at, _)| at);
            search(0..end).find(|found| {
                ranges
                    .binary_search_by_key(&found.range.start, |range| range.range().start)
                    .is_err()
                    && is_match(&found.range)
            })
        })?;
    let end = found.range.end;
    let next = std::iter::once(forward(found.range))
        .chain(ranges.iter().copied())
        .collect();
    let selection = EditorSelection::new(next, selection.main_index() + 1).ok()?;
    Some(
        Transaction::default()
            .with_selection(selection)
            .with_effects([
                SCROLL_INTO_VIEW.of(ScrollTarget::at(end)),
                SCROLL_TO_OCCURRENCE.of(end),
            ]),
    )
}

fn forward(range: Range<usize>) -> SelectionRange {
    SelectionRange::new(range.start, range.end)
}

/// Options for marks that match the selection or the word at a lone cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightSelectionOptions {
    pub highlight_word_around_cursor: bool,
    /// Minimum selection length in UTF-16 units. The maximum is 200 units.
    pub min_selection_length: usize,
    /// Do not mark any matches if more than this many marks are visible.
    pub max_matches: usize,
    pub whole_words: bool,
}

impl Default for HighlightSelectionOptions {
    fn default() -> Self {
        Self {
            highlight_word_around_cursor: false,
            min_selection_length: 1,
            max_matches: 100,
            whole_words: false,
        }
    }
}

/// Optional inputs to [`selection_match_extension`]. Omitted fields do not
/// take part in merging. Defaults apply after all inputs combine.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SelectionMatchConfig {
    pub highlight_word_around_cursor: Option<bool>,
    pub min_selection_length: Option<usize>,
    pub max_matches: Option<usize>,
    pub whole_words: Option<bool>,
}

type MatchOptions = Result<HighlightSelectionOptions, ConfigurationError>;
static MATCH_OPTIONS: LazyLock<Facet<SelectionMatchConfig, MatchOptions>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&SelectionMatchConfig]| {
        let mut whole_words = inputs.iter().filter_map(|input| input.whole_words);
        let first = whole_words.next();
        if first.is_some_and(|value| whole_words.any(|other| other != value)) {
            return Err(ConfigurationError::InvalidFieldData);
        }
        let defaults = HighlightSelectionOptions::default();
        Ok(HighlightSelectionOptions {
            highlight_word_around_cursor: inputs
                .iter()
                .any(|input| input.highlight_word_around_cursor == Some(true)),
            min_selection_length: inputs
                .iter()
                .filter_map(|input| input.min_selection_length)
                .min()
                .unwrap_or(defaults.min_selection_length),
            max_matches: inputs
                .iter()
                .filter_map(|input| input.max_matches)
                .min()
                .unwrap_or(defaults.max_matches),
            whole_words: first.unwrap_or(defaults.whole_words),
        })
    })
});
static MATCH_CONFIG: LazyLock<StateField<HighlightSelectionOptions>> = LazyLock::new(|| {
    StateField::new(
        |state| *state.facet(&MATCH_OPTIONS)?,
        |_, update| *update.state().facet(&MATCH_OPTIONS)?,
    )
});

/// Enable selection-match marks. Configured lengths and caps take their
/// minima, cursor flags combine with OR, and conflicting explicit
/// `whole_words` values reject the configuration.
pub fn selection_match_extension(options: SelectionMatchConfig) -> Extension {
    Extension::group([MATCH_OPTIONS.of(options), MATCH_CONFIG.extension()])
}

/// Return the resolved options only when [`selection_match_extension`] is installed.
pub fn selection_match_options(
    state: &EditorState,
) -> Result<Option<HighlightSelectionOptions>, ConfigurationError> {
    Ok(state.context().field(&MATCH_CONFIG)?.as_deref().copied())
}

/// Project the configured marks in the supplied viewport. Return an empty
/// set when the extension is absent. The caller owns rendering.
pub fn selection_match_decorations(
    state: &EditorState,
    viewport: &Viewport,
) -> Result<RangeSet<MarkDecoration>, EditError> {
    let Some(options) = selection_match_options(state)? else {
        return Ok(RangeSet::empty());
    };
    highlight_selection_matches(state, viewport, options)
}

/// Calculate selection-match marks within the viewport's visible ranges.
/// Call again after a document, selection, or viewport change. A nonempty
/// selection is not itself marked. For cursor-word matching, the mark at
/// the cursor also has the `cm-selectionMatch-main` class.
/// Return an error if a visible range is not valid in this state.
pub fn highlight_selection_matches(
    state: &EditorState,
    viewport: &Viewport,
    options: HighlightSelectionOptions,
) -> Result<RangeSet<MarkDecoration>, EditError> {
    let text = state.text();
    if viewport
        .visible_ranges()
        .iter()
        .any(|range| !text.is_valid_range(range.clone()))
    {
        return Err(EditError::InvalidRange);
    }
    let selection = state.selection();
    if selection.ranges().len() != 1 {
        return Ok(RangeSet::empty());
    }
    let source = text.as_str();
    let range = selection.main().range();
    let category = (range.is_empty() || options.whole_words)
        .then(|| state.char_categorizer(selection.main().head()));
    let logical = uses_logical_selection_lines(state);
    let query = if range.is_empty() {
        if !options.highlight_word_around_cursor {
            return Ok(RangeSet::empty());
        }
        let Some(word) = state.word_range_at(range.start) else {
            return Ok(RangeSet::empty());
        };
        Cow::Borrowed(&source[word])
    } else {
        let selected = selection_text(&source[range.clone()], logical);
        let length = selected.encode_utf16().take(201).count();
        if length < options.min_selection_length || length > 200 {
            return Ok(RangeSet::empty());
        }
        if let Some(category) = &category
            && !(at_word_boundaries(source, &range, category)
                && word_unit_at(&selected, 0, Assoc::After, category)
                && word_unit_at(&selected, selected.len(), Assoc::Before, category))
        {
            return Ok(RangeSet::empty());
        }
        selected
    };

    let mut marks = Vec::new();
    let pattern = LiteralPattern::new(&query, false);
    for part in viewport.visible_ranges() {
        for found in SearchCursor::selected_text(text, part.clone(), pattern.clone(), logical) {
            let found = found.range;
            if category
                .as_ref()
                .is_some_and(|cat| !at_word_boundaries(source, &found, cat))
            {
                continue;
            }
            let class = if range.is_empty() && found.start <= range.start && found.end >= range.end
            {
                "cm-selectionMatch cm-selectionMatch-main"
            } else if found.start >= range.end || found.end <= range.start {
                "cm-selectionMatch"
            } else {
                continue;
            };
            if marks.len() == options.max_matches {
                return Ok(RangeSet::empty());
            }
            marks.push(MarkDecoration::new(class).range(found)?);
        }
    }
    Ok(RangeSet::of(marks, false).expect("visible ranges and matches are ordered"))
}

fn at_word_boundaries(source: &str, range: &Range<usize>, cat: &CharCategorizer) -> bool {
    !word_unit_at(source, range.start, Assoc::Before, cat)
        && !word_unit_at(source, range.end, Assoc::After, cat)
}

fn word_unit_at(source: &str, at: usize, side: Assoc, cat: &CharCategorizer) -> bool {
    let ch = match side {
        Assoc::Before => source[..at].chars().next_back(),
        Assoc::After => source[at..].chars().next(),
    };
    ch.is_some_and(|ch| {
        let mut buffer = [0; 2];
        let units = ch.encode_utf16(&mut buffer);
        let index = match side {
            Assoc::Before => units.len() - 1,
            Assoc::After => 0,
        };
        cat.is_word_unit(units[index])
    })
}
