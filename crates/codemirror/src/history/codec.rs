use serde_json::{Value, json};

use crate::{
    Change, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    SerializationError, Text,
};

use super::{
    History,
    branch::{Branch, Entry, map_selections},
};

const INVALID: SerializationError = SerializationError::InvalidFormat("history");

impl<M: Clone> History<M> {
    /// Encode the two history branches, not the current editor state.
    ///
    /// Uses CodeMirror's `done` and `undone` format with native UTF-8 byte
    /// offsets, not JavaScript UTF-16 code units. A native event also stores
    /// its post-change selection in `oppositeSelection`. If the property is
    /// absent, replay infers the selection the same way CodeMirror does.
    /// Runtime effects, host metadata, and group timing are not stored.
    /// Without a configured history policy, empty selection-only entries are omitted.
    pub fn to_json(&self) -> Result<Value, SerializationError> {
        let length = self.state.text().len();
        let keep_empty_base = self.state.configured_history().is_some();
        Ok(json!({
            "done": encode(&self.undo, length, keep_empty_base)?,
            "undone": encode(&self.redo, length, keep_empty_base)?,
        }))
    }

    /// Restore history for the exact current document and selection supplied
    /// in `state`. Every restored change event uses the supplied host `metadata`
    /// for both undo and redo. Runtime effects are not restored. An event
    /// that held only effects can still restore its saved selection once.
    pub fn from_json(
        state: EditorState,
        value: &Value,
        metadata: M,
    ) -> Result<Self, SerializationError> {
        let undo = decode(value.get("done").ok_or(INVALID)?, state.text(), &metadata)?;
        let redo = decode(value.get("undone").ok_or(INVALID)?, state.text(), &metadata)?;
        let mut history = Self::new(state);
        history.undo = undo;
        history.redo = redo;
        Ok(history)
    }

    /// Encode the document, selection, and both branches with the `history` key.
    /// This does not encode other extension state. For custom field names or
    /// extension fields, use [`Self::to_json`] and the editor state codecs separately.
    pub fn to_state_json(&self) -> Result<Value, SerializationError> {
        let mut state = self.state.to_json();
        state["history"] = self.to_json()?;
        Ok(state)
    }

    /// Restore a document, selection, and optional `history` key. If `history`
    /// is absent, both branches start empty. Install `extensions` with their
    /// ordinary initial values. See [`Self::from_json`] for replay metadata.
    pub fn from_state_json(
        value: &Value,
        extensions: Extension,
        metadata: M,
    ) -> Result<Self, SerializationError> {
        let state = EditorState::from_json_with_fields(value, extensions, &[])?;
        if let Some(history) = value.get("history") {
            Self::from_json(state, history, metadata)
        } else {
            Ok(Self::new(state))
        }
    }
}

fn selection_values(selections: &[EditorSelection]) -> Vec<Value> {
    selections.iter().map(EditorSelection::to_json).collect()
}

fn encode<M: Clone>(
    branch: &Branch<M>,
    mut length: usize,
    keep_empty_base: bool,
) -> Result<Vec<Value>, SerializationError> {
    let mut values = Vec::new();
    for entry in branch.entries.iter().rev() {
        let changes = entry.changes.desc(length)?;
        let mut value = json!({
            "changes": entry.changes.to_json(length)?,
            "startSelection": entry.selection.to_json(),
            "selectionsAfter": selection_values(&entry.selections_after),
        });
        if let Some(mapping) = &entry.mapped {
            value["mapped"] = mapping.to_json();
        }
        if let Some(selection) = &entry.opposite_selection {
            value["oppositeSelection"] = selection.to_json();
        }
        length = older_length(&changes, entry.mapped.as_ref())?;
        values.push(value);
    }
    if let Some(selections) = &branch.base_selections
        && (keep_empty_base || !selections.is_empty())
    {
        values.push(json!({"selectionsAfter": selection_values(selections)}));
    }
    values.reverse();
    Ok(values)
}

