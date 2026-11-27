use gpui_editor_codemirror::{
    Assoc, Text,
    lsp::{
        DocumentWorkspace, PositionIndex, Range as LspRange,
        features::{CompletionEditMode, CompletionList},
    },
};

use super::{Editor, Range, SourceLspCompletionItem, Stamp, Value};
use crate::{
    document::{BlockPreview, Markdown, Section, SectionKind},
    editor::source_lsp::display_text,
};

const MAX_ITEMS: usize = 32;
const MAX_LABEL_BYTES: usize = 256;
const MAX_ADDITIONAL_EDITS: usize = 16;
const MAX_INSERT_BYTES: usize = 32 * 1024;

/// Remove metadata rather than passing it to the core decoder. Only literal
/// labels and text edits can be used by this host.
pub(super) fn retain_plain_payload(result: &mut Value) -> bool {
    let items = if let Some(list) = result.as_object_mut() {
        list.retain(|key, _| matches!(key.as_str(), "items" | "isIncomplete" | "itemDefaults"));
        if let Some(defaults) = list.get_mut("itemDefaults") {
            let Some(defaults) = defaults.as_object_mut() else {
                return false;
            };
            defaults.retain(|key, _| matches!(key.as_str(), "editRange" | "insertTextFormat"));
        }
        list.get_mut("items").and_then(Value::as_array_mut)
    } else {
        result.as_array_mut()
    };
    let Some(items) = items.filter(|items| (1..=MAX_ITEMS).contains(&items.len())) else {
        return false;
    };
    let mut remaining = MAX_INSERT_BYTES;
    let mut charge = |value: &Value| {
        let Some(left) = value
            .as_str()
            .and_then(|text| remaining.checked_sub(text.len()))
        else {
            return false;
        };
        remaining = left;
        true
    };
    for item in items {
        let Some(item) = item.as_object_mut() else {
            return false;
        };
        item.retain(|key, _| {
            matches!(
                key.as_str(),
                "label"
                    | "textEdit"
                    | "textEditText"
                    | "insertText"
                    | "insertTextFormat"
                    | "additionalTextEdits"
            )
        });
        let Some(label) = item.get("label").and_then(Value::as_str) else {
            return false;
        };
        if label.len() > MAX_LABEL_BYTES || display_text(label).as_deref() != Some(label) {
            return false;
        }
        for key in ["textEditText", "insertText"] {
            if item.get(key).is_some_and(|value| !charge(value)) {
                return false;
            }
        }
        if let Some(edit) = item.get("textEdit") {
            if !edit.get("newText").is_some_and(&mut charge) {
                return false;
            }
        } else if item.get("textEditText").is_none()
            && item.get("insertText").is_none()
            && !charge(&item["label"])
        {
            return false;
        }
        if let Some(edits) = item.get("additionalTextEdits") {
            let Some(edits) = edits
                .as_array()
                .filter(|edits| edits.len() <= MAX_ADDITIONAL_EDITS)
            else {
                return false;
            };
            if !edits
                .iter()
                .all(|edit| edit.get("newText").is_some_and(&mut charge))
            {
                return false;
            }
        }
    }
    true
}

pub(super) fn checked_items(
    workspace: &DocumentWorkspace,
    text: &Text,
    stamp: &Stamp,
    result: &Value,
    list: &CompletionList,
) -> Vec<SourceLspCompletionItem> {
    let Some(items) = result
        .as_array()
        .or_else(|| result.get("items")?.as_array())
    else {
        return Vec::new();
    };
    let positions = PositionIndex::new(text);
    let fallback = result
        .get("itemDefaults")
        .and_then(|defaults| defaults.get("editRange"))
        .or_else(|| items.first()?.get("textEdit").map(edit_range));
    let fallback = fallback.map_or_else(
        || contains(&stamp.body, &list.range).then(|| list.range.clone()),
        |range| primary_range(&positions, stamp, range),
    );
    items
        .iter()
        .zip(list.items())
        .enumerate()
        .filter_map(|(index, (raw, item))| {
            let primary = match raw.get("textEdit") {
                Some(edit) => primary_range(&positions, stamp, edit_range(edit))?,
                None => fallback.clone()?,
            };
            // The core can drop empty no-ops and skip invalid additional edits.
            // Check the wire ranges as well; a side edit must not escape the body.
            if let Some(edits) = raw.get("additionalTextEdits") {
                for edit in edits.as_array()? {
                    scoped_range(&positions, &stamp.body, edit.get("range")?)?;
                }
            }
            if text.as_str().get(primary.clone())? == item.insert_text {
                return None;
            }
            let plan = list
                .plan(workspace, index, CompletionEditMode::Insert)
                .ok()?;
            if plan.invalid_additional_edits != 0
                || plan.stale_additional_edits != 0
                || !plan
                    .changes
                    .changes()
                    .all(|(range, _)| contains(&stamp.body, range))
            {
                return None;
            }
            let caret = plan.changes.map_pos(primary.end, Assoc::Before)
                + if primary.is_empty() {
                    item.insert_text.len()
                } else {
                    0
                };
            Some(SourceLspCompletionItem {
                label: item.label.clone(),
                changes: plan.changes,
                caret,
            })
        })
        .collect()
}

