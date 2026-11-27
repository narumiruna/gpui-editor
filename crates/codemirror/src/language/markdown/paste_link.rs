use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use crate::{EditorState, Transaction};

use super::{MarkdownDocument, MarkdownNode};

/// Wrap the main Markdown selection as in CodeMirror's `pasteURLAsLink`.
///
/// Pass the clipboard's plain text and a Markdown parse of the current state.
/// This only builds a text edit. It does not validate or open the link target.
/// The caller controls whether paste is enabled, including in read-only views.
pub fn paste_url_as_link(
    document: &MarkdownDocument,
    state: &EditorState,
    clipboard: &str,
) -> Option<Transaction> {
    let selection = state.selection().main().range();
    if selection.is_empty() {
        return None;
    }
    let prefix = if clipboard.starts_with("www.") {
        "https://"
    } else if ["http://", "https://", "mailto:", "xmpp:"]
        .iter()
        .any(|prefix| clipboard.starts_with(prefix))
    {
        ""
    } else {
        return None;
    };

    let text = state.text();
    if document.text() != text || !text.is_valid_range(selection.clone()) {
        return None;
    }
    let source = text.as_str().as_bytes();
    let Range { start, end } = selection;
    // Upstream has no position between the CR and LF of a logical break.
    if [start, end]
        .into_iter()
        .any(|at| at > 0 && source.get(at - 1..=at) == Some(b"\r\n"))
        || !plain_selection(document.root(), &selection)
    {
        return None;
    }

    let suffix = format!("]({prefix}{clipboard})");
    let changes = state
        .changes([(start..start, "["), (end..end, suffix.as_str())])
        .ok()?;
    Some(
        Transaction::new(changes)
            .with_user_event("input.paste")
            .with_scroll_into_view(),
    )
}

fn plain_selection(node: &MarkdownNode, selection: &Range<usize>) -> bool {
    static NON_PLAIN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i:code|horizontalrule|html|link|comment|processing|escape|entity|image|mark|url)",
        )
        .unwrap()
    });

    // Lezer Tree.iterate includes nodes that only touch a selection endpoint.
    if node.range.end < selection.start || node.range.start > selection.end {
        return true;
    }
    node.range.start <= selection.start
        && node.range.end >= selection.end
        && !NON_PLAIN_TEXT.is_match(node.kind.name())
        && node
            .children()
            .all(|child| plain_selection(child, selection))
}
