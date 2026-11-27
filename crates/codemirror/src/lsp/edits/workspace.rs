use std::collections::BTreeSet;

use serde_json::Value;

use crate::{ChangeSet, Text};

use super::{LspError, TextEdit, decode_text_edits, text_edits};

/// Text and, if tracked by the caller, its LSP document version.
#[derive(Clone, Copy, Debug)]
pub struct DocumentSnapshot<'a> {
    pub text: &'a Text,
    pub version: Option<i32>,
}

/// Changes for one document. Apply them only to the snapshot used to plan them.
#[derive(Debug, Eq, PartialEq)]
pub struct DocumentChanges {
    pub uri: String,
    pub changes: ChangeSet,
}

/// A decoded text-only workspace edit. URIs are opaque identifiers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceEdit {
    pub(super) documents: Vec<TextDocumentEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TextDocumentEdit {
    pub uri: String,
    pub version: Option<i32>,
    pub edits: Vec<TextEdit>,
}

impl TryFrom<&Value> for WorkspaceEdit {
    type Error = LspError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let invalid = LspError::InvalidFormat("WorkspaceEdit");
        let object = value.as_object().ok_or_else(|| invalid.clone())?;
        let documents = if let Some(changes) = object.get("documentChanges") {
            let mut seen = BTreeSet::new();
            changes
                .as_array()
                .ok_or(invalid)?
                .iter()
                .map(|change| {
                    let (uri, version, edits) = document_change(change)?;
                    if !seen.insert(uri) {
                        return Err(LspError::RepeatedDocument(uri.into()));
                    }
                    Ok(TextDocumentEdit {
                        uri: uri.into(),
                        version,
                        edits: decode_text_edits(edits)?,
                    })
                })
                .collect::<Result<_, _>>()?
        } else if let Some(changes) = object.get("changes") {
            changes
                .as_object()
                .ok_or(invalid)?
                .iter()
                .map(|(uri, edits)| {
                    Ok(TextDocumentEdit {
                        uri: uri.clone(),
                        version: None,
                        edits: decode_text_edits(edits)?,
                    })
                })
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };
        Ok(Self { documents })
    }
}

impl WorkspaceEdit {
    /// Plan this batch against known snapshots. The lookup must not open files.
    /// A non-null wire version must match the corresponding snapshot version.
    /// An error returns no partial plan. Apply the edits as one batch and only
    /// if the snapshots are still current.
    pub fn plan<'a>(
        &self,
        mut document: impl FnMut(&str) -> Option<DocumentSnapshot<'a>>,
    ) -> Result<Vec<DocumentChanges>, LspError> {
        self.documents
            .iter()
            .map(|batch| {
                let snapshot = document(&batch.uri)
                    .ok_or_else(|| LspError::MissingDocument(batch.uri.clone()))?;
                check_version(&batch.uri, batch.version, snapshot.version)?;
                Ok(DocumentChanges {
                    uri: batch.uri.clone(),
                    changes: text_edits(snapshot.text, &batch.edits)?,
                })
            })
            .collect()
    }
}

pub(super) fn check_version(
    uri: &str,
    version: Option<i32>,
    snapshot: Option<i32>,
) -> Result<(), LspError> {
    if let Some(requested) = version
        && snapshot != Some(requested)
    {
        return Err(LspError::VersionMismatch {
            uri: uri.into(),
            requested,
            snapshot,
        });
    }
    Ok(())
}

/// Decode and plan a workspace edit without changing any documents or opening
/// files. Use `documentChanges` if present, otherwise use `changes`. Reject
/// resource operations, annotated edits, and repeated `documentChanges` URIs.
/// The lookup must only return snapshots that the host already trusts.
pub fn workspace_edit<'a>(
    edit: &Value,
    document: impl FnMut(&str) -> Option<DocumentSnapshot<'a>>,
) -> Result<Vec<DocumentChanges>, LspError> {
    WorkspaceEdit::try_from(edit)?.plan(document)
}

fn document_change(value: &Value) -> Result<(&str, Option<i32>, &Value), LspError> {
    if value.get("kind").is_some() {
        return Err(LspError::UnsupportedWorkspaceEdit("resource operations"));
    }
    let invalid = LspError::InvalidFormat("TextDocumentEdit");
    let id = value.get("textDocument").ok_or_else(|| invalid.clone())?;
    let uri = id
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid.clone())?;
    let version = match id.get("version") {
        Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| invalid.clone())?,
        ),
        None => return Err(invalid),
    };
    let edits = value.get("edits").ok_or(invalid)?;
    Ok((uri, version, edits))
}
