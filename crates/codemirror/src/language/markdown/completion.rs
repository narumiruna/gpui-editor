//! Native HTML tag completion from `@codemirror/lang-markdown` 6.5.2.
//! See `docs/licenses/codemirror-lang-markdown.txt`.

use std::sync::{Arc, LazyLock};

use crate::{
    Text,
    autocomplete::session::{CompletionItem, SourceResult},
    language::html::{HtmlDocument, HtmlOptions, html_completion_source},
};

use super::{MarkdownDocument, MarkdownNodeKind::*};

// Markdown uses the pinned HTML schema's explicit options for an empty document.
static TAG_OPTIONS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| {
    let empty = HtmlDocument::parse_with_options(
        Text::new(""),
        HtmlOptions {
            match_closing_tags: false,
            ..HtmlOptions::default()
        },
    );
    html_completion_source(&empty, 0, true).map_or_else(Arc::default, |result| result.options)
});

/// Return the Markdown source's HTML tag options at a UTF-8 byte position.
/// Explicit requests use the same rules as typing. The direct source stops
/// its ancestor check at an HTML overlay; it does not route the request to
/// the embedded language's completion source.
pub fn markdown_completion_source(
    doc: &MarkdownDocument,
    position: usize,
    _explicit: bool,
) -> Option<SourceResult> {
    let from = markdown_completion_start(doc.text(), position)?;
    let mut blocked = false;
    let mut inner = Document;
    for (node, _) in doc.path(position, true) {
        inner = node.kind();
        blocked |= matches!(
            inner,
            CodeBlock | FencedCode | ProcessingInstructionBlock | CommentBlock | Link | Image
        );
    }
    // HTML mounts cover the host node except for its children (quote marks).
    // At a mount, the ancestor scan stops at the inner language's top node.
    if blocked && !matches!(inner, HTMLTag | HTMLBlock | CommentBlock) {
        return None;
    }

    let mut result = SourceResult::new(from, TAG_OPTIONS.clone()).with_valid_for(|text, _, _| {
        text.strip_prefix('<')
            .is_some_and(|tail| tail.chars().all(tag_character))
    });
    result.to = Some(position);
    Some(result)
}

/// Find the start of a possible tag prefix without parsing the document.
pub fn markdown_completion_start(text: &Text, position: usize) -> Option<usize> {
    let before = text.as_str().get(..position)?;
    let mut remaining = 25_usize;
    for (index, ch) in before.char_indices().rev() {
        remaining = remaining.checked_sub(ch.len_utf16())?;
        if ch == '<' {
            return Some(index);
        }
        if !tag_character(ch) {
            break;
        }
    }
    None
}

fn tag_character(ch: char) -> bool {
    // The upstream expression uses ASCII \w and a UTF-16 range that includes
    // surrogate code units, so supplementary characters also match.
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '-' | '.') || ch >= '\u{b7}'
}
