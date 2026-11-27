use std::{ops::Range, sync::LazyLock};

use super::{HighlightTag, MarkdownNode as Node, MarkdownNodeKind as Kind, MarkdownNodeKind::*};
use crate::language::TagModifier;

pub(super) fn collect(root: &Node) -> Vec<(Range<usize>, Vec<HighlightTag>)> {
    let mut collector = Collector::default();
    collector.visit(root, &[]);
    collector.start(root.range.end, &[]);
    collector.spans
}

#[derive(Default)]
struct Collector {
    start: usize,
    tags: Vec<HighlightTag>,
    spans: Vec<(Range<usize>, Vec<HighlightTag>)>,
}

impl Collector {
    fn start(&mut self, at: usize, tags: &[HighlightTag]) {
        if self.tags == tags {
            return;
        }
        if self.start < at && !self.tags.is_empty() {
            self.spans
                .push((self.start..at, std::mem::take(&mut self.tags)));
        }
        self.start = at;
        self.tags = tags.to_vec();
    }

    fn visit(&mut self, node: &Node, inherited: &[HighlightTag]) {
        let mut class = inherited.to_vec();
        if let Some(tag) = tag(node.kind) {
            class.push(tag.clone());
        }
        let descendants = if inherits(node.kind) {
            &class
        } else {
            inherited
        };
        self.start(node.range.start, &class);
        for child in &node.children {
            self.visit(child, descendants);
            self.start(child.range.end, &class);
        }
    }
}

fn inherits(kind: Kind) -> bool {
    kind.heading_level().is_some()
        || matches!(
            kind,
            Blockquote
                | Emphasis
                | StrongEmphasis
                | Link
                | Image
                | OrderedList
                | BulletList
                | Strikethrough
                | TableHeader
        )
}

fn tag(kind: Kind) -> Option<&'static HighlightTag> {
    static TAGS: LazyLock<[HighlightTag; 24]> = LazyLock::new(|| {
        [
            "heading1",
            "heading2",
            "heading3",
            "heading4",
            "heading5",
            "heading6",
            "quote",
            "contentSeparator",
            "comment",
            "escape",
            "character",
            "emphasis",
            "strong",
            "link",
            "list",
            "monospace",
            "url",
            "processingInstruction",
            "labelName",
            "string",
            "content",
            "strikethrough",
            "heading",
            "atom",
        ]
        .map(HighlightTag::new)
    });
    static SCRIPT: LazyLock<HighlightTag> =
        LazyLock::new(|| HighlightTag::new("content").with_modifier(TagModifier::Special));
    if let Some(level) = kind.heading_level() {
        return Some(&TAGS[usize::from(level - 1)]);
    }
    Some(
        &TAGS[match kind {
            Blockquote => 6,
            HorizontalRule => 7,
            Comment | CommentBlock => 8,
            Escape => 9,
            Entity | Emoji => 10,
            Emphasis => 11,
            StrongEmphasis => 12,
            Link | Image => 13,
            OrderedList | BulletList | Task => 14,
            InlineCode | CodeText => 15,
            URL | Autolink => 16,
            HeaderMark | HardBreak | QuoteMark | ListMark | LinkMark | EmphasisMark | CodeMark
            | StrikethroughMark | SubscriptMark | SuperscriptMark | TableDelimiter => 17,
            CodeInfo | LinkLabel => 18,
            LinkTitle => 19,
            Paragraph | TableCell => 20,
            Strikethrough => 21,
            TableHeader => 22,
            TaskMarker => 23,
            Subscript | Superscript => return Some(&SCRIPT),
            _ => return None,
        }],
    )
}
