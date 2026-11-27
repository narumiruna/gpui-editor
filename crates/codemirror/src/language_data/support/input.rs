use std::ops::Range;

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorState, MapMode, RangeEdit, SelectionRange,
    Transaction, TransactionCoordinates,
    language::html::{html_input_is_active, tag_close_insert},
};

use super::{NativeLanguage, NativeLanguageSupport, projection::HtmlProjection};

impl NativeLanguageSupport {
    /// Build the default inherited HTML auto-close transaction for Jinja,
    /// Liquid, or PHP. The host must only call this for direct typing outside
    /// IME composition. Return `None` to let the host handle ordinary input.
    pub fn auto_close_tags(
        self,
        state: &EditorState,
        typed: &str,
    ) -> Result<Option<Transaction>, EditError> {
        self.auto_close_tags_in(state, 0..state.text().len(), typed)
    }

    /// Like [`Self::auto_close_tags`], but parse only `scope`. All selections
    /// must be inside the scope. The transaction uses full document byte
    /// offsets. Eligible input with an invalid UTF-8 scope returns an error.
    pub fn auto_close_tags_in(
        self,
        state: &EditorState,
        scope: Range<usize>,
        typed: &str,
    ) -> Result<Option<Transaction>, EditError> {
        let main = state.selection().main().range();
        if !matches!(
            self.language,
            NativeLanguage::Jinja | NativeLanguage::Liquid | NativeLanguage::Php
        ) || !matches!(typed, ">" | "/")
            || !main.is_empty()
            || state.read_only()
        {
            return Ok(None);
        }
        let Some(scoped_text) = state.text().slice_owned(scope.clone()) else {
            return Err(EditError::InvalidRange);
        };
        if state.selection().ranges().iter().any(|selection| {
            let range = selection.range();
            range.start < scope.start || range.end > scope.end
        }) {
            return Ok(None);
        }
        let Ok(before) = self.language.parse(scoped_text) else {
            return Ok(None);
        };
        let Some(before) = HtmlProjection::new(&before) else {
            return Ok(None);
        };
        let Some((_, position)) = before.at(main.start - scope.start, Assoc::Before) else {
            return Ok(None);
        };

        let base = state.replace_selection(typed)?;
        let mut text = state.text().clone();
        base.changes().apply_to(&mut text)?;
        let end = base.changes().map_pos(scope.end, Assoc::After);
        let scoped_text = text
            .slice_owned(scope.start..end)
            .ok_or(EditError::InvalidRange)?;
        let Ok(after) = self.language.parse(scoped_text) else {
            return Ok(None);
        };
        let Some(after) = HtmlProjection::new(&after) else {
            return Ok(None);
        };
        let post = EditorState {
            text,
            selection: base
                .selection()
                .expect("replace_selection sets selection")
                .clone(),
            ..EditorState::new("")
        };
        let main_after = after
            .at(post.selection().main().head() - scope.start, Assoc::Before)
            .map(|(_, head)| (after.document, head));
        let reverse = base.changes().desc(state.text().len())?.inverted();
        if !html_input_is_active(before.document, position, main_after, |start| {
            let source_start = scope.start + after.source(start..start + 1)?.start;
            let start = reverse
                .map_pos(source_start, Assoc::After, MapMode::Simple)?
                .checked_sub(scope.start)?;
            before.at(start, Assoc::After).map(|(_, head)| head)
        }) {
            return Ok(None);
        }
        let complete = post.change_by_range(|selection| {
            after.close_tag(post.text().as_str(), scope.start, typed, selection)
        })?;
        if complete.changes().is_empty() {
            return Ok(None);
        }
        Ok(Some(
            base.merge(
                complete
                    .with_user_event("input.complete")
                    .with_scroll_into_view(),
                state.text().len(),
                TransactionCoordinates::Sequential,
            )?,
        ))
    }
}

impl HtmlProjection<'_> {
    fn close_tag(
        &self,
        text: &str,
        offset: usize,
        typed: &str,
        selection: SelectionRange,
    ) -> Result<RangeEdit, EditError> {
        let mut edit = RangeEdit {
            changes: ChangeSet::default(),
            selection,
            effects: Vec::new(),
        };
        let head = selection.head();
        let local = head - offset;
        let Some((segment, projected)) = self.at(local, Assoc::Before) else {
            return Ok(edit);
        };
        // `</` must be adjacent in the source, not just in the compact tree.
        if typed == "/" && local - segment.source.start < 2 {
            return Ok(edit);
        }
        let Some(insert) = tag_close_insert(self.document, projected, typed, |name| {
            self.source(name).is_some()
        }) else {
            return Ok(edit);
        };
        if typed == "/" {
            let end = head + insert.len();
            edit.selection = SelectionRange::new(end, end);
        }
        let end = head
            + usize::from(local < segment.source.end && text.as_bytes().get(head) == Some(&b'>'));
        edit.changes = ChangeSet::single(Change::new(head..end, insert))?;
        Ok(edit)
    }
}
