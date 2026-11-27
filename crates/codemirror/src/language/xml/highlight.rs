use std::{ops::Range, sync::LazyLock};

use super::{
    HighlightSpan, Node,
    XmlNodeKind::{self, *},
};
use crate::language::{HighlightTag, TagModifier};

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    let mut spans = Vec::new();
    for node in nodes {
        let parent = node.parent.map(|id| nodes[id].kind);
        if parent == Some(AttributeValue) {
            continue;
        }
        if node.kind == AttributeValue {
            let mut start = node.range.start;
            for &child in &node.children {
                let child = &nodes[child];
                push(&mut spans, start..child.range.start, AttributeValue, None);
                push(&mut spans, child.range.clone(), child.kind, None);
                start = child.range.end;
            }
            push(&mut spans, start..node.range.end, AttributeValue, None);
        } else {
            push(&mut spans, node.range.clone(), node.kind, parent);
        }
    }
    spans
}

fn push(
    spans: &mut Vec<HighlightSpan<'static>>,
    range: Range<usize>,
    kind: XmlNodeKind,
    parent: Option<XmlNodeKind>,
) {
    if range.is_empty() {
        return;
    }
    static TAGS: LazyLock<[HighlightTag; 13]> = LazyLock::new(|| {
        let mut tags = [
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
            "string",
            "tagName",
            "invalid",
        ]
        .map(HighlightTag::new);
        tags[10] = HighlightTag::new("string").with_modifier(TagModifier::Special);
        tags
    });
    let index = match kind {
        Text => 0,
        StartTag | StartCloseTag | EndTag | SelfCloseEndTag => 1,
        TagName if parent == Some(MismatchedCloseTag) => {
            spans.push(HighlightSpan {
                range,
                tags: &TAGS[11..],
            });
            return;
        }
        TagName => 2,
        AttributeName => 3,
        AttributeValue => 4,
        Is => 5,
        EntityReference | CharacterReference => 6,
        Comment => 7,
        ProcessingInst => 8,
        DoctypeDecl => 9,
        Cdata => 10,
        _ => return,
    };
    spans.push(HighlightSpan {
        range,
        tags: std::slice::from_ref(&TAGS[index]),
    });
}
