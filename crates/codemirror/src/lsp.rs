//! Native protocol helpers based on `@codemirror/lsp-client` 6.3.1.
//!
//! Positions use UTF-16 code units, as in the upstream default capabilities.
//! Native text and change sets keep their UTF-8 bytes and original line endings.
//! The RPC session supplies framing and request state. The document workspace
//! tracks open files and maps request positions. The host runs I/O and loads files.

use std::fmt;

mod client;
mod diagnostics;
mod edits;
pub mod features;
mod position;
pub mod references;
pub mod rpc;
pub mod signature;
mod support;
mod sync;
pub mod transport;
mod workspace;

pub use client::LspClient;
pub use diagnostics::{DiagnosticPublication, server_diagnostics};
pub use edits::{
    DocumentChanges, DocumentSnapshot, EditRequest, FormattingOptions, TextEdit, WorkspaceEdit,
    formatting_request, rename_request, text_edits, workspace_edit,
};
pub use position::{Position, PositionIndex, Range};
pub use support::{
    LanguageServerCommand, LanguageServerExtensions, language_server_extensions,
    language_server_support,
};
pub use sync::{SyncKind, did_change};
pub use workspace::{DocumentWorkspace, WorkspaceFile, WorkspaceMapping};

/// Invalid or unsupported LSP data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LspError {
    InvalidFormat(&'static str),
    InvalidPosition(Position),
    InvalidByteOffset(usize),
    PositionOverflow,
    ReversedRange(Range),
    OverlappingEdits,
    InvalidChangeSet,
    UnsupportedPositionEncoding(String),
    UnsupportedWorkspaceEdit(&'static str),
    MissingDocument(String),
    DocumentAlreadyOpen(String),
    DocumentVersionExhausted(String),
    AlreadyConnected,
    RepeatedDocument(String),
    VersionMismatch {
        uri: String,
        requested: i32,
        snapshot: Option<i32>,
    },
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(kind) => write!(f, "invalid LSP JSON for {kind}"),
            Self::InvalidPosition(pos) => write!(f, "invalid LSP position {pos:?}"),
            Self::InvalidByteOffset(offset) => {
                write!(f, "no LSP position for byte offset {offset}")
            }
            Self::PositionOverflow => f.write_str("position exceeds the LSP integer limit"),
            Self::ReversedRange(range) => write!(f, "reversed LSP range {range:?}"),
            Self::OverlappingEdits => f.write_str("overlapping or misordered LSP edits"),
            Self::InvalidChangeSet => f.write_str("change set does not fit the source text"),
            Self::UnsupportedPositionEncoding(encoding) => {
                write!(f, "unsupported LSP position encoding {encoding:?}")
            }
            Self::UnsupportedWorkspaceEdit(kind) => {
                write!(f, "unsupported LSP workspace edit: {kind}")
            }
            Self::MissingDocument(uri) => write!(f, "no document snapshot for {uri:?}"),
            Self::DocumentAlreadyOpen(uri) => write!(f, "document {uri:?} is already open"),
            Self::DocumentVersionExhausted(uri) => {
                write!(f, "no more versions for document {uri:?}")
            }
            Self::AlreadyConnected => f.write_str("disconnect the LSP client before reconnecting"),
            Self::RepeatedDocument(uri) => write!(f, "multiple edit batches for {uri:?}"),
            Self::VersionMismatch {
                uri,
                requested,
                snapshot,
            } => write!(
                f,
                "workspace edit for {uri:?} requires version {requested}, got {snapshot:?}"
            ),
        }
    }
}

impl std::error::Error for LspError {}
