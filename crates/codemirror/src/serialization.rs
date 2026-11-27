//! JSON shapes from `@codemirror/state` 6.7.6.
//!
//! Lengths and positions are native UTF-8 bytes, not JavaScript UTF-16 code
//! units. Text keeps its original bytes. An insertion uses LF to separate
//! JSON line elements. A CR, including the CR in CRLF, stays in its element.
//! A state stores its document as one string without changing line endings.

use std::{collections::HashSet, fmt, sync::Arc};

use serde_json::{Value, json};

use crate::{
    ChangeDesc, ChangeSet, ConfigurationError, EditError, EditorSelection, EditorState, Extension,
    Extensions, SelectionRange, StateContext, StateField, Text, Transaction,
};

/// Invalid serialized editor data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SerializationError {
    /// The JSON shape or a value is invalid for this type.
    InvalidFormat(&'static str),
    /// A byte length is too large for this platform.
    LengthOverflow,
    /// The edit or selection is invalid for the document.
    Edit(EditError),
}

impl From<EditError> for SerializationError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

impl fmt::Display for SerializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(kind) => write!(f, "invalid JSON for {kind}"),
            Self::LengthOverflow => f.write_str("serialized byte length exceeds usize"),
            Self::Edit(error) => write!(f, "invalid serialized edit data: {error:?}"),
        }
    }
}

impl std::error::Error for SerializationError {}

impl ChangeSet {
    /// Encode a set for an original document of `length` bytes.
    ///
    /// A number represents unchanged bytes. A replacement is an array with
    /// the old byte length followed by the inserted lines. A deletion has
    /// only the old length. UTF-8 boundaries require the original text and
    /// are checked on decode.
    pub fn to_json(&self, length: usize) -> Result<Value, SerializationError> {
        let desc = self.desc(length)?;
        let mut changes = self.changes();
        let parts = desc.section_lengths().map(|(old, new)| {
            let Some(new) = new else {
                return Value::from(old);
            };
            let (_, insert) = changes.next().expect("section matches an edit");
            let mut part = vec![Value::from(old)];
            if new > 0 {
                part.extend(insert.split('\n').map(Value::from));
            }
            Value::Array(part)
        });
        Ok(Value::Array(parts.collect()))
    }

    /// Decode a set for `source` without changing the text.
    ///
    /// Check the original byte length and all section boundaries. Empty
    /// sections are invalid. Inserted lines must not contain LF.
    /// The set retains the original length and section boundaries.
    pub fn from_json(value: &Value, source: &Text) -> Result<Self, SerializationError> {
        let invalid = SerializationError::InvalidFormat("change set");
        let parts = value.as_array().ok_or(invalid)?;
        let mut sections = Vec::with_capacity(parts.len());
        let mut old = 0;
        let mut new = 0;
        for part in parts {
            let (length, lines) = if let Some(replacement) = part.as_array() {
                let (length, lines) = replacement.split_first().ok_or(invalid)?;
                (byte_count(length, invalid)?, Some(lines))
            } else {
                (byte_count(part, invalid)?, None)
            };
            let end = add_length(old, length)?;
            if end > source.len() {
                return Err(EditError::MismatchedLength.into());
            }
            if !source.is_char_boundary(end) {
                return Err(EditError::InvalidRange.into());
            }
            if let Some(lines) = lines {
                let mut insert = String::new();
                for (index, line) in lines.iter().enumerate() {
                    let line = line
                        .as_str()
                        .filter(|line| !line.contains('\n'))
                        .ok_or(invalid)?;
                    if index > 0 {
                        insert.push('\n');
                    }
                    insert.push_str(line);
                }
                if length == 0 && insert.is_empty() {
                    return Err(invalid);
                }
                new = add_length(new, insert.len())?;
                sections.push((length, Some(insert)));
            } else {
                if length == 0 {
                    return Err(invalid);
                }
                new = add_length(new, length)?;
                sections.push((length, None));
            }
            old = end;
        }
        if old != source.len() {
            return Err(EditError::MismatchedLength.into());
        }
        Ok(Self::from_wire_sections(sections)?)
    }
}

