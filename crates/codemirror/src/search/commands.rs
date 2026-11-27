//! Native Find and Replace transactions from `@codemirror/search` 6.7.3.

use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Change, ChangeSet, EditorSelection, EditorState, Extension, Facet, SelectionRange, StateEffect,
    Transaction,
    view::{ANNOUNCE, SCROLL_INTO_VIEW, ScrollTarget},
};

use super::query_state::stored_query;

type MatchScroll = Arc<dyn Fn(SelectionRange, &EditorState) -> StateEffect + Send + Sync>;

static MATCH_SCROLL: LazyLock<Facet<MatchScroll, Option<MatchScroll>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&MatchScroll]| inputs.first().map(|input| (*input).clone()))
});

/// Configure the effect that Find Next, Find Previous, and Replace Next add
/// when they select a match. This does not install query state; use
/// [`super::extension`] for that. The first callback in extension precedence
/// order wins. Return [`SCROLL_INTO_VIEW`] with a custom [`ScrollTarget`] to
/// change the reveal strategy, or return a different effect. Without a
/// callback, the commands use the default scroll target.
///
/// The callback receives the start state and the proposed selection range in
/// post-change UTF-8 byte offsets. It runs once before transaction filters.
/// The returned effect replaces the default scroll effect and goes through
/// normal effect mapping and filtering. Other search commands do not use it.
pub fn scroll_to_match(
    effect: impl Fn(SelectionRange, &EditorState) -> StateEffect + Send + Sync + 'static,
) -> Extension {
    MATCH_SCROLL.of(Arc::new(effect))
}

fn match_scroll_effect(state: &EditorState, range: SelectionRange) -> StateEffect {
    let callback = state
        .context()
        .facet(&MATCH_SCROLL)
        .expect("valid match scroll facet");
    match callback.as_ref() {
        Some(effect) => effect(range, state),
        None => SCROLL_INTO_VIEW.of(ScrollTarget::new(range)),
    }
}

/// Select the next match of the stored query, wrapping once. The host owns
/// panel opening, focus, scrolling and delivery of the announcement effect.
pub fn find_next(state: &EditorState) -> Option<Transaction> {
    let query = stored_query(state)?;
    let at = state.selection().main().range().end;
    let matched = query.in_state(state).find_next(at..at).ok()??;
    let range = matched.range;
    Some(select_match(state, range))
}

/// Select the previous match of the stored query, wrapping once.
pub fn find_previous(state: &EditorState) -> Option<Transaction> {
    let query = stored_query(state)?;
    let at = state.selection().main().range().start;
    let matched = query.in_state(state).find_previous(at..at).ok()??;
    Some(select_match(state, matched.range))
}

fn select_match(state: &EditorState, range: Range<usize>) -> Transaction {
    let announcement = match_announcement(state, &range);
    let selection = EditorSelection::single(range.start, range.end);
    let scroll = match_scroll_effect(state, selection.main());
    Transaction::default()
        .with_selection(selection)
        .with_effects([announcement, scroll])
        .with_user_event("select.search")
}

/// Replace the selected, precise match and move to the next one. If the
/// current selection is not a match, select the match without changing text.
/// A read-only state returns no transaction.
pub fn replace_next(state: &EditorState) -> Option<Transaction> {
    if state.read_only() {
        return None;
    }
    let query = stored_query(state)?;
    let bound = query.in_state(state);
    let selection = state.selection().main().range();
    let at = selection.start;
    let mut next = bound.find_next(at..at).ok()??;
    let mut changes = ChangeSet::default();
    let mut effects = Vec::new();
    if !next.precise || next.range == selection {
        if next.precise {
            effects.push(replacement_announcement(state, at));
            changes = ChangeSet::single(Change::new(
                next.range.clone(),
                query.replacement(&next, query.replacement_text()),
            ))
            .ok()?;
        }
        let found = bound.find_next(next.range).ok()?;
        let Some(following) = found else {
            return Some(
                Transaction::new(changes)
                    .with_user_event("input.replace")
                    .with_effects(effects),
            );
        };
        next = following;
    }
    effects.push(match_announcement(state, &next.range));
    let selected = EditorSelection::single(next.range.start, next.range.end).map(&changes);
    effects.push(match_scroll_effect(state, selected.main()));
    Some(
        Transaction::new(changes)
            .with_selection(selected)
            .with_effects(effects)
            .with_user_event("input.replace"),
    )
}

