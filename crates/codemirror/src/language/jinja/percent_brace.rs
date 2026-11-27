//! Shared input handler from `@codemirror/lang-jinja` 6.0.1 and
//! `@codemirror/lang-liquid` 6.3.4. See their MIT notices in `docs/licenses/`.

use std::ops::Range;

use crate::{Change, ChangeSet, EditError, EditorState, RangeEdit, SelectionRange, Transaction};

/// Build the typing transaction for `%` at the main caret between `{}`.
///
/// Use this for text input, not paste. The host selects when to call it and
/// handles composition. Return `None` to let the host handle ordinary input.
/// When handled, replace every selection with `%%` and put each caret between
/// the percent signs, even when a secondary selection is not between braces.
pub fn close_percent_brace(
    state: &EditorState,
    typed: &str,
) -> Result<Option<Transaction>, EditError> {
    close_percent_brace_in(state, 0..state.text().len(), typed)
}

/// Like [`close_percent_brace`], within a host-selected UTF-8 byte range.
///
/// The scope must contain both braces at the main caret and all selection
/// ranges. Edits and selections use full-document positions. For `%` at an
/// empty main caret in an editable document, an invalid scope is an error.
pub fn close_percent_brace_in(
    state: &EditorState,
    scope: Range<usize>,
    typed: &str,
) -> Result<Option<Transaction>, EditError> {
    let main = state.selection().main().range();
    if typed != "%" || !main.is_empty() || state.read_only() {
        return Ok(None);
    }
    if !state.text().is_valid_range(scope.clone()) {
        return Err(EditError::InvalidRange);
    }
    let caret = main.start;
    if caret <= scope.start
        || caret >= scope.end
        || state.text().slice(caret - 1..caret + 1) != Some("{}")
        || state.selection().ranges().iter().any(|selection| {
            let range = selection.range();
            range.start < scope.start || range.end > scope.end
        })
    {
        return Ok(None);
    }

    let transaction = state.change_by_range(|selection| {
        let range = selection.range();
        let caret = range.start + 1;
        Ok(RangeEdit {
            changes: ChangeSet::single(Change::new(range, "%%"))?,
            selection: SelectionRange::new(caret, caret),
            effects: Vec::new(),
        })
    })?;
    Ok(Some(
        transaction
            .with_user_event("input.type")
            .with_scroll_into_view(),
    ))
}