impl ChangeDesc {
    /// Encode alternating old and new byte lengths. A new length of `-1`
    /// marks an unchanged section.
    pub fn to_json(&self) -> Value {
        let parts = self.section_lengths().flat_map(|(old, new)| {
            [
                Value::from(old),
                new.map_or_else(|| Value::from(-1), Value::from),
            ]
        });
        Value::Array(parts.collect())
    }

    /// Decode a description and check both length sums for overflow.
    ///
    /// A description contains no text. It cannot check UTF-8 boundaries.
    /// Empty sections are invalid.
    pub fn from_json(value: &Value) -> Result<Self, SerializationError> {
        let invalid = SerializationError::InvalidFormat("change description");
        let parts = value.as_array().ok_or(invalid)?;
        let (pairs, []) = parts.as_chunks::<2>() else {
            return Err(invalid);
        };
        let sections = pairs
            .iter()
            .map(|[old, new]| {
                let old = byte_count(old, invalid)?;
                let new = if new.as_i64() == Some(-1) {
                    None
                } else {
                    Some(byte_count(new, invalid)?)
                };
                if old == 0 && new.unwrap_or(0) == 0 {
                    return Err(invalid);
                }
                Ok((old, new))
            })
            .collect::<Result<_, SerializationError>>()?;
        Self::from_section_lengths(sections).map_err(|_| SerializationError::LengthOverflow)
    }
}

impl SelectionRange {
    /// Encode the anchor and head in UTF-8 bytes. Omit selection metadata.
    pub fn to_json(self) -> Value {
        json!({"anchor": self.anchor(), "head": self.head()})
    }

    /// Decode the anchor and head. Restore their default association and
    /// ignore extra properties. This does not check any document boundaries.
    pub fn from_json(value: &Value) -> Result<Self, SerializationError> {
        Self::decode_json(value, SerializationError::InvalidFormat("selection range"))
    }

    fn decode_json(value: &Value, invalid: SerializationError) -> Result<Self, SerializationError> {
        let fields = value.as_object().ok_or(invalid)?;
        let anchor = byte_count(fields.get("anchor").ok_or(invalid)?, invalid)?;
        let head = byte_count(fields.get("head").ok_or(invalid)?, invalid)?;
        Ok(Self::new(anchor, head))
    }
}

impl EditorSelection {
    /// Encode the ranges, in order, and the index of the main range.
    /// Omit all range metadata.
    pub fn to_json(&self) -> Value {
        let ranges: Vec<_> = self
            .ranges()
            .iter()
            .copied()
            .map(SelectionRange::to_json)
            .collect();
        json!({"ranges": ranges, "main": self.main_index()})
    }

    /// Decode ordered, normalized ranges without an associated document.
    ///
    /// Reject overlapping or unordered ranges rather than changing them.
    /// [`EditorState::from_json`] also checks positions against the text.
    pub fn from_json(value: &Value) -> Result<Self, SerializationError> {
        let invalid = SerializationError::InvalidFormat("editor selection");
        let fields = value.as_object().ok_or(invalid)?;
        let ranges = fields
            .get("ranges")
            .and_then(Value::as_array)
            .ok_or(invalid)?;
        let main = byte_count(fields.get("main").ok_or(invalid)?, invalid)?;
        if main >= ranges.len() {
            return Err(invalid);
        }
        let mut selection = Vec::with_capacity(ranges.len());
        let mut previous_end = None;
        for value in ranges {
            let range = SelectionRange::decode_json(value, invalid)?;
            let span = range.range();
            if previous_end
                .is_some_and(|end| span.start < end || (span.is_empty() && span.start == end))
            {
                return Err(invalid);
            }
            previous_end = Some(span.end);
            selection.push(range);
        }
        Ok(Self::new(selection, main)?)
    }
}

