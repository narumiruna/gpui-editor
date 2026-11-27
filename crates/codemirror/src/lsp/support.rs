//! Headless feature bundles from `@codemirror/lsp-client` 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.

use std::time::Duration;

use serde_json::{Value, json};

use crate::{Text, view::keymap::KeyBinding};

use super::{
    DiagnosticPublication, DocumentWorkspace, LspError,
    rpc::RpcMessage,
    server_diagnostics,
    signature::{SIGNATURE_KEYMAP, SignatureCommand},
};

/// Commands supplied by the default bundle. The host runs them using the
/// native request adapters and its own interface. The [`super::references`]
/// module also supplies transactions for the headless reference panel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LanguageServerCommand {
    FormatDocument,
    RenameSymbol,
    JumpToDefinition,
    FindReferences,
    CloseReferencePanel,
    Signature(SignatureCommand),
}

/// The default LSP feature configuration. Completion, hover, formatting,
/// rename, navigation, references, and signature help use the native request
/// adapters. This bundle supplies their capabilities and default bindings. It
/// does not register event handlers, start timers, or run I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanguageServerExtensions {
    diagnostics: bool,
}

/// The current upstream bundle, including push diagnostics.
pub const fn language_server_extensions() -> LanguageServerExtensions {
    LanguageServerExtensions { diagnostics: true }
}

/// Open a document and return the bundle and the `didOpen` message. Like
/// upstream's deprecated `languageServerSupport`, this does not add push
/// diagnostics. To use the current bundle, configure it separately and call
/// [`DocumentWorkspace::open_file`] directly.
pub fn language_server_support(
    workspace: &mut DocumentWorkspace,
    uri: impl Into<String>,
    language_id: impl Into<String>,
    doc: Text,
) -> Result<(LanguageServerExtensions, Value), LspError> {
    let did_open = workspace.open_file(uri, language_id, doc)?;
    Ok((LanguageServerExtensions { diagnostics: false }, did_open))
}

impl LanguageServerExtensions {
    /// Native capabilities for the `capabilities` field in `initialize`.
    /// Unlike the browser client, this does not advertise a Markdown renderer,
    /// snippet application, or window message UI. A host can add its own.
    pub fn client_capabilities(self) -> Value {
        let mut text_document = json!({
            "completion": {
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["markdown", "plaintext"],
                    "insertReplaceSupport": false,
                },
                "completionList": {
                    "itemDefaults": ["commitCharacters", "editRange", "insertTextFormat"],
                },
                "completionItemKind": {"valueSet": []},
                "contextSupport": true,
            },
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "formatting": {},
            "rename": {},
            "signatureHelp": {
                "contextSupport": true,
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": {"labelOffsetSupport": true},
                    "activeParameterSupport": true,
                },
            },
            "definition": {},
            "declaration": {},
            "implementation": {},
            "typeDefinition": {},
            "references": {},
        });
        if self.diagnostics {
            text_document["publishDiagnostics"] = json!({"versionSupport": true});
        }
        json!({"textDocument": text_document})
    }

    /// Default editor bindings, with the high-priority signature keys first.
    /// Declaration, type definition, and implementation have no default keys.
    pub fn keymap(self) -> Vec<KeyBinding<LanguageServerCommand>> {
        use LanguageServerCommand as Command;

        SIGNATURE_KEYMAP
            .into_iter()
            .map(|(key, command)| KeyBinding::new(key, Command::Signature(command)))
            .chain(
                [
                    ("Shift-Alt-f", Command::FormatDocument, true),
                    ("F2", Command::RenameSymbol, true),
                    ("F12", Command::JumpToDefinition, true),
                    ("Shift-F12", Command::FindReferences, true),
                    ("Escape", Command::CloseReferencePanel, false),
                ]
                .map(|(key, command, prevent_default)| KeyBinding {
                    prevent_default,
                    ..KeyBinding::new(key, command)
                }),
            )
            .collect()
    }

    /// When diagnostics are enabled, wait this long after the last document
    /// edit before syncing. The host owns the timer and sends the changes.
    pub fn diagnostics_sync_delay(self) -> Option<Duration> {
        self.diagnostics.then_some(Duration::from_millis(500))
    }

    /// Dispatch a server message to the diagnostics adapter if enabled. Pass
    /// other server messages to the applicable host handler.
    pub fn receive_notification(
        self,
        workspace: &DocumentWorkspace,
        message: &RpcMessage,
    ) -> Result<Option<DiagnosticPublication>, LspError> {
        if self.diagnostics {
            server_diagnostics(workspace, message)
        } else {
            Ok(None)
        }
    }
}
