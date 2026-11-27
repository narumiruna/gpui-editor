use std::ops::Range as ByteRange;

use serde_json::{Value, json};

use crate::{Change, ChangeSet};

use super::super::{Range, TextEdit};
use super::{
    DocumentWorkspace, Documentation, FeatureError, FeatureRequest, LspError, PositionIndex,
    RequestDocument, byte_range, documentation, mapped_range, position_request, string_field,
    word_at,
};

/// Why the host requests completions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionTrigger {
    Invoked,
    Character(char),
    Incomplete,
}

impl CompletionTrigger {
    /// The pinned automatic trigger rule. Explicit requests use `Invoked`.
    pub fn automatic(character: char, server_capabilities: &Value) -> Option<Self> {
        let triggers = server_capabilities
            .get("completionProvider")
            .and_then(|provider| provider.get("triggerCharacters"))
            .and_then(Value::as_array);
        let mut buffer = [0; 4];
        let text: &str = character.encode_utf8(&mut buffer);
        if triggers.is_some_and(|values| values.iter().any(|value| value.as_str() == Some(text))) {
            Some(Self::Character(character))
        } else if character.is_ascii_alphabetic() || character == '_' {
            Some(Self::Invoked)
        } else {
            None
        }
    }

    fn params(self) -> Value {
        match self {
            Self::Invoked => json!({"triggerKind": 1}),
            Self::Character(character) => {
                json!({"triggerKind": 2, "triggerCharacter": character.to_string()})
            }
            Self::Incomplete => json!({"triggerKind": 3}),
        }
    }
}

