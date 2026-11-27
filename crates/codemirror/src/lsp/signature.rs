//! Native signature help based on `@codemirror/lsp-client` 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.
//!
//! The host owns selection tracking, debounce, and RPC. Sync the document and
//! send its notifications before sending a request. When a selection is set
//! or a newer request starts, discard the outstanding reply. Use the matching
//! RPC ID and the same workspace when resolving. This module does not send
//! requests or render server text.

use std::{fmt, ops::Range, sync::Arc};

use serde_json::{Value, json};

use crate::{Assoc, MapMode};

use super::{DocumentWorkspace, LspError, PositionIndex, WorkspaceMapping, features::FeatureError};

mod data;

use data::SignatureData;
pub use data::{SignatureInformation, SignatureParameter};

const METHOD: &str = "textDocument/signatureHelp";
const CAPABILITY: &str = "signatureHelpProvider";

/// Signature help commands that a host can bind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignatureCommand {
    Show,
    Previous,
    Next,
}

/// The upstream default bindings. This does not register keys with a host.
pub const SIGNATURE_KEYMAP: [(&str, SignatureCommand); 3] = [
    ("Mod-Shift-Space", SignatureCommand::Show),
    ("Mod-Shift-ArrowUp", SignatureCommand::Previous),
    ("Mod-Shift-ArrowDown", SignatureCommand::Next),
];

/// The reason for a signature help request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignatureTrigger {
    Invoked,
    Character(String),
    /// Recheck open signature help after an explicit selection change.
    ContentChange,
}

impl SignatureTrigger {
    /// Find a trigger in fragments inserted by a typing event. Pass the
    /// fragments in document order. The last fragment with a match wins; in
    /// that fragment the last matching configured trigger wins. Retrigger
    /// characters only apply when signature help is open.
    pub fn automatic<'a>(
        inserted: impl IntoIterator<Item = &'a str>,
        active: bool,
        server_capabilities: &Value,
    ) -> Option<Self> {
        let provider = server_capabilities.get(CAPABILITY)?;
        let characters = |key| {
            provider
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|ch| !ch.is_empty())
        };
        let triggers = characters("triggerCharacters")
            .chain(characters("retriggerCharacters").filter(|_| active))
            .collect::<Vec<_>>();
        let mut selected = None;
        for fragment in inserted {
            if let Some(character) = triggers.iter().rfind(|&&ch| fragment.contains(ch)) {
                selected = Some(*character);
            }
        }
        selected.map(|ch| Self::Character(ch.into()))
    }

    fn context(&self, active: Option<&SignaturePlan>) -> Result<Value, LspError> {
        let mut context = json!({"isRetrigger": active.is_some()});
        context["triggerKind"] = match self {
            Self::Invoked => json!(1),
            Self::Character(ch) if !ch.is_empty() => {
                context["triggerCharacter"] = json!(ch);
                json!(2)
            }
            Self::ContentChange if active.is_some() => json!(3),
            _ => return Err(LspError::InvalidFormat("SignatureHelpContext")),
        };
        if let Some(active) = active {
            context["activeSignatureHelp"] = active.data.wire.clone();
        }
        Ok(context)
    }
}

/// A prepared request tied to the originating document and workspace.
#[derive(Debug)]
pub struct SignatureRequest {
    params: Value,
    trigger: SignatureTrigger,
    location: SignatureLocation,
}

/// Prepare a signature help request from a synchronized document. Pass the
/// currently displayed plan, if any, to send the server its prior response.
/// The host must check the server capability and match the request to its RPC ID.
/// The upstream debounce for a `ContentChange` request is 250 milliseconds.
pub fn signature_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
    trigger: SignatureTrigger,
    active: Option<&SignaturePlan>,
) -> Result<SignatureRequest, FeatureError> {
    let file = workspace
        .file(uri)
        .ok_or_else(|| LspError::MissingDocument(uri.into()))?;
    if file.has_pending_changes() {
        return Err(FeatureError::UnsyncedDocument(uri.into()));
    }
    if let Some(active) = active {
        active.location.check_uri(workspace, uri)?;
    }
    let position = PositionIndex::new(file.synced()).position(byte_offset)?;
    let params = json!({
        "textDocument": {"uri": uri},
        "position": {"line": position.line, "character": position.character},
        "context": trigger.context(active)?,
    });
    let location = SignatureLocation {
        uri: uri.into(),
        version: file.version(),
        offset: byte_offset,
        mapping: workspace.workspace_mapping(),
    };
    Ok(SignatureRequest {
        params,
        trigger,
        location,
    })
}

