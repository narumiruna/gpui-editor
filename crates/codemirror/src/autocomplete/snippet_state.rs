use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    ChangeDesc, EditorSelection, EditorState, Extension, StateEffect, StateEffectType, StateField,
    Transaction,
};

use super::SnippetSession;

type ActiveSnippet = Option<Arc<SnippetSession>>;

static SET_ACTIVE: LazyLock<StateEffectType<ActiveSnippet>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|active, changes| {
        // A deleted boundary maps to a clear effect, not an omitted effect.
        Some(map_snippet(active, changes))
    })
});

static SNIPPET_STATE: LazyLock<StateField<ActiveSnippet>> = LazyLock::new(|| {
    StateField::new(
        |_| Ok(None),
        |active, update| {
            let transaction = update.transaction();
            for effect in transaction.effects() {
                if let Some(next) = effect.get(&SET_ACTIVE) {
                    return Ok(next.clone());
                }
            }
            let active = map_snippet(active, update.changes());
            Ok(active.filter(|session| {
                transaction.selection().is_none_or(|selection| {
                    selection
                        .ranges()
                        .iter()
                        .all(|range| snippet_range_in_active_field(session, range.range()))
                })
            }))
        },
    )
});

fn map_snippet(active: &ActiveSnippet, changes: &ChangeDesc) -> ActiveSnippet {
    let active = active.as_ref()?;
    if changes.is_empty() {
        return Some(active.clone());
    }
    let mut mapped = (**active).clone();
    mapped.update(changes, None).ok()?.then(|| Arc::new(mapped))
}

/// Install snippet tracking for this editor.
pub fn snippet_state_extension() -> Extension {
    SNIPPET_STATE.extension()
}

/// Read the current snippet, if the extension is installed and a snippet is active.
pub fn active_snippet(state: &EditorState) -> Option<Arc<SnippetSession>> {
    let active = state.context().field(&SNIPPET_STATE).ok()??;
    active.as_ref().clone()
}

/// Start a session in the document that results from this transaction.
/// Attach the effect to the insertion transaction returned by `Snippet::insert`.
pub fn start_snippet(session: SnippetSession) -> StateEffect {
    SET_ACTIVE.of(session.active_field().map(|_| Arc::new(session)))
}

/// Test whether a replacement is contained in one occurrence of the active field.
/// Positions at either edge of the occurrence are included.
pub fn snippet_range_in_active_field(session: &SnippetSession, range: Range<usize>) -> bool {
    range.start <= range.end
        && session.active_field().is_some_and(|active| {
            session.fields()[active]
                .iter()
                .any(|field| field.start <= range.start && range.end <= field.end)
        })
}

/// Whether the next-field command can move the active selection.
pub fn has_next_snippet_field(state: &EditorState) -> bool {
    active_snippet(state).is_some_and(|session| {
        session
            .active_field()
            .is_some_and(|index| index + 1 < session.fields().len())
    })
}

/// Whether the previous-field command can move the active selection.
pub fn has_previous_snippet_field(state: &EditorState) -> bool {
    active_snippet(state)
        .is_some_and(|session| session.active_field().is_some_and(|index| index > 0))
}

/// Select the next field. Moving to the final field also clears the snippet.
pub fn next_snippet_field(state: &EditorState) -> Option<Transaction> {
    move_field(state, SnippetSession::next_field)
}

/// Select the previous field, if there is one.
pub fn previous_snippet_field(state: &EditorState) -> Option<Transaction> {
    move_field(state, SnippetSession::previous_field)
}

fn move_field(
    state: &EditorState,
    select: impl FnOnce(&mut SnippetSession) -> Option<EditorSelection>,
) -> Option<Transaction> {
    let mut session = (*active_snippet(state)?).clone();
    let selection = select(&mut session)?;
    Some(
        Transaction::default()
            .with_selection(selection)
            .with_effects([start_snippet(session)])
            .with_scroll_into_view()
            .with_add_to_history(false),
    )
}

/// Clear the active snippet without changing the document or the selection.
pub fn clear_snippet(state: &EditorState) -> Option<Transaction> {
    active_snippet(state)?;
    Some(
        Transaction::default()
            .with_effects([SET_ACTIVE.of(None)])
            .with_add_to_history(false),
    )
}
