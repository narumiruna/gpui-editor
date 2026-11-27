use std::{collections::BTreeMap, fmt, ops::Range as ByteRange};

use serde_json::{Value, json};

use crate::{
    ChangeDesc, RangeTouch, Text,
    lsp::{
        DocumentWorkspace, PositionIndex, WorkspaceMapping,
        features::{FeatureError, word_at},
    },
};

use super::{
    DocumentChanges, LspError, TextEdit, WorkspaceEdit, decode_text_edits, text_edits,
    workspace::check_version,
};

type Decoder<T> = fn(&RequestSnapshot, &DocumentWorkspace, &Value) -> Result<T, FeatureError>;

/// A prepared formatting or rename request. Keep it with the matching RPC ID.
pub struct EditRequest<T> {
    method: &'static str,
    capability: &'static str,
    params: Value,
    snapshot: RequestSnapshot,
    decode: Decoder<T>,
}

impl<T> fmt::Debug for EditRequest<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditRequest")
            .field("method", &self.method)
            .field("uri", &self.snapshot.uri)
            .field("version", &self.version())
            .finish_non_exhaustive()
    }
}

impl<T> EditRequest<T> {
    pub fn method(&self) -> &'static str {
        self.method
    }

    pub fn params(&self) -> &Value {
        &self.params
    }

    pub fn version(&self) -> i32 {
        self.snapshot.source().version
    }

    /// The capability key in the server's initialize result.
    pub fn capability(&self) -> &'static str {
        self.capability
    }

    /// Plan a successful result with the matching RPC ID. Use the same
    /// workspace as at request time. A JSON `null` result gives no plan.
    /// This method does not change any file. Apply the plan as one batch only
    /// if the drafts are still current. Issue a fresh request if a reply or
    /// plan is stale.
    pub fn resolve(
        &self,
        workspace: &DocumentWorkspace,
        result: &Value,
    ) -> Result<Option<T>, FeatureError> {
        if result.is_null() {
            return Ok(None);
        }
        (self.decode)(&self.snapshot, workspace, result).map(Some)
    }
}

/// Options sent to `textDocument/formatting`. The host supplies indentation
/// settings. `tab_size` is the indentation width in columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormattingOptions {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

/// Prepare a whole-document format request. Send pending sync messages before
/// this request. Convert reply positions against the request snapshot and map
/// untouched ranges once. Reject the full result if any edit touches a change.
pub fn formatting_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    options: FormattingOptions,
) -> Result<EditRequest<DocumentChanges>, FeatureError> {
    if i32::try_from(options.tab_size).is_err() {
        return Err(LspError::InvalidFormat("FormattingOptions.tabSize").into());
    }
    Ok(EditRequest {
        method: "textDocument/formatting",
        capability: "documentFormattingProvider",
        params: json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": options.tab_size, "insertSpaces": options.insert_spaces },
        }),
        snapshot: RequestSnapshot::new(workspace, uri, false)?,
        decode: decode_formatting,
    })
}

/// Prepare a rename at the byte offset selected by the host. Send pending sync
/// messages before this request. The host supplies the name and position; the
/// pinned command uses the start of the selected word. No prompt, capability
/// check, or `prepareRename` request is performed here.
///
/// Only documents open at request time can be edited. Map versionless edits
/// through changes that do not touch their ranges. For versioned edits, the
/// request and current versions must match and the draft must be synchronized.
/// A stale source word or invalid edit rejects the full batch.
pub fn rename_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
    new_name: &str,
) -> Result<EditRequest<Vec<DocumentChanges>>, FeatureError> {
    let mut snapshot = RequestSnapshot::new(workspace, uri, true)?;
    let text = &snapshot.source().text;
    let position = PositionIndex::new(text).position(byte_offset)?;
    snapshot.query = Some(word_at(text, byte_offset));
    Ok(EditRequest {
        method: "textDocument/rename",
        capability: "renameProvider",
        params: json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
            "newName": new_name,
        }),
        snapshot,
        decode: decode_rename,
    })
}

struct RequestFile {
    version: i32,
    text: Text,
}

