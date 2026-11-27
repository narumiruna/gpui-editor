//! XML input handling from `@codemirror/lang-xml` 6.1.0.
//! See `docs/licenses/codemirror-lang-xml.txt`.

use std::ops::Range;

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorState, RangeEdit, SelectionRange, Transaction,
    TransactionCoordinates,
};

use super::{Side, XmlDocument, XmlNode, XmlNodeKind::*};

/// Build the typing transaction for `>` or `/` in an XML document.
///
/// Call this for direct typing in XML, outside IME composition, when automatic
/// tag closing is enabled. `None` lets the host handle ordinary input. A
/// handled edit includes both the typed character and the automatic closes.
/// Decisions in malformed XML follow the recovered [`XmlDocument`] syntax.
pub fn auto_close_tags(state: &EditorState, typed: &str) -> Result<Option<Transaction>, EditError> {
    auto_close_tags_in(state, 0..state.text().len(), typed)
}

/// Like [`auto_close_tags`], but parse only the given XML byte range.
/// All selection ranges must be inside this range. The transaction uses byte
/// positions in the full document. The host decides which range is XML.
/// Return an error if the range is invalid or splits a UTF-8 character.
pub fn auto_close_tags_in(
    state: &EditorState,
    scope: Range<usize>,
    typed: &str,
) -> Result<Option<Transaction>, EditError> {
    if !matches!(typed, ">" | "/")
        || state.read_only()
        || !state.selection().main().range().is_empty()
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
    let mut post = EditorState::new("");
    post.text = state.text().clone();
    base.changes().apply_to(&mut post.text)?;
    post.selection = base
        .selection()
        .expect("replace_selection sets selection")
        .clone();
    let end = base.changes().map_pos(scope.end, Assoc::After);
    let doc = XmlDocument::parse(
        post.text()
            .slice_owned(scope.start..end)
            .expect("mapped XML scope is valid"),
    );
    let complete =
        post.change_by_range(|selection| close_tag(&doc, scope.start, typed, selection))?;
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

fn close_tag(
    doc: &XmlDocument,
    offset: usize,
    typed: &str,
    selection: SelectionRange,
) -> Result<RangeEdit, EditError> {
    let mut edit = RangeEdit {
        changes: ChangeSet::default(),
        selection,
        effects: Vec::new(),
    };
    let source = doc.text().as_str();
    let head = selection.head() - offset;
    if head
        .checked_sub(1)
        .and_then(|start| source.get(start..head))
        != Some(typed)
    {
        return Ok(edit);
    }

    let node = doc.path(head, Side::Left).last().expect("XML has a root");
    let candidate = match (typed, node.kind()) {
        // The upstream rule also checks EndTag in a mismatched close tag.
        (">", EndTag) => node.parent().and_then(XmlNode::parent),
        ("/", StartCloseTag) if head.checked_sub(2) == Some(node.range().start) => node.parent(),
        _ => None,
    };
    let Some(candidate) = candidate else {
        return Ok(edit);
    };
    if candidate
        .children()
        .next_back()
        .is_some_and(|node| node.kind() == CloseTag)
    {
        return Ok(edit);
    }
    let Some(name) = element_name(source, candidate, head) else {
        return Ok(edit);
    };
    let start = selection.head();
    let end = start + usize::from(source.as_bytes().get(head) == Some(&b'>'));
    let insert = if typed == "/" {
        let insert = format!("{name}>");
        let end = start + insert.len();
        edit.selection = SelectionRange::new(end, end);
        insert
    } else {
        format!("</{name}>")
    };
    edit.changes = ChangeSet::single(Change::new(start..end, insert))?;
    Ok(edit)
}

fn element_name<'s>(source: &'s str, candidate: XmlNode<'_>, end: usize) -> Option<&'s str> {
    let name = candidate
        .children()
        .next()?
        .children()
        .find(|node| node.kind() == TagName)?
        .range();
    source
        .get(name.start..name.end.min(end))
        .filter(|name| !name.is_empty())
}
