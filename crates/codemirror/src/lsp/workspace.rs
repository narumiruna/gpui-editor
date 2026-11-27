//! Open documents and request position mapping, based on CodeMirror LSP 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock, Weak},
};

use serde_json::{Value, json};

use crate::{Assoc, ChangeDesc, ChangeSet, MapMode, Text};

use super::{DocumentSnapshot, LspError, Position, PositionIndex, SyncKind, did_change};

/// One open document. The server and the local editor can have different text
/// until [`DocumentWorkspace::sync`] emits the queued changes.
#[derive(Debug)]
pub struct WorkspaceFile {
    uri: String,
    language_id: String,
    version: i32,
    synced: Text,
    draft: Text,
    pending: ChangeSet,
}

impl WorkspaceFile {
    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn language_id(&self) -> &str {
        &self.language_id
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn synced(&self) -> &Text {
        &self.synced
    }

    pub fn draft(&self) -> &Text {
        &self.draft
    }

    pub fn has_pending_changes(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(super) fn pending_desc(&self) -> ChangeDesc {
        self.pending
            .desc(self.synced.len())
            .expect("valid pending changes")
    }

    /// The version and bytes of the document against which the server works.
    pub fn snapshot(&self) -> DocumentSnapshot<'_> {
        DocumentSnapshot {
            text: &self.synced,
            version: Some(self.version),
        }
    }

    fn did_open(&self) -> Value {
        notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": &self.uri,
                    "languageId": &self.language_id,
                    "text": self.synced.as_str(),
                    "version": self.version,
                },
            }),
        )
    }
}

struct MappedFile {
    start_doc: Text,
    changes: ChangeDesc,
}

type Mappings = RwLock<HashMap<String, MappedFile>>;

/// Map server positions from the last synchronized documents at creation time
/// to the latest drafts. Drop the handle after handling the response.
#[derive(Clone)]
pub struct WorkspaceMapping(Arc<Mappings>);

impl WorkspaceMapping {
    pub(super) fn belongs_to(&self, workspace: &DocumentWorkspace) -> bool {
        let identity = Arc::as_ptr(&self.0);
        workspace
            .mappings
            .lock()
            .expect("workspace mapping list lock is valid")
            .iter()
            .any(|mapping| std::ptr::eq(mapping.as_ptr(), identity))
    }

    pub fn get_mapping(&self, uri: &str) -> Option<ChangeDesc> {
        let files = self.0.read().expect("workspace mapping lock is valid");
        files.get(uri).map(|file| file.changes.clone())
    }

    /// Map an already converted UTF-8 byte offset from the request's document.
    /// Return `None` if the document was closed, the position is invalid, or a
    /// tracked position is deleted.
    pub fn map_offset(
        &self,
        uri: &str,
        offset: usize,
        assoc: Assoc,
        mode: MapMode,
    ) -> Option<usize> {
        let files = self.0.read().expect("workspace mapping lock is valid");
        let file = files.get(uri)?;
        file.start_doc.slice(offset..offset)?;
        file.changes.map_pos(offset, assoc, mode)
    }

    /// Convert a UTF-16 position against the request's document before mapping.
    /// A deleted tracked position returns `Ok(None)`. Closing and reopening the
    /// URI does not make this mapping valid again.
    pub fn map_position(
        &self,
        uri: &str,
        position: Position,
        assoc: Assoc,
        mode: MapMode,
    ) -> Result<Option<usize>, LspError> {
        let files = self.0.read().expect("workspace mapping lock is valid");
        let file = files
            .get(uri)
            .ok_or_else(|| LspError::MissingDocument(uri.into()))?;
        let offset = PositionIndex::new(&file.start_doc).offset(position)?;
        Ok(file.changes.map_pos(offset, assoc, mode))
    }
}

/// Document state for one language server. The host supplies the open documents,
/// reports accepted edits, and sends the returned notifications in order.
#[derive(Default)]
pub struct DocumentWorkspace {
    files: Vec<WorkspaceFile>,
    versions: HashMap<String, i32>,
    mappings: Mutex<Vec<Weak<Mappings>>>,
}

impl DocumentWorkspace {
    pub fn files(&self) -> &[WorkspaceFile] {
        &self.files
    }

    pub fn file(&self, uri: &str) -> Option<&WorkspaceFile> {
        self.files.iter().find(|file| file.uri == uri)
    }

    /// Open a file with one authoritative editor. A given URI starts at version
    /// zero and uses a higher version each time it reopens.
    pub fn open_file(
        &mut self,
        uri: impl Into<String>,
        language_id: impl Into<String>,
        doc: Text,
    ) -> Result<Value, LspError> {
        let uri = uri.into();
        if self.file(&uri).is_some() {
            return Err(LspError::DocumentAlreadyOpen(uri));
        }
        let version = self.next_version(&uri)?;
        let file = WorkspaceFile {
            uri: uri.clone(),
            language_id: language_id.into(),
            version,
            synced: doc.clone(),
            draft: doc,
            pending: ChangeSet::default(),
        };
        let message = file.did_open();
        self.files.push(file);
        self.versions.insert(uri, version);
        Ok(message)
    }