struct RequestSnapshot {
    uri: String,
    files: BTreeMap<String, RequestFile>,
    mapping: WorkspaceMapping,
    query: Option<ByteRange<usize>>,
}

impl RequestSnapshot {
    fn new(
        workspace: &DocumentWorkspace,
        uri: &str,
        include_targets: bool,
    ) -> Result<Self, FeatureError> {
        let file = workspace
            .file(uri)
            .ok_or_else(|| LspError::MissingDocument(uri.into()))?;
        if file.has_pending_changes() {
            return Err(FeatureError::UnsyncedDocument(uri.into()));
        }
        let files = workspace
            .files()
            .iter()
            .filter(|file| include_targets || file.uri() == uri)
            .map(|file| {
                (
                    file.uri().into(),
                    RequestFile {
                        version: file.version(),
                        text: file.synced().clone(),
                    },
                )
            })
            .collect();
        Ok(Self {
            uri: uri.into(),
            files,
            mapping: workspace.workspace_mapping(),
            query: None,
        })
    }

    fn source(&self) -> &RequestFile {
        &self.files[&self.uri]
    }

    fn mapping(
        &self,
        uri: &str,
        version: i32,
        workspace: &DocumentWorkspace,
    ) -> Result<ChangeDesc, FeatureError> {
        if !self.mapping.belongs_to(workspace) {
            return Err(stale(uri, version, workspace));
        }
        self.mapping
            .get_mapping(uri)
            .ok_or_else(|| stale(uri, version, workspace))
    }

    fn plan(
        &self,
        workspace: &DocumentWorkspace,
        uri: &str,
        version: Option<i32>,
        edits: &[TextEdit],
    ) -> Result<DocumentChanges, FeatureError> {
        let snapshot = self
            .files
            .get(uri)
            .ok_or_else(|| LspError::MissingDocument(uri.into()))?;
        check_version(uri, version, Some(snapshot.version))?;
        let file = workspace
            .file(uri)
            .ok_or_else(|| stale(uri, snapshot.version, workspace))?;
        check_version(uri, version, Some(file.version()))?;
        if version.is_some() && file.has_pending_changes() {
            return Err(stale(uri, snapshot.version, workspace));
        }
        let mapping = self.mapping(uri, snapshot.version, workspace)?;
        let changes = text_edits(&snapshot.text, edits)?;
        if changes
            .changes()
            .any(|(range, _)| mapping.touches_range(range.clone()) != RangeTouch::None)
        {
            return Err(stale(uri, snapshot.version, workspace));
        }
        let changes = if mapping.is_empty() {
            changes
        } else {
            changes
                .map(&mapping, false)
                .map_err(|_| LspError::InvalidChangeSet)?
        };
        changes
            .check(file.draft())
            .map_err(|_| LspError::InvalidChangeSet)?;
        Ok(DocumentChanges {
            uri: uri.into(),
            changes,
        })
    }
}

fn stale(uri: &str, requested: i32, workspace: &DocumentWorkspace) -> FeatureError {
    FeatureError::StaleResponse {
        uri: uri.into(),
        requested,
        current: workspace.file(uri).map(|file| file.version()),
    }
}

fn decode_formatting(
    snapshot: &RequestSnapshot,
    workspace: &DocumentWorkspace,
    value: &Value,
) -> Result<DocumentChanges, FeatureError> {
    snapshot.plan(workspace, &snapshot.uri, None, &decode_text_edits(value)?)
}

fn decode_rename(
    snapshot: &RequestSnapshot,
    workspace: &DocumentWorkspace,
    value: &Value,
) -> Result<Vec<DocumentChanges>, FeatureError> {
    let edit = WorkspaceEdit::try_from(value)?;
    let version = snapshot.source().version;
    let mapping = snapshot.mapping(&snapshot.uri, version, workspace)?;
    if let Some(query) = &snapshot.query
        && mapping.touches_range(query.clone()) != RangeTouch::None
    {
        return Err(stale(&snapshot.uri, version, workspace));
    }
    edit.documents
        .iter()
        .map(|batch| snapshot.plan(workspace, &batch.uri, batch.version, &batch.edits))
        .collect()
}
