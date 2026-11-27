use std::{ops::Range, sync::LazyLock};

use super::{
    super::HighlightTag,
    HighlightSpan, HtmlDocument,
    HtmlNodeKind::{self, *},
};

pub(super) fn collect(doc: &HtmlDocument) -> Vec<HighlightSpan<'static>> {
    let mut spans = Vec::new();
    for node in doc.nodes() {
        let parent = node.parent().map(|node| node.kind());
        if matches!(parent, Some(AttributeValue | Comment)) {
            continue;
        }
        if node.kind() == AttributeValue {
            let mut start = node.range().start;
            for child in node.children() {
                let range = child.range();
                push(&mut spans, start..range.start, AttributeValue, None);
                start = range.end;
                push(&mut spans, range, child.kind(), None);
            }
            push(&mut spans, start..node.range().end, AttributeValue, None);
        } else {
            push(&mut spans, node.range(), node.kind(), parent);
        }
    }
    let mut mounts = doc.mounts().iter().peekable();
    let mut outer = Vec::with_capacity(spans.len());
    for span in spans {
        let mut start = span.range.start;
        while let Some(mount) = mounts.peek() {
            let range = mount.range();
            if range.start >= span.range.end {
                break;
            }
            if start < range.start {
                outer.push(HighlightSpan {
                    range: start..range.start,
                    tags: span.tags,
                });
            }
            start = start.max(range.end);
            if range.end > span.range.end {
                break;
            }
            mounts.next();
        }
        if start < span.range.end {
            outer.push(HighlightSpan {
                range: start..span.range.end,
                tags: span.tags,
            });
        }
    }
    outer
}

fn push(
    spans: &mut Vec<HighlightSpan<'static>>,
    range: Range<usize>,
    kind: HtmlNodeKind,
    parent: Option<HtmlNodeKind>,
) {
    if range.is_empty() {
        return;
    }
    static TAGS: LazyLock<[HighlightTag; 12]> = LazyLock::new(|| {
        [
            "content",
            "angleBracket",
            "tagName",
            "attributeName",
            "attributeValue",
            "definitionOperator",
            "character",
            "blockComment",
            "processingInstruction",
            "documentMeta",
            "tagName",
            "invalid",
        ]
        .map(HighlightTag::new)
    });
    let index = match kind {
        Text | IncompleteTag | IncompleteCloseTag => 0,
        StartTag | StartCloseTag | EndTag | SelfClosingEndTag => 1,
        TagName if parent == Some(MismatchedCloseTag) => {
            spans.push(HighlightSpan {
                range,
                tags: &TAGS[10..],
            });
            return;
        }
        TagName => 2,
        AttributeName => 3,
        AttributeValue | UnquotedAttributeValue => 4,
        Is => 5,
        EntityReference | CharacterReference => 6,
        Comment => 7,
        ProcessingInst => 8,
        DoctypeDecl => 9,
        _ => return,
    };
    spans.push(HighlightSpan {
        range,
        tags: std::slice::from_ref(&TAGS[index]),
    });
}