/// Replace all precise, non-overlapping matches in one transaction. A
/// read-only state or missing precise matches yield no transaction.
pub fn replace_all(state: &EditorState) -> Option<Transaction> {
    if state.read_only() {
        return None;
    }
    let query = stored_query(state)?;
    let (changes, count) = query
        .in_state(state)
        .replacements(query.replacement_text())
        .ok()?;
    if count == 0 {
        return None;
    }
    Some(
        Transaction::new(changes)
            .with_effects([replacements_announcement(state, count)])
            .with_user_event("input.replace.all"),
    )
}

/// Announce a replacement on the line containing `at` in the start state.
/// `at` must be a valid byte position in the document.
pub fn replacement_announcement(state: &EditorState, at: usize) -> StateEffect {
    let line = state
        .text()
        .line_at(at)
        .expect("replacement is in the document");
    ANNOUNCE.of(state.phrase("replaced match on line $", &[&line.number.to_string()]) + ".")
}

/// Announce the number of matches replaced, before adjacent edits are combined.
pub fn replacements_announcement(state: &EditorState, count: usize) -> StateEffect {
    ANNOUNCE.of(state.phrase("replaced $ matches", &[&count.to_string()]) + ".")
}

const MARGIN: usize = 30;

/// Announce a match in the start state, including its bounded line context.
/// `range` must use valid byte positions in the document.
pub fn match_announcement(state: &EditorState, range: &Range<usize>) -> StateEffect {
    let text = state.text();
    let source = text.as_str();
    let first = text.line_at(range.start).expect("match is in the document");
    let last = text.line_at(range.end).expect("match is in the document");
    let before = &source[first.range.start..range.start];
    // A native CRLF can be addressed between the bytes. A line's content
    // stops before that pair, so the right-hand context can be empty here.
    let after = source.get(range.end..last.range.end).unwrap_or_default();
    let start = range.start - margin_bytes(before.chars().rev());
    let end = range.end + margin_bytes(after.chars());
    let mut excerpt = &source[start..end];
    if start != first.range.start {
        let mut units = 0;
        if let Some((pos, _)) = excerpt.char_indices().find(|&(pos, ch)| {
            let at = units;
            units += ch.len_utf16();
            at < MARGIN && is_break(ch) && !excerpt[pos + ch.len_utf8()..].starts_with(is_break)
        }) {
            excerpt = &excerpt[pos..];
        }
    }
    if end < last.range.end {
        let mut units = 0;
        if let Some((pos, _)) = excerpt.char_indices().rev().find(|&(pos, ch)| {
            units += ch.len_utf16();
            units < MARGIN && is_break(ch) && !excerpt[..pos].ends_with(is_break)
        }) {
            excerpt = &excerpt[..pos];
        }
    }
    ANNOUNCE.of(format!(
        "{}. {excerpt} {} {}.",
        state.phrase("current match", &[]),
        state.phrase("on line", &[]),
        first.number
    ))
}

fn margin_bytes(chars: impl Iterator<Item = char>) -> usize {
    let mut units = MARGIN;
    // Keep complete native scalars when upstream's cut splits a pair.
    chars
        .take_while(|ch| {
            let Some(left) = units.checked_sub(ch.len_utf16()) else {
                return false;
            };
            units = left;
            true
        })
        .map(char::len_utf8)
        .sum()
}

fn is_break(ch: char) -> bool {
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}' || ".,:;?!".contains(ch)
}
