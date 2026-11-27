//! Reference requests and headless panel state from `@codemirror/lsp-client`
//! 6.3.1. See `docs/licenses/codemirror-lsp-client.txt`.
//!
//! The host owns synchronization, RPC, rendering, focus, and error reporting.
//! These helpers do not open files or run I/O.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    ops::Range as ByteRange,
};

use serde_json::{Value, json};

use crate::{ChangeDesc, EditorState};

use super::{
    DocumentWorkspace, LspError, Position, PositionIndex, Range,
    features::{
        FeatureError, FeatureRequest, RequestDocument, byte_range, check_order, mapped_range,
        position_request, stale, string_field,
    },
};

mod panel;

pub use panel::{
    ReferencePanel, ReferencePanelKeyAction, close_reference_panel, get_reference_panel,
};

/// Prepare the Find References command for an attached editor. Run this
/// after synchronization, using the editor state for `uri`. The host checks
/// attachment and server capability and sends the returned request.
pub fn find_references(
    state: &EditorState,
    workspace: &DocumentWorkspace,
    uri: &str,
) -> Result<FeatureRequest<Vec<ReferenceLocation>>, FeatureError> {
    references_request(workspace, uri, state.selection().main().head())
}

/// A reference in server order. The URI is opaque server data, not a path or
/// a request to open a document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceLocation {
    pub uri: String,
    /// The original UTF-16 range supplied by the server.
    pub range: Range,
    /// Only present for a document already open when the request was prepared.
    pub preview: Option<ReferencePreview>,
}

/// Display data for an open document. Use the strings as plain text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferencePreview {
    /// UTF-8 byte range in the draft at resolution time. Do not reuse after an
    /// edit without mapping the response again against the latest draft.
    pub byte_range: ByteRange<usize>,
    /// URI suffix after the directory prefix common to the previewed files.
    pub file_name: String,
    /// Show a file header here. Nonadjacent groups can repeat a header.
    pub starts_file_group: bool,
    /// One-based line number in the draft.
    pub line_number: u32,
    pub before: String,
    /// The part of the reference on its first line.
    pub matched: String,
    pub after: String,
}

/// Prepare a reference request at the primary selection head, in UTF-8 bytes.
/// The host must check `referencesProvider`, synchronize changed documents,
/// send the sync notifications in order, and match the RPC ID on reply.
/// As upstream, the request always includes the declaration.
///
/// A `null` result resolves to `None`. An empty array resolves to an empty
/// list. Keep order and duplicates. Locations unknown at request time have no
/// preview and do not affect file headers. A reply with no previews does not
/// supply any new panel content. Reject a non-null reply from a different
/// workspace, after the source closes, or when a known range is invalid, its
/// document closed, or local edits touch the range.
pub fn references_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
) -> Result<FeatureRequest<Vec<ReferenceLocation>>, FeatureError> {
    let mut request = position_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/references",
        "referencesProvider",
        decode_references,
    )?;
    request.params["context"] = json!({"includeDeclaration": true});
    request.document.targets = workspace
        .files()
        .iter()
        .map(|file| (file.uri().into(), (file.version(), file.synced().clone())))
        .collect();
    Ok(request)
}

fn decode_references(
    doc: &RequestDocument,
    workspace: &DocumentWorkspace,
    result: &Value,
) -> Result<Vec<ReferenceLocation>, FeatureError> {
    doc.changes(workspace)?;
    let items = result
        .as_array()
        .ok_or(LspError::InvalidFormat("Location[]"))?;
    let prefix = common_directory(items.iter().filter_map(|item| {
        item.get("uri")
            .and_then(Value::as_str)
            .filter(|uri| doc.targets.contains_key(*uri))
    }));
    let mut indices = BTreeMap::new();
    let mut previous_file = None;
    items
        .iter()
        .map(|item| {
            let uri = string_field(item, "uri", "Location.uri")?;
            let range: Range = item
                .get("range")
                .ok_or(LspError::InvalidFormat("Location.range"))?
                .try_into()?;
            check_order(range)?;
            let preview = if let Some((version, text)) = doc.targets.get(uri) {
                let target = match indices.entry(uri) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        let changes = doc
                            .mapping
                            .get_mapping(uri)
                            .ok_or_else(|| stale(workspace, uri, *version))?;
                        let draft = workspace
                            .file(uri)
                            .ok_or_else(|| stale(workspace, uri, *version))?
                            .draft();
                        entry.insert(Target {
                            synced: PositionIndex::new(text),
                            draft: PositionIndex::new(draft),
                            draft_text: draft.as_str(),
                            changes,
                        })
                    }
                };
                let bytes = byte_range(&target.synced, range)?;
                let bytes = mapped_range(&target.changes, bytes)
                    .ok_or_else(|| stale(workspace, uri, *version))?;
                let starts_file_group = previous_file.replace(uri) != Some(uri);
                Some(target.preview(bytes, &uri[prefix..], starts_file_group)?)
            } else {
                None
            };
            Ok(ReferenceLocation {
                uri: uri.into(),
                range,
                preview,
            })
        })
        .collect()
}

fn common_directory<'a>(mut uris: impl Iterator<Item = &'a str>) -> usize {
    let Some(first) = uris.next() else { return 0 };
    let first = first.as_bytes();
    let prefix = uris.fold(first, |prefix, uri| {
        let shared = prefix
            .iter()
            .zip(uri.bytes())
            .take_while(|(a, b)| **a == *b)
            .count();
        &prefix[..shared]
    });
    prefix
        .iter()
        .rposition(|&byte| byte == b'/')
        .map_or(0, |slash| slash + 1)
}

struct Target<'a> {
    synced: PositionIndex<'a>,
    draft: PositionIndex<'a>,
    draft_text: &'a str,
    changes: ChangeDesc,
}

impl Target<'_> {
    fn preview(
        &self,
        bytes: ByteRange<usize>,
        file_name: &str,
        starts_file_group: bool,
    ) -> Result<ReferencePreview, LspError> {
        let position = self.draft.position(bytes.start)?;
        let line_start = self.draft.offset(Position {
            character: 0,
            ..position
        })?;
        let line_end = self.draft_text[bytes.start..]
            .find(['\r', '\n'])
            .map_or(self.draft_text.len(), |i| bytes.start + i);
        let leading = &self.draft_text[line_start..bytes.start];
        let matched_end = bytes.end.min(line_end);
        let matched = &self.draft_text[bytes.start..matched_end];
        let trailing = &self.draft_text[matched_end..line_end];

        // Use the upstream UTF-16 preview window. If a limit is inside a
        // surrogate pair, omit the partial character to keep valid UTF-8.
        let column = position.character as usize;
        let before_units = column.min(50);
        let before_size = utf16_window(leading.chars().rev(), before_units);
        let after_units =
            (100 - before_units).saturating_sub(column + matched.encode_utf16().count());
        let after_size = utf16_window(trailing.chars(), after_units);
        Ok(ReferencePreview {
            byte_range: bytes,
            file_name: file_name.into(),
            starts_file_group,
            line_number: position.line + 1,
            before: leading[leading.len() - before_size..].into(),
            matched: matched.into(),
            after: trailing[..after_size].into(),
        })
    }
}

fn utf16_window(chars: impl Iterator<Item = char>, mut units: usize) -> usize {
    chars
        .take_while(|ch| {
            let Some(left) = units.checked_sub(ch.len_utf16()) else {
                return false;
            };
            units = left;
            true
        })
        .map(char::len_utf8)
        .sum()
}