    /// Return a close notification only for an open document. Discard its draft
    /// changes and mark outstanding mappings stale for that document.
    pub fn close_file(&mut self, uri: &str) -> Option<Value> {
        let index = self.files.iter().position(|file| file.uri == uri)?;
        self.files.remove(index);
        self.mappings
            .get_mut()
            .expect("workspace mapping list lock is valid")
            .retain(|item| {
                let Some(mapping) = item.upgrade() else {
                    return false;
                };
                mapping
                    .write()
                    .expect("workspace mapping lock is valid")
                    .remove(uri);
                true
            });
        Some(notification(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        ))
    }

    /// Reopen all synchronized snapshots when the transport reconnects. Call
    /// [`Self::sync`] afterward to send edits not included in those snapshots.
    pub fn connected(&self) -> impl Iterator<Item = Value> + '_ {
        self.files.iter().map(WorkspaceFile::did_open)
    }

    /// Start mapping the positions returned by a request. Include changes that
    /// have already reached the editor but have not reached the server.
    pub fn workspace_mapping(&self) -> WorkspaceMapping {
        let entries = self
            .files
            .iter()
            .map(|file| {
                (
                    file.uri.clone(),
                    MappedFile {
                        start_doc: file.synced.clone(),
                        changes: file.pending_desc(),
                    },
                )
            })
            .collect();
        let mapping = Arc::new(RwLock::new(entries));
        let mut mappings = self
            .mappings
            .lock()
            .expect("workspace mapping list lock is valid");
        mappings.retain(|item| item.strong_count() != 0);
        mappings.push(Arc::downgrade(&mapping));
        WorkspaceMapping(mapping)
    }

    pub(super) fn invalidate_mappings(&mut self) {
        for mapping in self
            .mappings
            .get_mut()
            .expect("workspace mapping list lock is valid")
            .drain(..)
            .filter_map(|item| item.upgrade())
        {
            mapping
                .write()
                .expect("workspace mapping lock is valid")
                .clear();
        }
    }

    /// Record edits against the current draft. Invalid UTF-8 ranges or edits
    /// leave the file, pending changes, and request mappings unchanged.
    pub fn record_change(&mut self, uri: &str, changes: &ChangeSet) -> Result<(), LspError> {
        let file = self
            .files
            .iter_mut()
            .find(|file| file.uri == uri)
            .ok_or_else(|| LspError::MissingDocument(uri.into()))?;
        let mut draft = file.draft.clone();
        changes
            .apply_to(&mut draft)
            .map_err(|_| LspError::InvalidChangeSet)?;
        let pending = file
            .pending
            .compose(changes, file.synced.len())
            .map_err(|_| LspError::InvalidChangeSet)?;
        let desc = changes
            .desc(file.draft.len())
            .map_err(|_| LspError::InvalidChangeSet)?;
        let mut mapped = Vec::new();
        self.mappings
            .get_mut()
            .expect("workspace mapping list lock is valid")
            .retain(|link| {
                let Some(mapping) = link.upgrade() else {
                    return false;
                };
                let updated = mapping
                    .read()
                    .expect("workspace mapping lock is valid")
                    .get(uri)
                    .map(|entry| entry.changes.compose(&desc));
                if let Some(updated) = updated {
                    mapped.push((mapping, updated));
                }
                true
            });
        let mapped = mapped
            .into_iter()
            .map(|(mapping, result)| {
                result
                    .map(|changes| (mapping, changes))
                    .map_err(|_| LspError::InvalidChangeSet)
            })
            .collect::<Result<Vec<_>, _>>()?;
        file.draft = draft;
        file.pending = pending;
        for (mapping, changes) in mapped {
            let mut entries = mapping.write().expect("workspace mapping lock is valid");
            entries
                .get_mut(uri)
                .expect("mapped document is open")
                .changes = changes;
        }
        Ok(())
    }

    /// Advance each changed file's version once and return notifications in
    /// open order. `SyncKind::None` advances snapshots without emitting them,
    /// as in the upstream workspace. Errors do not consume pending changes.
    pub fn sync(&mut self, kind: SyncKind) -> Result<Vec<Value>, LspError> {
        let updates = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.has_pending_changes())
            .map(|(index, file)| {
                let version = self.next_version(&file.uri)?;
                let message = did_change(&file.uri, version, &file.synced, &file.pending, kind)?;
                Ok((index, version, message))
            })
            .collect::<Result<Vec<_>, LspError>>()?;
        let mut messages = Vec::new();
        for (index, version, message) in updates {
            let file = &mut self.files[index];
            file.synced = file.draft.clone();
            file.pending = ChangeSet::default();
            file.version = version;
            self.versions.insert(file.uri.clone(), version);
            messages.extend(message);
        }
        Ok(messages)
    }

    fn next_version(&self, uri: &str) -> Result<i32, LspError> {
        self.versions
            .get(uri)
            .copied()
            .unwrap_or(-1)
            .checked_add(1)
            .ok_or_else(|| LspError::DocumentVersionExhausted(uri.into()))
    }
}

fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}
