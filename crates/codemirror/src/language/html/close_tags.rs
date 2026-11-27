//! HTML input handling from `@codemirror/lang-html` 6.4.12.
//! See `docs/licenses/codemirror-lang-html.txt`.

use std::ops::Range;

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorState, MapMode, RangeEdit, SelectionRange, Text,
    Transaction, TransactionCoordinates,
};

use super::{HtmlDocument, HtmlNode, HtmlNodeKind::*, HtmlOptions, lexer};

/// Build the typing transaction for `>` or `/` in an HTML document.
///
/// Return `None` to let the host handle ordinary input. Call this for direct
/// typing only, outside IME composition, when automatic tag closing is enabled.
/// A handled edit includes the typed character and all generated closes.
pub fn auto_close_tags(
    state: &EditorState,
    typed: &str,
    options: HtmlOptions,
) -> Result<Option<Transaction>, EditError> {
    auto_close_tags_in(state, 0..state.text().len(), typed, options)
}

/// Like [`auto_close_tags`], but parse only the given HTML byte range.
/// All selection ranges must be inside this range. The transaction uses byte
/// positions in the full document. The host decides which range is HTML.
/// Return an error if the byte range is outside the document or splits a character.
pub fn auto_close_tags_in(
    state: &EditorState,
    scope: Range<usize>,
    typed: &str,
    options: HtmlOptions,
) -> Result<Option<Transaction>, EditError> {
    let main = state.selection().main().range();
    if !matches!(typed, ">" | "/") || !main.is_empty() || state.read_only() {
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
    let end = base.changes().map_pos(scope.end, Assoc::After);
    let after = parse(&text, scope.start..end, options);
    let post = EditorState {
        text,
        selection: base
            .selection()
            .expect("replace_selection sets selection")
            .clone(),
        ..EditorState::new("")
    };
    let complete =
        post.change_by_range(|selection| close_tag(&after, scope.start, typed, selection))?;
    if complete.changes().is_empty() {
        return Ok(None);
    }
    let offset = scope.start;
    let position = main.start - offset;
    let before = parse(state.text(), scope, options);
    let head = post.selection().main().head() - offset;
    let reverse = base.changes().desc(state.text().len())?.inverted();
    if !html_input_is_active(&before, position, Some((&after, head)), |start| {
        reverse
            .map_pos(start + offset, Assoc::After, MapMode::Simple)?
            .checked_sub(offset)
    }) {
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

fn parse(text: &Text, scope: Range<usize>, options: HtmlOptions) -> HtmlDocument {
    HtmlDocument::parse_with_options(text.slice_owned(scope).expect("valid HTML range"), options)
}

fn close_tag(
    doc: &HtmlDocument,
    offset: usize,
    typed: &str,
    selection: SelectionRange,
) -> Result<RangeEdit, EditError> {
    let source = doc.text().as_str();
    let head = selection.head() - offset;
    let mut edit = RangeEdit {
        changes: ChangeSet::default(),
        selection,
        effects: Vec::new(),
    };
    let range = selection.range();
    if range.start == offset
        || source.get(range.start - offset - 1..range.end - offset) != Some(typed)
    {
        return Ok(edit);
    }
    let Some(insert) = tag_close_insert(doc, head, typed, |_| true) else {
        return Ok(edit);
    };
    if typed == "/" {
        let end = selection.head() + insert.len();
        edit.selection = SelectionRange::new(end, end);
    }
    let end = selection.head() + usize::from(source.as_bytes().get(head) == Some(&b'>'));
    edit.changes = ChangeSet::single(Change::new(selection.head()..end, insert))?;
    Ok(edit)
}

/// Inspect the tree after direct typing. The caller checks the input and maps
/// the edit. `name_is_mapped` must reject names that cross a projection gap.
pub(crate) fn tag_close_insert(
    doc: &HtmlDocument,
    head: usize,
    typed: &str,
    name_is_mapped: impl FnOnce(Range<usize>) -> bool,
) -> Option<String> {
    let node = resolve_left(doc, head);
    let parent = node.parent()?;
    let element = match (typed, node.kind()) {
        (">", EndTag) => parent.parent()?,
        ("/", IncompleteCloseTag)
            if head.checked_sub(2) == Some(node.range().start) && !has_close(parent) =>
        {
            parent
        }
        _ => return None,
    };
    let source = doc.text().as_str();
    let name_range = element_name_range(element, head)?;
    let name = source.get(name_range.clone())?;
    // Use the source spelling. The upstream void-name lookup is case-sensitive.
    if name.is_empty()
        || !name_is_mapped(name_range)
        || lexer::is_void(name)
        || (typed == ">" && is_closed(source, element, name))
    {
        return None;
    }
    Some(if typed == ">" {
        format!("</{name}>")
    } else {
        format!("{name}>")
    })
}

fn resolve_left(doc: &HtmlDocument, position: usize) -> HtmlNode<'_> {
    let mut node = doc.root();
    loop {
        let children = &node.data().children;
        let i = children.partition_point(|&id| doc.nodes[id].range.end < position);
        let Some(&id) = children.get(i) else { break };
        if doc.nodes[id].range.start >= position {
            break;
        }
        node = HtmlNode { id, ..node };
    }
    node
}

fn element_name_range(element: HtmlNode<'_>, end: usize) -> Option<Range<usize>> {
    let name = element.children().next()?.child(TagName)?.range();
    Some(name.start..name.end.min(end))
}

fn element_name<'s>(source: &'s str, element: HtmlNode<'_>, end: usize) -> Option<&'s str> {
    source
        .get(element_name_range(element, end)?)
        .filter(|name| !name.is_empty())
}

fn has_close(element: HtmlNode<'_>) -> bool {
    element
        .children()
        .next_back()
        .is_some_and(|node| node.kind() == CloseTag)
}

fn is_closed(source: &str, mut element: HtmlNode<'_>, name: &str) -> bool {
    // For equal nested names, each ancestor must also have a close tag.
    while has_close(element) {
        let Some(parent) = element
            .parent()
            .filter(|&node| element_name(source, node, source.len()) == Some(name))
        else {
            return true;
        };
        element = parent;
    }
    false
}

/// Check the pre-input HTML language. Some incomplete raw tags only have a
/// native script or style mount after typing. Map their start back to `before`.
pub(crate) fn html_input_is_active(
    before: &HtmlDocument,
    position: usize,
    after: Option<(&HtmlDocument, usize)>,
    previous_position: impl FnOnce(usize) -> Option<usize>,
) -> bool {
    !in_mount(before, position)
        && !after
            .and_then(|(doc, head)| mounted_raw_open_start(doc, head))
            .and_then(previous_position)
            .is_some_and(|start| at_missing_raw_end(before, start, position))
}

fn in_mount(doc: &HtmlDocument, position: usize) -> bool {
    // The upstream language check enters bracketed mounts at either boundary.
    doc.mounts().iter().any(|mount| {
        let range = mount.range();
        if !(range.start..=range.end).contains(&position) {
            return false;
        }
        if range.start < position && position < range.end {
            return true;
        }
        let attribute = doc.nodes().find(|node| match node.kind() {
            UnquotedAttributeValue => node.range() == range,
            AttributeValue => {
                let value = node.range();
                value.start + 1 == range.start && value.end == range.end
            }
            _ => false,
        });
        match attribute.map(HtmlNode::kind) {
            // The overlay has no brackets and the upstream query looks left.
            Some(UnquotedAttributeValue) => range.start < position,
            // For a missing quote, the pinned overlay still drops the last UTF-16 unit.
            Some(AttributeValue) => {
                position < range.end
                    && doc.text().as_str()[range].encode_utf16().take(2).count() == 2
            }
            _ => true,
        }
    })
}

fn mounted_raw_open_start(doc: &HtmlDocument, head: usize) -> Option<usize> {
    let token = resolve_left(doc, head);
    if token.kind() != EndTag {
        return None;
    }
    let tag = token.parent().filter(|tag| tag.kind() == OpenTag)?;
    let body = tag.parent()?.children().nth(1)?;
    (matches!(body.kind(), ScriptText | StyleText)
        && body.range().start == head
        && doc.mounts().iter().any(|mount| mount.range().start == head))
    .then(|| tag.range().start)
}

fn at_missing_raw_end(before: &HtmlDocument, start: usize, position: usize) -> bool {
    // Lezer mounts a raw body at the missing `>` of an unfinished script or
    // style tag. Native recovery creates that body only after the tag ends.
    before.nodes().any(|tag| {
        tag.kind() == OpenTag
            && tag.range().start == start
            && tag.children().next_back().is_some_and(|last| {
                let end = tag.range().end;
                last.kind() == Error
                    && last.range().is_empty()
                    && end + lexer::space_len(&before.text().as_str()[end..]) == position
            })
    })
}