fn decode<M: Clone>(
    value: &Value,
    text: &Text,
    metadata: &M,
) -> Result<Branch<M>, SerializationError> {
    let values = value.as_array().ok_or(INVALID)?;
    let mut branch = Branch::default();
    let mut source = text.clone();
    let mut length = text.len();
    let mut pending = None;
    for (index, value) in values.iter().enumerate().rev() {
        let selections = value
            .get("selectionsAfter")
            .and_then(Value::as_array)
            .ok_or(INVALID)?
            .iter()
            .map(EditorSelection::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let Some(changes) = value.get("changes") else {
            if index != 0 || value.get("mapped").is_some() || value.get("startSelection").is_some()
            {
                return Err(INVALID);
            }
            let mapped;
            let replay = if let Some(mapping) = &pending {
                mapped = map_selections(&selections, mapping)?;
                &mapped
            } else {
                &selections
            };
            for selection in replay {
                selection.check(|pos| source.is_char_boundary(pos))?;
            }
            branch.base_selections = Some(selections);
            break;
        };
        let changes = if pending.is_none() {
            ChangeSet::from_json(changes, &source)?
        } else {
            decode_deferred_changes(changes, length)?
        };
        let desc = changes.desc(length)?;
        let selection = EditorSelection::from_json(value.get("startSelection").ok_or(INVALID)?)?;
        let opposite_selection = value
            .get("oppositeSelection")
            .map(EditorSelection::from_json)
            .transpose()?;
        let mapped = value.get("mapped").map(ChangeDesc::from_json).transpose()?;
        length = older_length(&desc, mapped.as_ref())?;
        let entry = Entry {
            changes,
            effects: Vec::new(),
            selection,
            opposite_selection,
            before: metadata.clone(),
            after: metadata.clone(),
            selections_after: selections,
            mapped,
        };

        // Validate the changes that can actually be replayed. Older events can
        // refer to text removed by a deferred mapping. Apply that mapping before
        // checking their offsets against actual UTF-8 text.
        let mapped_entry = pending
            .as_ref()
            .map(|mapping| entry.map(mapping))
            .transpose()?;
        let replay = mapped_entry.as_ref().unwrap_or(&entry);
        for selection in replay
            .selections_after
            .iter()
            .chain(replay.opposite_selection.as_ref())
        {
            selection.check(|pos| source.is_char_boundary(pos))?;
        }
        replay.changes.apply_to(&mut source)?;
        replay.selection.check(|pos| source.is_char_boundary(pos))?;
        pending = replay.mapped.clone();
        branch.entries.push_front(entry);
    }
    Ok(branch)
}

fn older_length(
    changes: &ChangeDesc,
    mapping: Option<&ChangeDesc>,
) -> Result<usize, SerializationError> {
    match mapping {
        Some(mapping) if mapping.new_length() == changes.new_length() => Ok(mapping.length()),
        Some(_) => Err(EditError::MismatchedLength.into()),
        None => Ok(changes.new_length()),
    }
}

// The shared ChangeSet codec requires the original text. An older entry with a
// pending mapping may refer to a document whose removed text is not available.
fn decode_deferred_changes(value: &Value, length: usize) -> Result<ChangeSet, SerializationError> {
    let mut changes = Vec::new();
    let mut old = 0usize;
    for part in value.as_array().ok_or(INVALID)? {
        let (size, lines) = if let Some(parts) = part.as_array() {
            let (size, lines) = parts.split_first().ok_or(INVALID)?;
            (size, Some(lines))
        } else {
            (part, None)
        };
        let size = usize::try_from(size.as_u64().ok_or(INVALID)?)
            .map_err(|_| SerializationError::LengthOverflow)?;
        let end = old
            .checked_add(size)
            .ok_or(SerializationError::LengthOverflow)?;
        if end > length {
            return Err(EditError::MismatchedLength.into());
        }
        if let Some(lines) = lines {
            let lines = lines
                .iter()
                .map(|line| {
                    line.as_str()
                        .filter(|line| !line.contains('\n'))
                        .ok_or(INVALID)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let insert = lines.join("\n");
            if size == 0 && insert.is_empty() {
                return Err(INVALID);
            }
            changes.push(Change::new(old..end, insert));
        } else if size == 0 {
            return Err(INVALID);
        }
        old = end;
    }
    if old != length {
        return Err(EditError::MismatchedLength.into());
    }
    Ok(ChangeSet::from_ordered_changes(changes))
}
