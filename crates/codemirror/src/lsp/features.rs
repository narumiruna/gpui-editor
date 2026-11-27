//! Native protocol features based on `@codemirror/lsp-client` 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.
//!
//! Send the workspace's sync notifications before the request. Use the current
//! RPC session or stdio transport to send the method and params. Match the RPC
//! ID before passing its result here. These helpers do not run a server or open
//! a URI. Server text and markup are data, not trusted display content.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    fmt,
    ops::Range as ByteRange,
};

use serde_json::{Value, json};
use unicode_segmentation::UnicodeSegmentation;

use crate::{Assoc, ChangeDesc, MapMode, RangeTouch, Text};

use super::{DocumentWorkspace, LspError, PositionIndex, Range, WorkspaceMapping};

mod completion;

pub use completion::{
    CompletionEditMode, CompletionEditPlan, CompletionItem, CompletionList, CompletionTrigger,
    InsertTextFormat, completion_request,
};

/// The feature could not use the request snapshot or response data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureError {
    Protocol(LspError),
    UnsyncedDocument(String),
    /// The query or an edit range is no longer valid, or an open file closed.
    /// The two versions can be equal if the edits have not reached the server.
    StaleResponse {
        uri: String,
        requested: i32,
        current: Option<i32>,
    },
    Unsupported(&'static str),
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(f),
            Self::UnsyncedDocument(uri) => write!(f, "sync {uri:?} before requesting features"),
            Self::StaleResponse {
                uri,
                requested,
                current,
            } => write!(
                f,
                "stale LSP response for {uri:?} (request version {requested}, current {current:?})"
            ),
            Self::Unsupported(kind) => write!(f, "unsupported LSP feature data: {kind}"),
        }
    }
}

impl std::error::Error for FeatureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LspError> for FeatureError {
    fn from(error: LspError) -> Self {
        Self::Protocol(error)
    }
}

type Decoder<T> = fn(&RequestDocument, &DocumentWorkspace, &Value) -> Result<T, FeatureError>;

/// One prepared request. Keep this with its RPC ID until it resolves.
#[derive(Debug)]
pub struct FeatureRequest<T> {
    method: &'static str,
    capability: &'static str,
    pub(super) params: Value,
    pub(super) document: RequestDocument,
    decode: Decoder<T>,
}

impl<T> FeatureRequest<T> {
    pub fn method(&self) -> &'static str {
        self.method
    }

    pub fn params(&self) -> &Value {
        &self.params
    }

    pub fn version(&self) -> i32 {
        self.document.version
    }

    /// The key in the server's initialize result capabilities object.
    pub fn capability(&self) -> &'static str {
        self.capability
    }

    /// Decode a successful result with the matching request ID. Use the same
    /// workspace as at preparation time. Do not pass a canceled result here.
    /// A JSON `null` is an empty result. Discard stale responses; to get a
    /// fresh result, issue a new request against the current document.
    pub fn resolve(
        &self,
        workspace: &DocumentWorkspace,
        result: &Value,
    ) -> Result<Option<T>, FeatureError> {
        if result.is_null() {
            return Ok(None);
        }
        (self.decode)(&self.document, workspace, result).map(Some)
    }
}

#[derive(Clone)]
pub(super) struct RequestDocument {
    pub(super) uri: String,
    cursor: usize,
    version: i32,
    text: Text,
    pub(super) mapping: WorkspaceMapping,
    pub(super) targets: BTreeMap<String, (i32, Text)>,
}

impl fmt::Debug for RequestDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestDocument")
            .field("uri", &self.uri)
            .field("cursor", &self.cursor)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl RequestDocument {
    pub(super) fn changes(
        &self,
        workspace: &DocumentWorkspace,
    ) -> Result<ChangeDesc, FeatureError> {
        if workspace.file(&self.uri).is_none() || !self.mapping.belongs_to(workspace) {
            return Err(self.stale(workspace));
        }
        self.mapping
            .get_mapping(&self.uri)
            .ok_or_else(|| stale(workspace, &self.uri, self.version))
    }

    fn stale(&self, workspace: &DocumentWorkspace) -> FeatureError {
        stale(workspace, &self.uri, self.version)
    }
}

pub(super) fn stale(workspace: &DocumentWorkspace, uri: &str, requested: i32) -> FeatureError {
    FeatureError::StaleResponse {
        uri: uri.into(),
        requested,
        current: workspace.file(uri).map(|file| file.version()),
    }
}

