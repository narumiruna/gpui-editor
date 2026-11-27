//! Push diagnostics from `@codemirror/lsp-client` 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.

use serde_json::Value;

use crate::{
    Assoc, MapMode,
    lint::{Diagnostic, Severity},
};

use super::{DocumentWorkspace, LspError, PositionIndex, features::byte_range, rpc::RpcMessage};

const METHOD: &str = "textDocument/publishDiagnostics";

/// Replace the diagnostics for one open document. Apply to the same draft
/// used to handle the notification, with [`crate::lint::set_diagnostics_for`].
/// An empty list clears the diagnostics. Messages are plain text.
#[derive(Debug)]
pub struct DiagnosticPublication {
    pub uri: String,
    /// The synchronized version used to interpret positions, even if omitted
    /// in the notification. It does not include changes pending in the draft.
    pub version: i32,
    pub diagnostics: Vec<Diagnostic>,
}

/// Handle a `publishDiagnostics` notification. Ignore other messages, unknown
/// documents, and versions that do not match the synchronized document.
/// Missing and null versions refer to the current synchronized snapshot.
/// Convert UTF-16 ranges there, then map both ends into the draft with left
/// association, even when edits touch the range. Reject malformed data for a
/// matching document without returning a partial publication.
pub fn server_diagnostics(
    workspace: &DocumentWorkspace,
    message: &RpcMessage,
) -> Result<Option<DiagnosticPublication>, LspError> {
    let RpcMessage::Notification { method, params } = message else {
        return Ok(None);
    };
    if method != METHOD {
        return Ok(None);
    }
    let params = params
        .as_ref()
        .ok_or(LspError::InvalidFormat("PublishDiagnosticsParams"))?;
    let uri = super::features::string_field(params, "uri", "PublishDiagnosticsParams.uri")?;
    let Some(file) = workspace.file(uri) else {
        return Ok(None);
    };
    if let Some(version) = params.get("version").filter(|value| !value.is_null()) {
        let version = version
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(LspError::InvalidFormat("PublishDiagnosticsParams.version"))?;
        if version != file.version() {
            return Ok(None);
        }
    }
    let items =
        params
            .get("diagnostics")
            .and_then(Value::as_array)
            .ok_or(LspError::InvalidFormat(
                "PublishDiagnosticsParams.diagnostics",
            ))?;
    let mut diagnostics = Vec::with_capacity(items.len());
    if !items.is_empty() {
        let index = PositionIndex::new(file.synced());
        let changes = file.pending_desc();
        let map = |pos| {
            changes
                .map_pos(pos, Assoc::Before, MapMode::Simple)
                .expect("diagnostic offset is in the synchronized document")
        };
        for item in items {
            let range = item
                .get("range")
                .ok_or(LspError::InvalidFormat("Diagnostic.range"))?
                .try_into()?;
            let range = byte_range(&index, range)?;
            let message = super::features::string_field(item, "message", "Diagnostic.message")?;
            let severity = match item.get("severity").filter(|value| !value.is_null()) {
                None => Severity::Error,
                Some(value) => match value.as_i64() {
                    Some(1) => Severity::Error,
                    Some(2) => Severity::Warning,
                    Some(3) => Severity::Info,
                    Some(_) => Severity::Hint,
                    None => return Err(LspError::InvalidFormat("Diagnostic.severity")),
                },
            };
            diagnostics.push(Diagnostic::new(
                map(range.start)..map(range.end),
                severity,
                message,
            ));
        }
    }
    Ok(Some(DiagnosticPublication {
        uri: uri.into(),
        version: file.version(),
        diagnostics,
    }))
}
