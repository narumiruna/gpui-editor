use serde_json::{Value, json};

use crate::{ChangeSet, Text};

use super::{LspError, PositionIndex, Range};

// @codemirror/lsp-client 6.3.1 uses a full update below this document size.
const FULL_SYNC_LIMIT: usize = 1024;

/// The server's `textDocumentSync` change mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SyncKind {
    #[default]
    None,
    Full,
    Incremental,
}

impl SyncKind {
    /// Read the change mode from `InitializeResult.capabilities`. Accept only
    /// absent or explicit `utf-16` position encoding. The upstream default
    /// does not advertise `general.positionEncodings`.
    pub fn from_server_capabilities(capabilities: &Value) -> Result<Self, LspError> {
        let object = capabilities
            .as_object()
            .ok_or(LspError::InvalidFormat("ServerCapabilities"))?;
        if let Some(encoding) = object.get("positionEncoding") {
            let encoding = encoding
                .as_str()
                .ok_or(LspError::InvalidFormat("positionEncoding"))?;
            if encoding != "utf-16" {
                return Err(LspError::UnsupportedPositionEncoding(encoding.into()));
            }
        }
        let sync = object.get("textDocumentSync");
        let change = match sync {
            Some(Value::Object(options)) => options.get("change"),
            _ => sync,
        };
        match change {
            None | Some(Value::Null) => Ok(Self::None),
            Some(change) => match change.as_u64() {
                Some(0) => Ok(Self::None),
                Some(1) => Ok(Self::Full),
                Some(2) => Ok(Self::Incremental),
                _ => Err(LspError::InvalidFormat("textDocumentSync")),
            },
        }
    }
}

/// Build a `textDocument/didChange` notification for a change set against
/// `before`. Supply the document version after the change.
///
/// Like upstream, send full text for a small result or in full sync mode.
/// Otherwise group adjacent changes and send ranges from right to left, so the
/// receiver can apply them in order without rebasing the original coordinates.
/// If an edit splits CRLF and has no LSP range, send full text instead.
/// Return no notification for an empty set or disabled sync.
pub fn did_change(
    uri: &str,
    version: i32,
    before: &Text,
    changes: &ChangeSet,
    sync: SyncKind,
) -> Result<Option<Value>, LspError> {
    if sync == SyncKind::None || changes.is_empty() {
        return Ok(None);
    }
    changes
        .check(before)
        .map_err(|_| LspError::InvalidChangeSet)?;
    let mut parts = Vec::new();
    let mut end = 0;
    let source = before.as_str();
    for (range, insert) in changes.changes() {
        parts.extend([&source[end..range.start], insert]);
        end = range.end;
    }
    parts.push(&source[end..]);

    let full = || vec![json!({"text": parts.concat()})];
    let events = if sync == SyncKind::Full || is_small(&parts) {
        full()
    } else {
        match incremental_events(before, changes) {
            Ok(events) => events,
            Err(LspError::InvalidByteOffset(_)) => full(),
            Err(error) => return Err(error),
        }
    };
    Ok(Some(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {"uri": uri, "version": version},
            "contentChanges": events,
        },
    })))
}

fn incremental_events(before: &Text, changes: &ChangeSet) -> Result<Vec<Value>, LspError> {
    let positions = PositionIndex::new(before);
    let mut events = Vec::new();
    let mut changes = changes.changes().peekable();
    while let Some((range, text)) = changes.next() {
        let mut end = range.end;
        let mut inserted = text.to_owned();
        while let Some((range, text)) = changes.next_if(|(range, _)| range.start == end) {
            end = range.end;
            inserted.push_str(text);
        }
        let range = Range {
            start: positions.position(range.start)?,
            end: positions.position(end)?,
        };
        events.push(json!({"range": range.to_json(), "text": inserted}));
    }
    events.reverse();
    Ok(events)
}

fn is_small(parts: &[&str]) -> bool {
    // The upstream UTF-16 document length counts CRLF as one line break.
    let mut units = 0;
    let mut prev_cr = false;
    for ch in parts.iter().flat_map(|part| part.chars()) {
        if !(prev_cr && ch == '\n') {
            units += ch.len_utf16();
            if units >= FULL_SYNC_LIMIT {
                return false;
            }
        }
        prev_cr = ch == '\r';
    }
    true
}
