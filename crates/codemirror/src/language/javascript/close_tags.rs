//! JSX input handling from `@codemirror/lang-javascript` 6.2.5.
//! See `docs/licenses/codemirror-lang-javascript.txt`.

use std::ops::Range;

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorState, RangeEdit, SelectionRange, Transaction,
    TransactionCoordinates,
};

use super::{JavascriptDocument, JavascriptNode, JavascriptOptions};

/// Build a single transaction for direct `>` or `/` input in JSX or TSX.
///
/// Call this outside IME composition when automatic tag closing is enabled.
/// Positions use UTF-8 bytes. If no caret gets a close tag, return `None` so
/// the host can handle the input normally. Completion on malformed input
/// depends on the native parser's recovery.
pub fn auto_close_tags(
    state: &EditorState,
    typed: &str,
    options: JavascriptOptions,
) -> Result<Option<Transaction>, EditError> {
    auto_close_tags_in(state, 0..state.text().len(), typed, options)
}

/// Like [`auto_close_tags`], but parse only the given JSX or TSX byte range.
/// All selections must be inside the range. The transaction uses positions
/// in the full document. Return an error if the range is not valid UTF-8 bounds.
pub fn auto_close_tags_in(
    state: &EditorState,
    scope: Range<usize>,
    typed: &str,
    options: JavascriptOptions,
) -> Result<Option<Transaction>, EditError> {
    if !options.jsx
        || !matches!(typed, ">" | "/")
        || !state.selection().main().range().is_empty()
        || state.read_only()
    {
        return Ok(None);
    }

    if !state.text().is_valid_range(scope.clone()) {
        return Err(EditError::InvalidRange);
    }
    if state.selection().ranges().iter().any(|selection| {
        let range = selection.range();
        range.start < scope.start || range.end > scope.end
    }) {
        return Ok(None);
    }

    let base = state.replace_selection(typed)?;
    let mut text = state.text().clone();
    base.changes().apply_to(&mut text)?;
    let offset = scope.start;
    let end = base.changes().map_pos(scope.end, Assoc::After);
    let source = text
        .slice_owned(offset..end)
        .ok_or(EditError::InvalidRange)?;
    let doc = JavascriptDocument::parse_for_tag_input(source, options);
    let post = EditorState {
        text,
        selection: base
            .selection()
            .expect("replace_selection sets selection")
            .clone(),
        ..EditorState::new("")
    };
    let complete = post.change_by_range(|selection| {
        let head = selection.head();
        let mut edit = RangeEdit {
            changes: ChangeSet::default(),
            selection,
            effects: Vec::new(),
        };
        if let Some((insert, caret)) = close_at(&doc, typed, head - offset) {
            edit.changes = ChangeSet::single(Change::new(head..head, insert))?;
            edit.selection = SelectionRange::new(offset + caret, offset + caret);
        }
        Ok(edit)
    })?;
    if complete.changes().is_empty() {
        return Ok(None);
    }
    Ok(Some(
        base.merge(
            complete
                .with_user_event("input.complete")
                .with_scroll_into_view(),
            state.text().len(),
            TransactionCoordinates::Sequential,
        )?,
    ))
}

fn close_at(doc: &JavascriptDocument, typed: &str, head: usize) -> Option<(String, usize)> {
    let source = doc.text().as_str();
    let input_start = head.checked_sub(1)?;
    if source.get(input_start..head)? != typed {
        return None;
    }

    // Resolve to the left of the input character, not to the left of the caret.
    let mut around = resolve_left(doc.root(), input_start);
    if around.kind().name() == "JSXStartTag" {
        around = around.parent()?;
    }
    let kind = around.kind().name();
    if kind == "JSXAttributeValue" && around.range().end > head {
        return None;
    }
    match (typed, kind) {
        (">", "JSXFragmentTag") => Some(("</>".to_owned(), head)),
        ("/", "JSXStartCloseTag") => {
            let close = around.parent()?;
            let open = close.parent()?.children().next()?;
            if head.checked_sub(2) != Some(close.range().start) {
                return None;
            }
            let name = if open.kind().name() == "JSXFragmentTag" {
                ""
            } else {
                element_name(source, open, head)?
            };
            let insert = format!("{name}>");
            let caret = head + insert.len();
            Some((insert, caret))
        }
        (">", _) => {
            let open = find_open_tag(around)?;
            let suffix = &source[head..];
            if open.kind().name() != "JSXOpenTag"
                || suffix.starts_with('>')
                || suffix.starts_with("/>")
                || suffix.starts_with("</")
            {
                return None;
            }
            let name = element_name(source, open, head)?;
            Some((format!("</{name}>"), head))
        }
        _ => None,
    }
}

fn resolve_left(mut node: JavascriptNode<'_>, position: usize) -> JavascriptNode<'_> {
    while let Some(child) = node.children().find(|child| {
        let range = child.range();
        range.start < position && position <= range.end
    }) {
        node = child;
    }
    node
}

fn find_open_tag(mut node: JavascriptNode<'_>) -> Option<JavascriptNode<'_>> {
    loop {
        match node.kind().name() {
            "JSXOpenTag" | "JSXSelfClosingTag" | "JSXFragmentTag" => return Some(node),
            "JSXEscape" => return None,
            _ => node = node.parent()?,
        }
    }
}

fn element_name<'s>(source: &'s str, tag: JavascriptNode<'_>, head: usize) -> Option<&'s str> {
    let name = tag
        .children()
        .find(|node| {
            matches!(
                node.kind().name(),
                "JSXIdentifier" | "JSXBuiltin" | "JSXNamespacedName" | "JSXMemberExpression"
            )
        })?
        .range();
    source
        .get(name.start..name.end.min(head))
        .filter(|name| !name.is_empty())
}
