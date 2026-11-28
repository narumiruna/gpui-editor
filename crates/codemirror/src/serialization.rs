//! JSON shapes from `@codemirror/state` 6.7.6.
//!
//! Default codecs use native UTF-8 bytes. Explicit UTF-16 change codecs use
//! document text to convert the wire lengths. Text keeps its original bytes.
//! An insertion uses LF to separate
//! JSON line elements. A CR, including the CR in CRLF, stays in its element.
//! A state stores its document as one string without changing line endings.

use std::{collections::HashSet, fmt, ops::Range, sync::Arc};

use serde_json::{Value, json};

use crate::{
    ALLOW_MULTIPLE_SELECTIONS, ChangeDesc, ChangeSet, ConfigurationError, EditError,
    EditorSelection, EditorState, Extension, Extensions, LINE_SEPARATOR, SelectionRange,
    StateContext, StateField, Text, Transaction, extension::InitialConfiguration,
    state_config::parse_lines,
};

/// Invalid serialized editor data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SerializationError {
    /// The JSON shape or a value is invalid for this type.
    InvalidFormat(&'static str),
    /// A length is too large for this platform.
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
            Self::LengthOverflow => f.write_str("serialized length exceeds usize"),
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
        self.encode_json(length, |range| Ok(range.len()))
    }

    /// Encode with CodeMirror's UTF-16 lengths. `source` must hold the exact
    /// bytes before the edit. A CR byte stays part of its line content.
    pub fn to_utf16_json(&self, source: &Text) -> Result<Value, SerializationError> {
        self.check(source)?;
        self.encode_json(source.len(), |range| utf16_width(source, range))
    }

    fn encode_json(
        &self,
        length: usize,
        width: impl Fn(Range<usize>) -> Result<usize, SerializationError>,
    ) -> Result<Value, SerializationError> {
        let desc = self.desc(length)?;
        let mut changes = self.changes();
        let mut start = 0;
        desc.section_lengths()
            .map(|(old, new)| {
                let end = start + old;
                let old = width(start..end)?;
                start = end;
                let Some(new) = new else {
                    return Ok(Value::from(old));
                };
                let (_, insert) = changes.next().expect("section matches an edit");
                let mut part = vec![Value::from(old)];
                if new > 0 {
                    part.extend(insert.split('\n').map(Value::from));
                }
                Ok(Value::Array(part))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    }

    /// Decode a set for `source` without changing the text.
    ///
    /// Check the original byte length and all section boundaries. Reject
    /// zero-length unchanged sections, but keep explicit empty replacements.
    /// Inserted lines must not contain LF.
    pub fn from_json(value: &Value, source: &Text) -> Result<Self, SerializationError> {
        Self::decode_json(value, source, |start, length| {
            let end = add_length(start, length)?;
            if end > source.len() {
                return Err(EditError::MismatchedLength.into());
            }
            if !source.is_char_boundary(end) {
                return Err(EditError::InvalidRange.into());
            }
            Ok(end)
        })
    }

    /// Decode CodeMirror wire lengths as UTF-16 code units. Reject a boundary
    /// inside a surrogate pair. Input strings must contain only Unicode scalars.
    /// Use the same LF-delimited text as on the wire; do not normalize CR bytes.
    pub fn from_utf16_json(value: &Value, source: &Text) -> Result<Self, SerializationError> {
        Self::decode_json(value, source, |start, length| {
            utf16_end(source, start, length)
        })
    }

    fn decode_json(
        value: &Value,
        source: &Text,
        end_at: impl Fn(usize, usize) -> Result<usize, SerializationError>,
    ) -> Result<Self, SerializationError> {
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
            let end = end_at(old, length)?;
            let length = end - old;
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
    /// Zero-length unchanged sections are invalid.
    pub fn from_json(value: &Value) -> Result<Self, SerializationError> {
        let sections = description_lengths(value)?.collect::<Result<_, _>>()?;
        Self::from_section_lengths(sections).map_err(|_| SerializationError::LengthOverflow)
    }

    /// Encode a description as CodeMirror UTF-16 lengths. The two documents
    /// must match the description lengths and the content of unchanged sections.
    pub fn to_utf16_json(&self, before: &Text, after: &Text) -> Result<Value, SerializationError> {
        if self.length() != before.len() || self.new_length() != after.len() {
            return Err(EditError::MismatchedLength.into());
        }
        let (mut from_a, mut from_b) = (0, 0);
        let mut parts = Vec::new();
        for (old, new) in self.section_lengths() {
            let to_a = from_a + old;
            let to_b = from_b + new.unwrap_or(old);
            let old = utf16_width(before, from_a..to_a)?;
            let new = if new.is_some() {
                Value::from(utf16_width(after, from_b..to_b)?)
            } else {
                check_gap(before, from_a..to_a, after, from_b..to_b)?;
                Value::from(-1)
            };
            parts.extend([Value::from(old), new]);
            (from_a, from_b) = (to_a, to_b);
        }
        Ok(Value::Array(parts))
    }

    /// Decode a CodeMirror description against exact before and after text.
    /// Keep section boundaries, and reject split scalars or unequal gaps.
    pub fn from_utf16_json(
        value: &Value,
        before: &Text,
        after: &Text,
    ) -> Result<Self, SerializationError> {
        let (mut from_a, mut from_b) = (0, 0);
        let mut lengths = Vec::new();
        for section in description_lengths(value)? {
            let (old, new) = section?;
            let to_a = utf16_end(before, from_a, old)?;
            let to_b = utf16_end(after, from_b, new.unwrap_or(old))?;
            if new.is_none() {
                check_gap(before, from_a..to_a, after, from_b..to_b)?;
            }
            lengths.push((to_a - from_a, new.map(|_| to_b - from_b)));
            (from_a, from_b) = (to_a, to_b);
        }
        if from_a != before.len() || from_b != after.len() {
            return Err(EditError::MismatchedLength.into());
        }
        Self::from_section_lengths(lengths).map_err(|_| SerializationError::LengthOverflow)
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

    /// Encode the selection in UTF-16 coordinates against the exact text.
    /// Keep selection direction and omit metadata, as in the default codec.
    pub fn to_utf16_json(&self, text: &Text) -> Result<Value, SerializationError> {
        let (mut byte, mut units) = (0, 0);
        let converted = self.map_wire_positions(|position| {
            units += utf16_width(text, byte..position)?;
            byte = position;
            Ok(units)
        })?;
        Ok(converted.to_json())
    }

    /// Decode ordered, normalized UTF-16 ranges against the exact text. Do
    /// not accept a position inside a surrogate pair or beyond the buffer.
    pub fn from_utf16_json(value: &Value, text: &Text) -> Result<Self, SerializationError> {
        let (mut byte, mut units) = (0, 0);
        Self::from_json(value)?.map_wire_positions(|position| {
            byte = utf16_end(text, byte, position - units)?;
            units = position;
            Ok(byte)
        })
    }

    fn map_wire_positions(
        &self,
        mut map: impl FnMut(usize) -> Result<usize, SerializationError>,
    ) -> Result<Self, SerializationError> {
        let mut ranges = Vec::with_capacity(self.ranges().len());
        for range in self.ranges() {
            let span = range.range();
            let from = map(span.start)?;
            let to = map(span.end)?;
            ranges.push(if range.anchor() <= range.head() {
                SelectionRange::new(from, to)
            } else {
                SelectionRange::new(to, from)
            });
        }
        Ok(Self::new(ranges, self.main_index())?)
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

    /// Encode a state as CodeMirror JSON with logical UTF-16 selection offsets.
    /// Use the state's output line separator for `doc`. Count an LF or CRLF
    /// line break as one unit in selection positions, independent of its
    /// serialized width. Reject a selection endpoint inside a CRLF pair.
    pub fn to_utf16_json(&self) -> Result<Value, SerializationError> {
        let (mut byte, mut units) = (0, 0);
        let text = self.text();
        let selection = self.selection().map_wire_positions(|position| {
            if position > 0
                && text.as_str().as_bytes().get(position - 1..=position) == Some(b"\r\n")
            {
                return Err(EditError::InvalidRange.into());
            }
            let span = text.slice(byte..position).ok_or(EditError::InvalidRange)?;
            units += span.encode_utf16().count() - span.match_indices("\r\n").count();
            byte = position;
            Ok(units)
        })?;
        let doc = self.slice_doc(0..text.len()).expect("whole document range");
        Ok(json!({"doc": doc, "selection": selection.to_json()}))
    }

    /// Restore CodeMirror JSON with the default input line and selection rules.
    /// Use [`Self::from_utf16_json_with_fields`] to configure those rules.
    pub fn from_utf16_json(value: &Value) -> Result<Self, SerializationError> {
        Self::from_utf16_json_with_fields(value, Extension::default(), &[])
    }

    /// Also encode configured fields with the supplied names. Omit disabled fields.
    pub fn to_json_with_fields(&self, fields: &[JsonField]) -> Result<Value, SerializationError> {
        check_field_names(fields)?;
        self.encode_fields(self.to_json(), fields)
    }

    /// Encode UTF-16 state JSON and the configured fields. Field codecs still
    /// receive the native state and own their data representation.
    pub fn to_utf16_json_with_fields(
        &self,
        fields: &[JsonField],
    ) -> Result<Value, SerializationError> {
        check_field_names(fields)?;
        self.encode_fields(self.to_utf16_json()?, fields)
    }

    fn encode_fields(
        &self,
        mut result: Value,
        fields: &[JsonField],
    ) -> Result<Value, SerializationError> {
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
        let extension = restore_fields(value, extension, fields);
        state.extensions = Extensions::create(extension, &state.text, &state.selection)
            .map_err(EditError::from)?;
        Ok(state)
    }

    /// Restore UTF-16 JSON with optional fields and extensions. Parse the
    /// configured input separator before decoding selection coordinates.
    /// With no selection policy, retain only the main range as CodeMirror
    /// does. Field initializers receive the final native text and selection.
    /// Reject an LF that the explicit separator would leave inside a line.
    pub fn from_utf16_json_with_fields(
        value: &Value,
        extension: Extension,
        fields: &[JsonField],
    ) -> Result<Self, SerializationError> {
        check_field_names(fields)?;
        let invalid = SerializationError::InvalidFormat("editor state");
        let source = value.get("doc").and_then(Value::as_str).ok_or(invalid)?;
        let selection = EditorSelection::from_json(value.get("selection").ok_or(invalid)?)?;
        let extension = Extension::group([
            ALLOW_MULTIPLE_SELECTIONS.of(false),
            restore_fields(value, extension, fields),
        ]);
        let configuration = InitialConfiguration::new(extension).map_err(EditError::from)?;
        let separator = configuration
            .static_facet(&LINE_SEPARATOR)
            .map_err(EditError::from)?;
        let separator = separator.as_deref().unwrap_or("");
        if !separator.is_empty() && source.split(separator).any(|line| line.contains('\n')) {
            return Err(invalid);
        }
        let text = Text::new(parse_lines(source, separator));
        let (mut byte, mut units) = (0, 0);
        let selection = selection.map_wire_positions(|position| {
            byte = utf16_end_with_lines(&text, byte, position - units, true)?;
            units = position;
            Ok(byte)
        })?;
        Ok(Self::finish_create(text, Some(selection), configuration)?)
    }
}

fn restore_fields(value: &Value, extension: Extension, fields: &[JsonField]) -> Extension {
    let restore = fields.iter().filter_map(|field| {
        value
            .get(&field.name)
            .map(|data| (field.decode)(data.clone()))
    });
    Extension::group(restore.chain(std::iter::once(extension)))
}

type SectionLength = (usize, Option<usize>);

fn description_lengths(
    value: &Value,
) -> Result<impl Iterator<Item = Result<SectionLength, SerializationError>> + '_, SerializationError>
{
    let invalid = SerializationError::InvalidFormat("change description");
    let parts = value.as_array().ok_or(invalid)?;
    let (pairs, []) = parts.as_chunks::<2>() else {
        return Err(invalid);
    };
    Ok(pairs.iter().map(move |[old, new]| {
        let old = byte_count(old, invalid)?;
        let new = if new.as_i64() == Some(-1) {
            None
        } else {
            Some(byte_count(new, invalid)?)
        };
        if old == 0 && new.is_none() {
            return Err(invalid);
        }
        Ok((old, new))
    }))
}

fn utf16_width(text: &Text, range: Range<usize>) -> Result<usize, SerializationError> {
    Ok(text
        .slice(range)
        .ok_or(EditError::InvalidRange)?
        .encode_utf16()
        .count())
}

fn utf16_end(text: &Text, start: usize, units: usize) -> Result<usize, SerializationError> {
    utf16_end_with_lines(text, start, units, false)
}

fn utf16_end_with_lines(
    text: &Text,
    start: usize,
    mut units: usize,
    logical_lines: bool,
) -> Result<usize, SerializationError> {
    let mut chars = text.as_str()[start..].char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if units == 0 {
            return Ok(start + offset);
        }
        if logical_lines && ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            chars.next();
        }
        units = units
            .checked_sub(ch.len_utf16())
            .ok_or(EditError::InvalidRange)?;
    }
    if units == 0 {
        Ok(text.len())
    } else {
        Err(EditError::MismatchedLength.into())
    }
}

fn check_gap(
    before: &Text,
    a: Range<usize>,
    after: &Text,
    b: Range<usize>,
) -> Result<(), SerializationError> {
    if before.slice(a).ok_or(EditError::InvalidRange)?
        != after.slice(b).ok_or(EditError::InvalidRange)?
    {
        return Err(SerializationError::InvalidFormat("change description"));
    }
    Ok(())
}

fn byte_count(value: &Value, invalid: SerializationError) -> Result<usize, SerializationError> {
    usize::try_from(value.as_u64().ok_or(invalid)?).map_err(|_| SerializationError::LengthOverflow)
}

fn add_length(left: usize, right: usize) -> Result<usize, SerializationError> {
    left.checked_add(right)
        .ok_or(SerializationError::LengthOverflow)
}
