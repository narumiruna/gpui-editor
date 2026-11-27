//! Text edit plans based on `@codemirror/lsp-client` 6.3.1.
//! See `docs/licenses/codemirror-lsp-client.txt`.

use serde_json::Value;

use crate::{Change, ChangeSet, EditError, Text};

use super::{LspError, PositionIndex, Range};

mod request;
mod workspace;

pub use request::{EditRequest, FormattingOptions, formatting_request, rename_request};
pub use workspace::{DocumentChanges, DocumentSnapshot, WorkspaceEdit, workspace_edit};

/// Replace one LSP range with text. Keep the inserted line endings as sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

impl TryFrom<&Value> for TextEdit {
    type Error = LspError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let invalid = LspError::InvalidFormat("TextEdit");
        if value.get("annotationId").is_some() {
            return Err(LspError::UnsupportedWorkspaceEdit("annotated text edits"));
        }
        Ok(Self {
            range: value
                .get("range")
                .ok_or_else(|| invalid.clone())?
                .try_into()?,
            new_text: value
                .get("newText")
                .and_then(Value::as_str)
                .ok_or(invalid)?
                .into(),
        })
    }
}

/// Convert simultaneous edits against the same snapshot. Reject overlapping
/// edits. Keep insertions at the same position in input order. If a replacement
/// starts at that position, it must follow those insertions in the input.
pub fn text_edits(text: &Text, edits: &[TextEdit]) -> Result<ChangeSet, LspError> {
    let positions = PositionIndex::new(text);
    let mut ranges = Vec::with_capacity(edits.len());
    for edit in edits {
        let start = positions.offset(edit.range.start)?;
        let end = positions.offset(edit.range.end)?;
        if start > end {
            return Err(LspError::ReversedRange(edit.range));
        }
        ranges.push((start..end, &edit.new_text));
    }
    ranges.sort_by_key(|(range, _)| range.start);
    if ranges
        .windows(2)
        .any(|pair| pair[0].0.end > pair[1].0.start)
    {
        return Err(LspError::OverlappingEdits);
    }
    change_set(
        ranges
            .into_iter()
            .map(|(range, text)| Change::new(range, text.as_str()))
            .collect(),
    )
}

fn change_set(changes: Vec<Change>) -> Result<ChangeSet, LspError> {
    ChangeSet::new(changes).map_err(|error| match error {
        EditError::OverlappingChanges => LspError::OverlappingEdits,
        _ => LspError::InvalidChangeSet,
    })
}

fn decode_text_edits(value: &Value) -> Result<Vec<TextEdit>, LspError> {
    value
        .as_array()
        .ok_or(LspError::InvalidFormat("TextEdit[]"))?
        .iter()
        .map(TextEdit::try_from)
        .collect()
}
