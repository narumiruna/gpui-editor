use std::ops::Range;

use gpui_editor_codemirror::{
    lint::Diagnostic,
    lsp::{
        rpc::{LspState, RpcMessage},
        server_diagnostics,
        transport::StdioClient,
    },
};
use serde_json::Value;

use super::{Editor, display_text};

const METHOD: &str = "textDocument/publishDiagnostics";
const MAX_ITEMS: usize = 128;
const MAX_MESSAGE_BYTES: usize = 2048;
const MAX_BATCH_MESSAGE_BYTES: usize = 16 * 1024;

pub(super) struct Diagnostics {
    enabled: bool,
    synced_revision: u64,
    version: i32,
    batch: Option<Batch>,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Scope {
    revision: u64,
    body: Range<usize>,
}

struct Batch {
    scope: Scope,
    diagnostics: Vec<Diagnostic>,
}

impl Editor {
    /// Accept versioned push diagnostics on the current trusted stdio attachment.
    /// Initially off on every attachment. Returns false if no client is attached.
    /// The host may advertise `textDocument.publishDiagnostics.versionSupport`
    /// when it starts the client. Still poll explicitly and notify after changes.
    pub fn set_source_lsp_diagnostics(&mut self, enabled: bool) -> bool {
        let Some(lsp) = &mut self.source_lsp else {
            return false;
        };
        lsp.diagnostics.enabled = enabled;
        if !enabled {
            lsp.diagnostics.batch = None;
        }
        true
    }

    /// The last accepted diagnostic batch for the current editable code body.
    /// Ranges use source bytes and messages are plain text; no actions are set.
    pub fn source_lsp_diagnostics(&self) -> &[Diagnostic] {
        self.source_lsp
            .as_ref()
            .and_then(|lsp| lsp.diagnostics.batch.as_ref())
            .filter(|batch| self.source_lsp_diagnostic_scope().as_ref() == Some(&batch.scope))
            .map_or(&[], |batch| batch.diagnostics.as_slice())
    }

    pub(in crate::editor) fn retain_source_lsp_diagnostics(&mut self) {
        if self
            .source_lsp
            .as_ref()
            .is_some_and(|lsp| lsp.diagnostics.batch.is_some())
        {
            let scope = self.source_lsp_diagnostic_scope();
            if let Some(lsp) = &mut self.source_lsp {
                lsp.diagnostics.retain(scope.as_ref());
            }
        }
    }

    pub(super) fn source_lsp_diagnostic_scope(&self) -> Option<Scope> {
        if !self.input_enabled || self.is_composing() || self.view.state().read_only() {
            return None;
        }
        Some(Scope {
            revision: self.view.revision(),
            body: self.source_fence()?.body,
        })
    }
}

impl Diagnostics {
    pub(super) fn new(open_revision: u64, open_version: i32) -> Self {
        Self {
            enabled: false,
            synced_revision: open_revision,
            version: open_version,
            batch: None,
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn retain(&mut self, scope: Option<&Scope>) {
        self.batch.take_if(|batch| Some(&batch.scope) != scope);
    }

    pub(super) fn observe_sync(&mut self, client: &StdioClient, uri: &str, editor: &Editor) {
        client.with_workspace(|workspace| {
            if let Some(file) = workspace.file(uri)
                && file.version() != self.version
                && !file.has_pending_changes()
                && file.synced() == editor.view.state().text()
            {
                self.version = file.version();
                self.synced_revision = editor.view.revision();
            }
        });
    }

    pub(super) fn receive(
        &mut self,
        client: &StdioClient,
        uri: &str,
        scope: &Scope,
        message: &RpcMessage,
    ) {
        if client.state() != LspState::Ready
            || scope.revision != self.synced_revision
            || !batch_fits(message, uri, self.version)
        {
            return;
        }
        let Some(publication) = client.with_workspace(|workspace| {
            let file = workspace.file(uri)?;
            if file.version() != self.version || file.has_pending_changes() {
                return None;
            }
            server_diagnostics(workspace, message).ok().flatten()
        }) else {
            return;
        };
        // A publication replaces the entire snapshot's diagnostics. Only the
        // editable code body has a source presentation in this Markdown editor.
        let diagnostics = publication
            .diagnostics
            .into_iter()
            .filter(|diagnostic| {
                scope.body.start <= diagnostic.range.start && diagnostic.range.end <= scope.body.end
            })
            .map(|mut diagnostic| {
                diagnostic.message = display_text(&diagnostic.message).unwrap_or_default();
                diagnostic.source = Some("LSP".into());
                diagnostic
            })
            .collect();
        self.batch = Some(Batch {
            scope: scope.clone(),
            diagnostics,
        });
    }
}

fn batch_fits(message: &RpcMessage, uri: &str, version: i32) -> bool {
    let RpcMessage::Notification {
        method,
        params: Some(params),
    } = message
    else {
        return false;
    };
    // A versionless message could describe an earlier draft even after sync.
    if method != METHOD
        || params.get("uri").and_then(Value::as_str) != Some(uri)
        || params.get("version").and_then(Value::as_i64) != Some(i64::from(version))
    {
        return false;
    }
    let Some(items) = params.get("diagnostics").and_then(Value::as_array) else {
        return false;
    };
    let mut remaining = MAX_BATCH_MESSAGE_BYTES;
    items.len() <= MAX_ITEMS
        && items.iter().all(|item| {
            let Some(message) = item.get("message").and_then(Value::as_str) else {
                return false;
            };
            let Some(left) = remaining.checked_sub(message.len()) else {
                return false;
            };
            remaining = left;
            message.len() <= MAX_MESSAGE_BYTES
        })
}

#[cfg(test)]
mod tests;