impl SignatureRequest {
    pub fn method(&self) -> &'static str {
        METHOD
    }

    pub fn capability(&self) -> &'static str {
        CAPABILITY
    }

    pub fn params(&self) -> &Value {
        &self.params
    }

    pub fn version(&self) -> i32 {
        self.location.version
    }

    /// Compare the result with the plan currently shown by the host, if any.
    /// The current overload may have changed since the request was prepared.
    /// Do not apply an effect for a canceled or superseded request.
    pub fn resolve(
        &self,
        workspace: &DocumentWorkspace,
        result: &Value,
        current: Option<&SignaturePlan>,
    ) -> Result<SignatureEffect, FeatureError> {
        self.location.anchor(workspace)?;
        if let Some(current) = current {
            current.location.check_uri(workspace, &self.location.uri)?;
        }
        let Some((data, mut active)) = SignatureData::parse(result)? else {
            return Ok(if current.is_some() {
                SignatureEffect::Clear
            } else {
                SignatureEffect::Keep
            });
        };
        let mut location = &self.location;
        if let Some(current) = current.filter(|current| current.data.same_labels(&data)) {
            if self.trigger == SignatureTrigger::ContentChange {
                active = current.active;
            }
            // Upstream compares the parameter index at the chosen overload,
            // not documentation or the previous overload index.
            if current.data.parameter_index(active) == data.parameter_index(active) {
                return Ok(SignatureEffect::Keep);
            }
            location = &current.location;
        }
        Ok(SignatureEffect::Show(SignaturePlan {
            location: location.clone(),
            version: self.version(),
            data: Arc::new(data),
            active,
        }))
    }
}

/// The host action for a resolved signature help request.
#[derive(Debug)]
pub enum SignatureEffect {
    Keep,
    Clear,
    Show(SignaturePlan),
}

/// One immutable display plan. Labels and documentation are inert server data.
#[derive(Clone, Debug)]
pub struct SignaturePlan {
    location: SignatureLocation,
    version: i32,
    data: Arc<SignatureData>,
    active: usize,
}

impl SignaturePlan {
    pub fn uri(&self) -> &str {
        &self.location.uri
    }

    /// The synchronized version from which this plan's response was obtained.
    pub fn version(&self) -> i32 {
        self.version
    }

    /// The tooltip anchor as a byte offset in the current draft. This maps
    /// edits before the anchor and refuses deleted anchors and closed files.
    pub fn anchor(&self, workspace: &DocumentWorkspace) -> Result<usize, FeatureError> {
        self.location.anchor(workspace)
    }

    pub fn signatures(&self) -> &[SignatureInformation] {
        &self.data.signatures
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn signature(&self) -> &SignatureInformation {
        &self.data.signatures[self.active]
    }

    pub fn active_parameter(&self) -> Option<&SignatureParameter> {
        self.signature()
            .parameters
            .get(self.data.parameter_index(self.active)?)
    }

    pub fn highlight(&self) -> Option<Range<usize>> {
        let span = self.active_parameter()?.byte_range.clone()?;
        (span.end != 0).then_some(span)
    }

    /// Advance without wrapping. At the last overload, keep it selected.
    pub fn next_signature(&self) -> Self {
        Self {
            active: (self.active + 1).min(self.data.signatures.len() - 1),
            ..self.clone()
        }
    }

    /// Go back without wrapping. At the first overload, keep it selected.
    pub fn prev_signature(&self) -> Self {
        Self {
            active: self.active.saturating_sub(1),
            ..self.clone()
        }
    }
}

#[derive(Clone)]
struct SignatureLocation {
    uri: String,
    offset: usize,
    version: i32,
    mapping: WorkspaceMapping,
}

impl SignatureLocation {
    fn check_uri(&self, workspace: &DocumentWorkspace, uri: &str) -> Result<(), FeatureError> {
        if self.uri != uri {
            return Err(self.stale(workspace));
        }
        self.anchor(workspace).map(|_| ())
    }

    fn anchor(&self, workspace: &DocumentWorkspace) -> Result<usize, FeatureError> {
        if !self.mapping.belongs_to(workspace) {
            return Err(self.stale(workspace));
        }
        self.mapping
            .map_offset(&self.uri, self.offset, Assoc::Before, MapMode::TrackDel)
            .ok_or_else(|| self.stale(workspace))
    }

    fn stale(&self, workspace: &DocumentWorkspace) -> FeatureError {
        FeatureError::StaleResponse {
            uri: self.uri.clone(),
            requested: self.version,
            current: workspace.file(&self.uri).map(|file| file.version()),
        }
    }
}

impl fmt::Debug for SignatureLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignatureLocation")
            .field("uri", &self.uri)
            .field("offset", &self.offset)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