fn edit_range(edit: &Value) -> &Value {
    edit.get("range").unwrap_or(edit)
}

fn primary_range(
    positions: &PositionIndex<'_>,
    stamp: &Stamp,
    range: &Value,
) -> Option<Range<usize>> {
    let insert = match (range.get("insert"), range.get("replace")) {
        (Some(insert), Some(replace)) => {
            scoped_range(positions, &stamp.body, replace)?;
            insert
        }
        (None, None) => range,
        _ => return None,
    };
    let insert = scoped_range(positions, &stamp.body, insert)?;
    (insert.start..=insert.end)
        .contains(&stamp.caret)
        .then_some(insert)
}

fn scoped_range(
    positions: &PositionIndex<'_>,
    body: &Range<usize>,
    wire: &Value,
) -> Option<Range<usize>> {
    let range = LspRange::try_from(wire).ok()?;
    let bytes = positions.offset(range.start).ok()?..positions.offset(range.end).ok()?;
    contains(body, &bytes).then_some(bytes)
}

fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.start <= inner.end && inner.end <= outer.end
}

/// Check the chosen plan against protected Markdown and the active fence.
pub(super) fn checked_fence(
    editor: &Editor,
    stamp: &Stamp,
    item: &SourceLspCompletionItem,
) -> Option<Section> {
    let original = editor.view.state().text();
    let before = editor.markdown().sections();
    let is_fence = |section: &Section| section.source == stamp.fence && is_code(section);
    if !before.iter().any(is_fence)
        || before.iter().any(|section| {
            section.kind == SectionKind::ReadOnly
                && !is_fence(section)
                && item.changes.changes().any(|(range, _)| {
                    range.start < section.source.end && section.source.start < range.end
                })
        })
    {
        return None;
    }
    let mut draft = original.clone();
    item.changes.apply_to(&mut draft).ok()?;
    if draft == *original || !draft.as_str().is_char_boundary(item.caret) {
        return None;
    }
    let body_end = item.changes.map_pos(stamp.body.end, Assoc::After);
    // Do not join an inserted CR to the protected LF before the closing fence.
    if original.as_str()[stamp.body.end..].starts_with('\n')
        && draft.as_str()[..body_end].ends_with('\r')
        && !original.as_str()[..stamp.body.end].ends_with('\r')
    {
        return None;
    }
    let after = Markdown::new(draft.as_str()).sections();
    if before.len() != after.len()
        || !before.iter().zip(&after).all(|(old, new)| {
            let fence = is_fence(old);
            let end = if fence { Assoc::After } else { Assoc::Before };
            old.kind == new.kind
                && (!fence || is_code(new))
                && new.source.start == item.changes.map_pos(old.source.start, Assoc::After)
                && new.source.end == item.changes.map_pos(old.source.end, end)
        })
    {
        return None;
    }
    let mut active = editor.active.clone()?;
    active.content.end = item.changes.map_pos(active.content.end, Assoc::After);
    active.source.end = item.changes.map_pos(active.source.end, Assoc::After);
    Some(active)
}

fn is_code(section: &Section) -> bool {
    section.kind == SectionKind::DraftCode
        || matches!(section.preview, Some(BlockPreview::Code { .. }))
}