pub(super) fn position_request<T>(
    workspace: &DocumentWorkspace,
    uri: &str,
    cursor: usize,
    method: &'static str,
    capability: &'static str,
    decode: Decoder<T>,
) -> Result<FeatureRequest<T>, FeatureError> {
    let file = workspace
        .file(uri)
        .ok_or_else(|| LspError::MissingDocument(uri.into()))?;
    if file.has_pending_changes() {
        return Err(FeatureError::UnsyncedDocument(uri.into()));
    }
    let position = PositionIndex::new(file.synced()).position(cursor)?;
    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": position.line, "character": position.character },
    });
    let document = RequestDocument {
        uri: uri.into(),
        cursor,
        version: file.version(),
        text: file.synced().clone(),
        mapping: workspace.workspace_mapping(),
        targets: BTreeMap::new(),
    };
    Ok(FeatureRequest {
        method,
        capability,
        params,
        document,
        decode,
    })
}

/// Documentation supplied by the server. Do not interpret it as trusted HTML,
/// run code in it, open links, or load images based on these values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Documentation {
    PlainText(String),
    Markdown(String),
    Code { language: String, value: String },
}

pub(super) fn documentation(value: &Value, hover: bool) -> Result<Documentation, LspError> {
    if let Some(text) = value.as_str() {
        return Ok(if hover {
            Documentation::Markdown(text.into())
        } else {
            Documentation::PlainText(text.into())
        });
    }
    let text = string_field(value, "value", "documentation")?.into();
    match value.get("kind").and_then(Value::as_str) {
        Some("plaintext") => Ok(Documentation::PlainText(text)),
        Some("markdown") => Ok(Documentation::Markdown(text)),
        None if hover && value.get("kind").is_none() => Ok(Documentation::Code {
            language: string_field(value, "language", "MarkedString")?.into(),
            value: text,
        }),
        _ => Err(LspError::InvalidFormat("MarkupContent.kind")),
    }
}

pub(super) fn string_field<'a>(
    value: &'a Value,
    field: &str,
    name: &'static str,
) -> Result<&'a str, LspError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(LspError::InvalidFormat(name))
}

/// Typed hover content and its range in UTF-8 bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hover {
    pub range: ByteRange<usize>,
    pub contents: Vec<Documentation>,
}

/// Prepare a hover request for a synchronized document. A reply is stale if
/// the source document has changed or closed.
pub fn hover_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
) -> Result<FeatureRequest<Hover>, FeatureError> {
    position_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/hover",
        "hoverProvider",
        decode_hover,
    )
}

fn decode_hover(
    doc: &RequestDocument,
    workspace: &DocumentWorkspace,
    result: &Value,
) -> Result<Hover, FeatureError> {
    // The pinned hover tooltip is hidden when the source document changes.
    if !doc.changes(workspace)?.is_empty()
        || workspace.file(&doc.uri).map(|file| file.version()) != Some(doc.version)
    {
        return Err(doc.stale(workspace));
    }
    let range = result.get("range").map_or_else(
        || Ok(doc.cursor..doc.cursor),
        |range| byte_range(&PositionIndex::new(&doc.text), range.try_into()?),
    )?;
    let contents = result
        .get("contents")
        .ok_or(LspError::InvalidFormat("Hover.contents"))?;
    let contents = match contents {
        Value::Array(items) => items.iter().map(|item| documentation(item, true)).collect(),
        item => documentation(item, true).map(|item| vec![item]),
    }?;
    Ok(Hover { range, contents })
}

/// A definition, declaration, type definition, or implementation location.
/// Treat `uri` as opaque server data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefinitionTarget {
    pub uri: String,
    /// The server's original UTF-16 range.
    pub range: Range,
    /// The mapped byte range if this document was already in the workspace.
    /// This range applies to the draft at the time of response handling.
    pub byte_range: Option<ByteRange<usize>>,
}

/// Prepare a definition request for a synchronized document. Map known target
/// ranges through edits when they and the source word remain untouched. Keep
/// other targets in protocol coordinates. Return all locations in server
/// order; the upstream command jumps to the start of the first location.
/// Like upstream, this does not support `LocationLink`. The host checks the
/// server capability and decides whether to open a document or navigate.
pub fn definition_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
) -> Result<FeatureRequest<Vec<DefinitionTarget>>, FeatureError> {
    navigation_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/definition",
        "definitionProvider",
    )
}

