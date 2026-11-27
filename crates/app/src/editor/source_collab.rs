//! Synchronous host collaboration for one existing unclosed code draft.

use gpui::Context;
use gpui_editor_codemirror::{
    ChangeSet, ConfigurationError, EditError, EditorState, Text,
    collab::{collab_state, receive_updates},
};

use super::Editor;
use crate::document::{Markdown, Section, SectionKind};

pub use gpui_editor_codemirror::collab::{CollabConfig, CollabError, Update, collab};

/// A snapshot of the native pending queue at its authority version.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceCollabQueue {
    pub version: usize,
    pub updates: Vec<Update>,
}

/// A rejected host operation. The editor and collaboration state are unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceCollabError {
    Collaboration(CollabError),
    InputBlocked,
    ImeUnsupported,
    EffectsUnsupported,
    OutsideDraft,
    ChangedTopology,
}

impl From<CollabError> for SourceCollabError {
    fn from(error: CollabError) -> Self {
        Self::Collaboration(error)
    }
}

impl From<EditError> for SourceCollabError {
    fn from(error: EditError) -> Self {
        CollabError::from(error).into()
    }
}

pub(super) struct SourceCollab {
    // The synchronized bytes are needed to check individual authority updates.
    // The view's text can include pending local edits, and composition of a
    // batch can hide an out-of-scope edit that a later update cancels.
    authority: Text,
    mapped_history: bool,
    ime_edited: bool,
}

impl SourceCollab {
    pub(super) fn install(state: &EditorState) -> Result<Option<Self>, ConfigurationError> {
        Ok(collab_state(state)?.map(|_| Self {
            authority: state.text().clone(),
            mapped_history: false,
            ime_edited: false,
        }))
    }

    pub(super) fn note_ime_edit(&mut self) {
        self.ime_edited = true;
    }
}

impl Editor {
    /// Read the native queue without acknowledging or transforming any update.
    /// Install `collab` with `with_native_extensions`. Keep that configuration
    /// for the lifetime of this editor. The host owns the authority and must
    /// submit at the returned version, in order, and only within this slice's
    /// supported draft body. The native queue also records other local edits;
    /// this API does not make them eligible for publication.
    ///
    /// IME composition is not part of this protocol. The host must reload from
    /// the authority to resume collaboration after a local marked-text edit.
    pub fn source_collab_queue(&self) -> Result<SourceCollabQueue, SourceCollabError> {
        self.source_collab_input()?;
        let session = collab_state(self.view.state())
            .map_err(EditError::from)?
            .ok_or(CollabError::NotConfigured)?;
        Ok(SourceCollabQueue {
            version: session.synced_version(),
            updates: session.sendable_updates().to_vec(),
        })
    }

    /// Receive an exact-base authority batch in whole-document UTF-8 bytes.
    /// Text from peers can change only one existing, unclosed code draft at
    /// the end of the document, outside other Markdown sections. Its opening
    /// line must already end in LF or CRLF. This does not require an active
    /// source frame. Matching echoes and empty batches need no draft body.
    /// No shared effects, IME, or transport are supported.
    pub fn receive_source_collab_updates(
        &mut self,
        version: usize,
        updates: &[Update],
        cx: &mut Context<Self>,
    ) -> Result<(), SourceCollabError> {
        let host = self.source_collab_input()?;
        if updates.iter().any(|update| !update.effects().is_empty()) {
            return Err(SourceCollabError::EffectsUnsupported);
        }
        let state = self.view.state();
        let transaction = receive_updates(state, version, updates)?;
        let session = collab_state(state)
            .map_err(EditError::from)?
            .ok_or(CollabError::NotConfigured)?;
        let mut optimistic = host.authority.clone();
        for pending in session.sendable_updates() {
            pending.apply_to(&mut optimistic)?;
        }
        if &optimistic != state.text() {
            return Err(CollabError::StateMismatch.into());
        }
        let mut authority = host.authority.clone();
        for update in updates {
            if update.client_id() == session.client_id() {
                // Core preparation already checked the echo against the queue.
                update.apply_to(&mut authority)?;
            } else {
                check_draft_edit(&mut authority, update.changes())?;
            }
        }
        let changed = check_draft_edit(&mut optimistic, transaction.changes())?;
        let before = self.active.clone();
        let mut after = before.clone();
        if let Some((old, new)) = &changed
            && before.as_ref() == Some(old)
        {
            after = Some(new.clone());
        }
        let proposed = transaction.clone();
        // Use the prepared transaction itself. Rebuilding or filtering it can
        // lose the typed receipt. Extenders may run, but cannot alter its edit,
        // selection, history policy, effects, or scroll behavior.
        self.active = self.view.update_checked(
            transaction,
            before,
            |resolved| {
                if resolved.changes() != proposed.changes()
                    || resolved.selection().is_some()
                    || !resolved.effects().is_empty()
                    || resolved.scroll_into_view()
                    || resolved.add_to_history()
                    || resolved.is_remote() != proposed.is_remote()
                    || resolved.history_isolation().is_some()
                {
                    return Err(EditError::Configuration(
                        ConfigurationError::InvalidFieldData,
                    ));
                }
                Ok(())
            },
            |_| (None, after),
        )?;
        let host = self
            .source_collab
            .as_mut()
            .expect("collaboration installed");
        host.authority = authority;
        if changed.is_some() {
            host.mapped_history = true;
            // Rebuild text layout without starting a reveal or moving focus.
            self.layout = None;
            self.rectangle = None;
            self.completion = None;
        }
        if !updates.is_empty() {
            cx.notify();
        }
        Ok(())
    }

