//! Headless editor attachment based on `@codemirror/lsp-client` 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use serde_json::Value;

use crate::{
    Change, ChangeSet,
    view::{EditorView, PluginId, ViewException, ViewPlugin, ViewUpdate},
};

use super::{DocumentWorkspace, LspError, SyncKind};

/// The documents and editor attachments for one language server.
///
/// The host owns the RPC session and transport. Call [`Self::connected`] after
/// the session queues `initialized`. Flush before preparing or sending feature
/// requests. No editor callback performs I/O or starts a timer.
#[derive(Default)]
pub struct LspClient {
    state: Rc<RefCell<ClientState>>,
}

#[derive(Default)]
struct ClientState {
    workspace: DocumentWorkspace,
    sync: Option<SyncKind>,
    lifecycle: Vec<Value>,
}

impl LspClient {
    /// Attach an editor and take its current document as the opening snapshot.
    /// The host supplies the language ID, or an empty string if unknown. Each
    /// URI can have only one attached editor. Remove the returned plugin or drop
    /// the view to close the file and discard its pending changes.
    pub fn attach<M: Clone>(
        &mut self,
        view: &mut EditorView<M>,
        uri: impl Into<String>,
        language_id: impl Into<String>,
    ) -> Result<PluginId, LspError> {
        let uri = uri.into();
        let language_id = language_id.into();
        let mut state = self.state.borrow_mut();
        let open =
            state
                .workspace
                .open_file(uri.clone(), language_id, view.state().text().clone())?;
        state.queue_lifecycle(open);
        Ok(view.add_plugin(LspPlugin {
            state: Rc::downgrade(&self.state),
            uri,
        }))
    }

    /// Mark a newly initialized session ready and queue the existing snapshots
    /// for reopening. Keep pending edits for the next flush.
    pub fn connected(&mut self, capabilities: &Value) -> Result<(), LspError> {
        let mut state = self.state.borrow_mut();
        if state.sync.is_some() {
            return Err(LspError::AlreadyConnected);
        }
        let sync = SyncKind::from_server_capabilities(capabilities)?;
        state.lifecycle = state.workspace.connected().collect();
        state.sync = Some(sync);
        Ok(())
    }

    /// Forget the connection's notifications and invalidate all its request
    /// mappings. Keep attached editors and their drafts for a new session.
    pub fn disconnected(&mut self) {
        let mut state = self.state.borrow_mut();
        state.sync = None;
        state.lifecycle.clear();
        state.workspace.invalidate_mappings();
    }

    /// Advance the synchronized versions and take the ordered notifications.
    /// When disconnected, return no notifications and keep pending edits.
    ///
    /// Queue the entire returned batch in order before subsequent requests.
    /// On transport backpressure, retain the unsent suffix and retry it in order.
    /// On connection loss, discard the suffix and call [`Self::disconnected`].
    /// An error from this method does not advance versions or consume messages.
    pub fn flush(&mut self) -> Result<Vec<Value>, LspError> {
        let mut state = self.state.borrow_mut();
        let Some(kind) = state.sync else {
            return Ok(Vec::new());
        };
        let changes = state.workspace.sync(kind)?;
        let mut messages = std::mem::take(&mut state.lifecycle);
        messages.extend(changes);
        Ok(messages)
    }

    #[cfg(unix)]
    pub(super) fn take_lifecycle(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.state.borrow_mut().lifecycle)
    }

    /// Inspect the current drafts or use a native request or response adapter.
    /// Apply any returned edits after this callback.
    ///
    /// # Panics
    /// Editing or destroying an attached view in the callback can cause a
    /// borrow panic.
    pub fn with_workspace<T>(&self, inspect: impl FnOnce(&DocumentWorkspace) -> T) -> T {
        inspect(&self.state.borrow().workspace)
    }
}

impl ClientState {
    fn queue_lifecycle(&mut self, notification: Value) {
        if self.sync.is_some() {
            self.lifecycle.push(notification);
        }
    }
}

struct LspPlugin {
    state: Weak<RefCell<ClientState>>,
    uri: String,
}

impl ViewPlugin for LspPlugin {
    fn try_update(&mut self, update: &ViewUpdate<'_>) -> Result<(), ViewException> {
        if !update.doc_changed() {
            return Ok(());
        }
        let Some(state) = self.state.upgrade() else {
            return Ok(());
        };
        // View reports contain an accepted change description, not inserted
        // text. Use the corresponding ranges in the accepted result, not a
        // fresh diff that could change the mapping of response positions.
        let text = update.state.text();
        let changes = update
            .changes
            .changed_ranges()
            .map(|(old, new)| {
                text.slice_string(new)
                    .map(|insert| Change::new(old, insert))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| ViewException::new(LspError::InvalidChangeSet.to_string()))?;
        state
            .borrow_mut()
            .workspace
            .record_change(&self.uri, &ChangeSet::from_ordered_changes(changes))
            .map_err(|error| ViewException::new(error.to_string()))
    }

    fn destroy(&mut self) {
        if let Some(state) = self.state.upgrade() {
            let mut state = state.borrow_mut();
            if let Some(close) = state.workspace.close_file(&self.uri) {
                state.queue_lifecycle(close);
            }
        }
    }
}