/// Like [`definition_request`], for `textDocument/declaration`.
pub fn declaration_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
) -> Result<FeatureRequest<Vec<DefinitionTarget>>, FeatureError> {
    navigation_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/declaration",
        "declarationProvider",
    )
}

/// Like [`definition_request`], for `textDocument/typeDefinition`.
pub fn type_definition_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
) -> Result<FeatureRequest<Vec<DefinitionTarget>>, FeatureError> {
    navigation_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/typeDefinition",
        "typeDefinitionProvider",
    )
}

/// Like [`definition_request`], for `textDocument/implementation`.
pub fn implementation_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
) -> Result<FeatureRequest<Vec<DefinitionTarget>>, FeatureError> {
    navigation_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/implementation",
        "implementationProvider",
    )
}

fn navigation_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
    method: &'static str,
    capability: &'static str,
) -> Result<FeatureRequest<Vec<DefinitionTarget>>, FeatureError> {
    let mut request = position_request(
        workspace,
        uri,
        byte_offset,
        method,
        capability,
        decode_navigation,
    )?;
    request.document.targets = workspace
        .files()
        .iter()
        .map(|file| (file.uri().into(), (file.version(), file.synced().clone())))
        .collect();
    Ok(request)
}

fn decode_navigation(
    doc: &RequestDocument,
    workspace: &DocumentWorkspace,
    result: &Value,
) -> Result<Vec<DefinitionTarget>, FeatureError> {
    let changes = doc.changes(workspace)?;
    if changes.touches_range(word_at(&doc.text, doc.cursor)) != RangeTouch::None {
        return Err(doc.stale(workspace));
    }
    let items = result
        .as_array()
        .map_or(std::slice::from_ref(result), Vec::as_slice);
    let mut indices = BTreeMap::new();
    items
        .iter()
        .map(|item| {
            let uri = string_field(item, "uri", "Location.uri")?;
            let range: Range = item
                .get("range")
                .ok_or(LspError::InvalidFormat("Location.range"))?
                .try_into()?;
            check_order(range)?;
            let byte_range = if let Some((version, text)) = doc.targets.get(uri) {
                let (index, changes) = match indices.entry(uri) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        let changes = doc
                            .mapping
                            .get_mapping(uri)
                            .ok_or_else(|| stale(workspace, uri, *version))?;
                        entry.insert((PositionIndex::new(text), changes))
                    }
                };
                let bytes = byte_range(index, range)?;
                Some(mapped_range(changes, bytes).ok_or_else(|| stale(workspace, uri, *version))?)
            } else {
                None
            };
            Ok(DefinitionTarget {
                uri: uri.into(),
                range,
                byte_range,
            })
        })
        .collect()
}

pub(super) fn check_order(range: Range) -> Result<(), LspError> {
    let key = |pos: super::Position| (pos.line, pos.character);
    if key(range.start) > key(range.end) {
        return Err(LspError::ReversedRange(range));
    }
    Ok(())
}

pub(super) fn byte_range(
    index: &PositionIndex<'_>,
    range: Range,
) -> Result<ByteRange<usize>, LspError> {
    check_order(range)?;
    Ok(index.offset(range.start)?..index.offset(range.end)?)
}

pub(super) fn mapped_range(
    changes: &ChangeDesc,
    range: ByteRange<usize>,
) -> Option<ByteRange<usize>> {
    if changes.touches_range(range.clone()) != RangeTouch::None {
        return None;
    }
    let start = changes.map_pos(range.start, Assoc::After, MapMode::Simple)?;
    let end = changes.map_pos(range.end, Assoc::Before, MapMode::Simple)?;
    Some(start..end)
}

pub(super) fn word_at(text: &Text, cursor: usize) -> ByteRange<usize> {
    let (before, after) = text.as_str().split_at(cursor);
    let word = |part: &&str| part.chars().any(|ch| ch.is_alphanumeric() || ch == '_');
    let left = before
        .graphemes(true)
        .rev()
        .take_while(word)
        .map(str::len)
        .sum::<usize>();
    let right = after
        .graphemes(true)
        .take_while(word)
        .map(str::len)
        .sum::<usize>();
    cursor - left..cursor + right
}
