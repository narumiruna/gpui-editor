use std::{ops::Range, sync::LazyLock};

use super::{AngularDocument, AngularNode};
use crate::language::{HighlightSpan, HighlightTag, TagModifier};

static TAGS: LazyLock<[HighlightTag; 12]> = LazyLock::new(|| {
    let mut tags = [
        "content",
        "definitionOperator",
        "attributeName",
        "attributeValue",
        "character",
        "invalid",
        "attributeName",
        "variableName",
        "keyword",
        "brace",
        "paren",
        "bracket",
    ]
    .map(HighlightTag::new);
    tags[6] = tags[6].clone().with_modifier(TagModifier::Special);
    tags
});
static PUNCTUATION: LazyLock<HighlightTag> = LazyLock::new(|| HighlightTag::new("punctuation"));

fn tags(node: AngularNode<'_>) -> Option<&'static [HighlightTag]> {
    let index = match node.name() {
        "Text" => 0,
        "Is" => 1,
        "AttributeName" if node.children().len() == 0 => 2,
        "AttributeValue" | "ExpressionAttributeValue" | "StatementAttributeValue" => 3,
        "Entity" => 4,
        "InvalidEntity" => 5,
        "Identifier" => match node.parent()?.name() {
            "BoundAttributeName" => 2,
            "EventName" => 6,
            "ReferenceName" => 7,
            "DirectiveName" => 8,
            _ => return None,
        },
        "{{" | "}}" => 9,
        "(" | ")" => 10,
        "[" | "]" => 11,
        "#" => return Some(std::slice::from_ref(&PUNCTUATION)),
        _ => return None,
    };
    Some(std::slice::from_ref(&TAGS[index]))
}

pub(super) fn collect(doc: &AngularDocument, range: Range<usize>) -> Vec<HighlightSpan<'_>> {
    if range.is_empty() {
        return Vec::new();
    }
    let regions = doc.regions();
    let from = regions.partition_point(|region| region.root().range().end <= range.start);
    let to = regions.partition_point(|region| region.root().range().start < range.end);
    let visible = &regions[from..to];
    let mut spans = Vec::new();
    for span in doc.html().highlights(range.clone()).into_iter().flatten() {
        let from = visible.partition_point(|region| region.root().range().end <= span.range.start);
        let covered = visible[from..]
            .iter()
            .filter(|region| region.replaces_html())
            .map(|region| region.root().range());
        add_gaps(&mut spans, span.range, covered, span.tags);
    }
    for region in visible {
        for node in region.nodes() {
            let Some(tags) = tags(node) else { continue };
            let clipped = clip(node.range(), &range);
            add_gaps(
                &mut spans,
                clipped,
                node.children().map(|child| child.range()),
                tags,
            );
        }
        for script in region.scripts() {
            let full = script.range();
            let clipped = clip(full.clone(), &range);
            if clipped.start >= clipped.end {
                continue;
            }
            let offset = full.start;
            let query = clipped.start - offset..clipped.end - offset;
            for highlight in script.document().highlights(query).into_iter().flatten() {
                spans.push(HighlightSpan {
                    range: highlight.range.start + offset..highlight.range.end + offset,
                    tags: highlight.tags,
                });
            }
        }
    }
    spans.sort_unstable_by_key(|span| span.range.start);
    spans
}

fn clip(span: Range<usize>, query: &Range<usize>) -> Range<usize> {
    span.start.max(query.start)..span.end.min(query.end)
}

fn add_gaps<'a>(
    spans: &mut Vec<HighlightSpan<'a>>,
    range: Range<usize>,
    occupied: impl Iterator<Item = Range<usize>>,
    tags: &'a [HighlightTag],
) {
    let mut start = range.start;
    for area in occupied {
        if area.start >= range.end {
            break;
        }
        if area.end > area.start && area.end > start {
            if start < area.start {
                spans.push(HighlightSpan {
                    range: start..area.start,
                    tags,
                });
            }
            start = area.end;
        }
    }
    if start < range.end {
        spans.push(HighlightSpan {
            range: start..range.end,
            tags,
        });
    }
}