    fn source_collab_input(&self) -> Result<&SourceCollab, SourceCollabError> {
        let host = self
            .source_collab
            .as_ref()
            .ok_or(CollabError::NotConfigured)?;
        if !self.input_enabled || self.view.state().read_only() || self.is_composing() {
            return Err(SourceCollabError::InputBlocked);
        }
        if host.ime_edited {
            return Err(SourceCollabError::ImeUnsupported);
        }
        Ok(host)
    }

    pub(super) fn restore_source_collab_frame(&self, active: Section) -> Section {
        if active.kind == SectionKind::DraftCode
            && self
                .source_collab
                .as_ref()
                .is_some_and(|host| host.mapped_history)
        {
            // Native history maps text and selection, not opaque host metadata.
            // Only refresh the restored unclosed draft's bounds. Keep synthetic
            // closed source frames and non-code metadata under the native rules.
            let source = self.source();
            let sections = self.markdown().sections();
            if let Some((draft, _)) = draft_body(source, &sections)
                && active.source.start == draft.source.start
                && active.content.start == draft.content.start
            {
                return draft.clone();
            }
        }
        active
    }
}

fn check_draft_edit(
    text: &mut Text,
    changes: &ChangeSet,
) -> Result<Option<(Section, Section)>, SourceCollabError> {
    if changes.is_empty() {
        return Ok(None);
    }
    let markdown = Markdown::new(text.as_str());
    let mut sections = markdown.sections();
    let (draft, start) =
        draft_body(text.as_str(), &sections).ok_or(SourceCollabError::OutsideDraft)?;
    for (range, insert) in changes.changes() {
        if range.start < start || range.end > draft.source.end {
            return Err(SourceCollabError::OutsideDraft);
        }
        let (actual, planned) = markdown
            .plan_replacement(range.clone(), insert, Some(draft))
            .map_err(|_| SourceCollabError::OutsideDraft)?;
        if actual != *range || planned != insert {
            return Err(SourceCollabError::OutsideDraft);
        }
    }
    let old = draft.clone();
    changes.apply_to(text)?;
    let expected = sections.last_mut().expect("checked draft section");
    expected.source.end = text.len();
    expected.content.end = text.len();
    let new = expected.clone();
    if sections != Markdown::new(text.as_str()).sections() {
        return Err(SourceCollabError::ChangedTopology);
    }
    Ok(Some((old, new)))
}

fn draft_body<'a>(source: &str, sections: &'a [Section]) -> Option<(&'a Section, usize)> {
    let (draft, before) = sections.split_last()?;
    if draft.kind != SectionKind::DraftCode
        || draft.source.end != source.len()
        || before.iter().any(|section| {
            section.kind == SectionKind::DraftCode || section.source.end > draft.source.start
        })
    {
        return None;
    }
    let (header, _) = source.get(draft.source.clone())?.split_once('\n')?;
    Some((draft, draft.source.start + header.len() + 1))
}

#[cfg(test)]
mod tests;