/// Prepare a completion request for a synchronized document. Rebase untouched
/// ranges in the reply onto the current draft. A touched filter range makes
/// the reply stale.
pub fn completion_request(
    workspace: &DocumentWorkspace,
    uri: &str,
    byte_offset: usize,
    trigger: CompletionTrigger,
) -> Result<FeatureRequest<CompletionList>, FeatureError> {
    let mut request = position_request(
        workspace,
        uri,
        byte_offset,
        "textDocument/completion",
        "completionProvider",
        decode_completion,
    )?;
    request.params["context"] = trigger.params();
    Ok(request)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InsertTextFormat {
    #[default]
    PlainText,
    Snippet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionEditMode {
    Insert,
    Replace,
}

/// One completion item. No commands or resolve tokens are run or interpreted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
    pub kind: Option<u32>,
    pub detail: Option<String>,
    pub documentation: Option<Documentation>,
    /// Use the list's commit characters when this field is `None`.
    pub commit_characters: Option<Vec<String>>,
    pub insert_text: String,
    pub insert_text_format: InsertTextFormat,
    ranges: EditRanges,
    additional: Vec<ByteEdit>,
    invalid_additional_edits: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EditRanges {
    insert: ByteRange<usize>,
    replace: ByteRange<usize>,
}

impl EditRanges {
    fn single(range: ByteRange<usize>) -> Self {
        Self {
            insert: range.clone(),
            replace: range,
        }
    }

    fn parse(value: &Value, index: &PositionIndex<'_>, cursor: usize) -> Result<Self, LspError> {
        let ranges =
            if let (Some(insert), Some(replace)) = (value.get("insert"), value.get("replace")) {
                let insert = completion_range(index, insert, cursor)?;
                let replace = completion_range(index, replace, cursor)?;
                if insert.start != replace.start || insert.end > replace.end {
                    return Err(LspError::InvalidFormat("completion insert/replace range"));
                }
                Self { insert, replace }
            } else {
                Self::single(completion_range(index, value, cursor)?)
            };
        Ok(ranges)
    }
}

fn completion_range(
    index: &PositionIndex<'_>,
    value: &Value,
    cursor: usize,
) -> Result<ByteRange<usize>, LspError> {
    let range: Range = value.try_into()?;
    let bytes = byte_range(index, range)?;
    if range.start.line != range.end.line || cursor < bytes.start || cursor > bytes.end {
        return Err(LspError::InvalidFormat("completion edit range"));
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ByteEdit {
    range: ByteRange<usize>,
    text: String,
}

/// A completion array or list, with defaults from the server.
#[derive(Debug)]
pub struct CompletionList {
    /// The shared filter range in the draft at the time of response handling.
    /// Use [`Self::plan`] for the selected item's current edit range.
    pub range: ByteRange<usize>,
    pub is_incomplete: bool,
    pub commit_characters: Option<Vec<String>>,
    items: Vec<CompletionItem>,
    document: RequestDocument,
}

/// Edits for the current draft. Apply the batch only to that same draft.
#[derive(Debug, Eq, PartialEq)]
pub struct CompletionEditPlan {
    pub changes: ChangeSet,
    /// Edits with UTF-16 endpoints outside the request snapshot or reversed.
    pub invalid_additional_edits: usize,
    /// Edits skipped because the current draft has touched their ranges.
    pub stale_additional_edits: usize,
}

impl CompletionList {
    pub fn items(&self) -> &[CompletionItem] {
        &self.items
    }

    /// Plan one item's edits. Recheck against edits made after the reply.
    /// A touched primary range makes the plan stale. As in the pinned client,
    /// omit additional edits with unusable positions or touched ranges.
    /// The skipped edit counts tell the host when the batch is incomplete.
    /// Reject overlapping edits; do not execute commands or expand snippets.
    pub fn plan(
        &self,
        workspace: &DocumentWorkspace,
        item: usize,
        mode: CompletionEditMode,
    ) -> Result<CompletionEditPlan, FeatureError> {
        let item = self
            .items
            .get(item)
            .ok_or(LspError::InvalidFormat("completion item index"))?;
        let doc = &self.document;
        let changes = doc.changes(workspace)?;
        let draft = workspace
            .file(&doc.uri)
            .ok_or_else(|| doc.stale(workspace))?
            .draft();
        let index = PositionIndex::new(draft);
        let map = |range| {
            let bytes = mapped_range(&changes, range)?;
            index.position(bytes.start).ok()?;
            index.position(bytes.end).ok()?;
            Some(bytes)
        };
        let range = match mode {
            CompletionEditMode::Insert => &item.ranges.insert,
            CompletionEditMode::Replace => &item.ranges.replace,
        };
        let primary = map(range.clone()).ok_or_else(|| doc.stale(workspace))?;
        if item.insert_text_format == InsertTextFormat::Snippet {
            return Err(FeatureError::Unsupported("completion snippet expansion"));
        }
        let mut edits = vec![ByteEdit {
            range: primary,
            text: item.insert_text.clone(),
        }];
        let mut stale_additional_edits = 0;
        for edit in &item.additional {
            if let Some(range) = map(edit.range.clone()) {
                edits.push(ByteEdit {
                    range,
                    text: edit.text.clone(),
                });
            } else {
                stale_additional_edits += 1;
            }
        }
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        if edits.windows(2).any(|pair| {
            let left = &pair[0].range;
            let right = &pair[1].range;
            left.end > right.start || left.is_empty() && left == right
        }) {
            return Err(LspError::OverlappingEdits.into());
        }
        let changes = ChangeSet::new(
            edits
                .into_iter()
                .map(|edit| Change::new(edit.range, edit.text))
                .collect(),
        )
        .map_err(|_| LspError::InvalidChangeSet)?;
        changes
            .check(draft)
            .map_err(|_| LspError::InvalidChangeSet)?;
        Ok(CompletionEditPlan {
            changes,
            invalid_additional_edits: item.invalid_additional_edits,
            stale_additional_edits,
        })
    }
}

fn decode_completion(
    doc: &RequestDocument,
    workspace: &DocumentWorkspace,
    result: &Value,
) -> Result<CompletionList, FeatureError> {
    let changes = doc.changes(workspace)?;
    let (items, is_incomplete, defaults) = if let Some(items) = result.as_array() {
        (items, false, None)
    } else {
        let items = result
            .get("items")
            .and_then(Value::as_array)
            .ok_or(LspError::InvalidFormat("CompletionList.items"))?;
        let is_incomplete = result
            .get("isIncomplete")
            .and_then(Value::as_bool)
            .ok_or(LspError::InvalidFormat("CompletionList.isIncomplete"))?;
        let defaults = result.get("itemDefaults");
        if defaults.is_some_and(|defaults| !defaults.is_object()) {
            return Err(LspError::InvalidFormat("CompletionList.itemDefaults").into());
        }
        (items, is_incomplete, defaults)
    };
    let index = PositionIndex::new(&doc.text);
    let default_ranges = defaults
        .and_then(|defaults| defaults.get("editRange"))
        .map(|range| EditRanges::parse(range, &index, doc.cursor))
        .transpose()?;
    let default_format =
        format(defaults.and_then(|defaults| defaults.get("insertTextFormat")))?.unwrap_or_default();
    let commit_characters =
        characters(defaults.and_then(|defaults| defaults.get("commitCharacters")))?;
    let fallback = items
        .first()
        .map(|first| {
            if let Some(ranges) = &default_ranges {
                Ok(ranges.clone())
            } else {
                item_ranges(first, &index, doc.cursor).map(|ranges| {
                    ranges.unwrap_or_else(|| EditRanges::single(word_at(&doc.text, doc.cursor)))
                })
            }
        })
        .transpose()?
        .unwrap_or_else(|| EditRanges::single(doc.cursor..doc.cursor));
    let range =
        mapped_range(&changes, fallback.insert.clone()).ok_or_else(|| doc.stale(workspace))?;
    let items = items
        .iter()
        .map(|item| decode_item(item, &index, doc.cursor, &fallback, default_format))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompletionList {
        range,
        is_incomplete,
        commit_characters,
        items,
        document: doc.clone(),
    })
}

fn item_ranges(
    item: &Value,
    index: &PositionIndex<'_>,
    cursor: usize,
) -> Result<Option<EditRanges>, LspError> {
    item.get("textEdit")
        .map(|edit| EditRanges::parse(edit.get("range").unwrap_or(edit), index, cursor))
        .transpose()
}

fn decode_item(
    item: &Value,
    index: &PositionIndex<'_>,
    cursor: usize,
    fallback: &EditRanges,
    default_format: InsertTextFormat,
) -> Result<CompletionItem, LspError> {
    let label = string_field(item, "label", "CompletionItem.label")?;
    let field = |key| {
        item.get(key)
            .map(|value| {
                value
                    .as_str()
                    .ok_or(LspError::InvalidFormat("CompletionItem string"))
            })
            .transpose()
    };
    let edit_text = item
        .get("textEdit")
        .map(|edit| string_field(edit, "newText", "CompletionItem.textEdit.newText"))
        .transpose()?;
    let insert_text = edit_text
        .or(field("textEditText")?)
        .or(field("insertText")?)
        .unwrap_or(label);
    let kind = item
        .get("kind")
        .map(|kind| {
            kind.as_u64()
                .filter(|&number| number <= i32::MAX as u64)
                .map(|number| number as u32)
                .ok_or(LspError::InvalidFormat("CompletionItem.kind"))
        })
        .transpose()?;
    let mut additional = Vec::new();
    let mut invalid_additional_edits = 0;
    if let Some(edits) = item.get("additionalTextEdits") {
        for edit in edits.as_array().ok_or(LspError::InvalidFormat(
            "CompletionItem.additionalTextEdits",
        ))? {
            let edit = TextEdit::try_from(edit)?;
            match byte_range(index, edit.range) {
                Ok(range) => additional.push(ByteEdit {
                    range,
                    text: edit.new_text,
                }),
                Err(LspError::InvalidPosition(_) | LspError::ReversedRange(_)) => {
                    invalid_additional_edits += 1
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(CompletionItem {
        label: label.into(),
        filter_text: field("filterText")?.map(str::to_owned),
        sort_text: field("sortText")?.map(str::to_owned),
        kind,
        detail: field("detail")?.map(str::to_owned),
        documentation: item
            .get("documentation")
            .map(|value| documentation(value, false))
            .transpose()?,
        commit_characters: characters(item.get("commitCharacters"))?,
        insert_text: insert_text.into(),
        insert_text_format: format(item.get("insertTextFormat"))?.unwrap_or(default_format),
        ranges: item_ranges(item, index, cursor)?.unwrap_or_else(|| fallback.clone()),
        additional,
        invalid_additional_edits,
    })
}

fn format(value: Option<&Value>) -> Result<Option<InsertTextFormat>, LspError> {
    value
        .map(|value| match value.as_u64() {
            Some(1) => Ok(InsertTextFormat::PlainText),
            Some(2) => Ok(InsertTextFormat::Snippet),
            _ => Err(LspError::InvalidFormat("completion insertTextFormat")),
        })
        .transpose()
}

fn characters(value: Option<&Value>) -> Result<Option<Vec<String>>, LspError> {
    let invalid = LspError::InvalidFormat("completion commitCharacters");
    value
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| invalid.clone())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid.clone())
                })
                .collect()
        })
        .transpose()
}