type FieldEncoder =
    dyn Fn(&StateContext<'_>) -> Result<Option<Value>, ConfigurationError> + Send + Sync;
type FieldDecoder = dyn Fn(Value) -> Extension + Send + Sync;

/// An explicit property name and JSON codec for a typed editor field.
pub struct JsonField {
    name: String,
    encode: Box<FieldEncoder>,
    decode: Box<FieldDecoder>,
}

impl<T: Send + Sync + 'static> StateField<T> {
    pub fn json(
        &self,
        name: impl Into<String>,
        encode: impl Fn(&T, &StateContext<'_>) -> Result<Value, ConfigurationError>
        + Send
        + Sync
        + 'static,
        decode: impl Fn(&Value, &StateContext<'_>) -> Result<T, ConfigurationError>
        + Send
        + Sync
        + 'static,
    ) -> JsonField {
        let field = self.clone();
        let restore = self.clone();
        let decode = Arc::new(decode);
        JsonField {
            name: name.into(),
            encode: Box::new(move |state| {
                state
                    .field(&field)?
                    .map(|value| encode(&value, state))
                    .transpose()
            }),
            decode: Box::new(move |value| {
                let decode = decode.clone();
                restore.init(move |state| decode(&value, state))
            }),
        }
    }
}

fn check_field_names(fields: &[JsonField]) -> Result<(), SerializationError> {
    let mut names = HashSet::from(["doc", "selection"]);
    if fields
        .iter()
        .any(|field| !names.insert(field.name.as_str()))
    {
        return Err(SerializationError::InvalidFormat("state field names"));
    }
    Ok(())
}

impl EditorState {
    /// Encode the exact document string and its selection. Extensions and
    /// state fields are not serialized.
    pub fn to_json(&self) -> Value {
        json!({"doc": self.text().as_str(), "selection": self.selection().to_json()})
    }

    /// Restore the document and selection without extensions. Extra object
    /// properties are ignored. Check all selection positions against the
    /// document's byte length and UTF-8 boundaries.
    pub fn from_json(value: &Value) -> Result<Self, SerializationError> {
        let invalid = SerializationError::InvalidFormat("editor state");
        let fields = value.as_object().ok_or(invalid)?;
        let source = fields.get("doc").and_then(Value::as_str).ok_or(invalid)?;
        let selection = EditorSelection::from_json(fields.get("selection").ok_or(invalid)?)?;
        let mut state = Self::new(source);
        state.update(Transaction::default().with_selection(selection))?;
        Ok(state)
    }

    /// Also encode configured fields with the supplied names. Omit disabled fields.
    pub fn to_json_with_fields(&self, fields: &[JsonField]) -> Result<Value, SerializationError> {
        check_field_names(fields)?;
        let mut result = self.to_json();
        for field in fields {
            if let Some(value) = (field.encode)(&self.context()).map_err(EditError::from)? {
                result[&field.name] = value;
            }
        }
        Ok(result)
    }

    /// Restore named fields as initializers, in addition to the given extensions.
    /// A field with no serialized value keeps its ordinary configured initializer.
    pub fn from_json_with_fields(
        value: &Value,
        extension: Extension,
        fields: &[JsonField],
    ) -> Result<Self, SerializationError> {
        check_field_names(fields)?;
        let mut state = Self::from_json(value)?;
        let restore = fields.iter().filter_map(|field| {
            value
                .get(&field.name)
                .map(|data| (field.decode)(data.clone()))
        });
        let extension = Extension::group(restore.chain(std::iter::once(extension)));
        state.extensions = Extensions::create(extension, &state.text, &state.selection)
            .map_err(EditError::from)?;
        Ok(state)
    }
}

fn byte_count(value: &Value, invalid: SerializationError) -> Result<usize, SerializationError> {
    usize::try_from(value.as_u64().ok_or(invalid)?).map_err(|_| SerializationError::LengthOverflow)
}

fn add_length(left: usize, right: usize) -> Result<usize, SerializationError> {
    left.checked_add(right)
        .ok_or(SerializationError::LengthOverflow)
}
