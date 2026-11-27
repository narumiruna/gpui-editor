// Style tags from @codemirror/lang-less 6.0.2, dist/index.js.
use std::sync::LazyLock;

use crate::language::{HighlightSpan, HighlightTag, TagModifier};

use super::{LessNodeKind, LessNodeKind::*, Node};

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    let mut spans = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut emit = |kind, start, end| {
        if start < end
            && let Some(tags) = tags(kind)
        {
            spans.push(HighlightSpan {
                range: start..end,
                tags,
            });
        }
    };
    for (id, node) in nodes.iter().enumerate() {
        if node.range.is_empty() {
            continue;
        }
        while open.last().is_some_and(|&(id, _)| Some(id) != node.parent) {
            let (i, start) = open.pop().expect("open node");
            emit(nodes[i].kind, start, nodes[i].range.end);
        }
        if let Some((i, start)) = open.last_mut() {
            emit(nodes[*i].kind, *start, node.range.start);
            *start = node.range.end;
        }
        open.push((id, node.range.start));
    }
    for (i, start) in open.into_iter().rev() {
        emit(nodes[i].kind, start, nodes[i].range.end);
    }
    spans
}

fn tags(kind: LessNodeKind) -> Option<&'static [HighlightTag]> {
    static TAGS: LazyLock<[HighlightTag; 29]> = LazyLock::new(|| {
        let mut tags = [
            "definitionKeyword",
            "keyword",
            "namespace",
            "labelName",
            "tagName",
            "className",
            "className",
            "propertyName",
            "attributeName",
            "number",
            "atom",
            "variableName",
            "variableName",
            "operatorKeyword",
            "unit",
            "definitionOperator",
            "compareOperator",
            "logicOperator",
            "arithmeticOperator",
            "modifier",
            "blockComment",
            "color",
            "string",
            "string",
            "punctuation",
            "derefOperator",
            "separator",
            "paren",
            "squareBracket",
        ]
        .map(HighlightTag::new);
        tags[6] = HighlightTag::new("className").with_modifier(TagModifier::Constant);
        tags[12] = HighlightTag::new("variableName").with_modifier(TagModifier::Special);
        tags[23] = HighlightTag::new("string").with_modifier(TagModifier::Special);
        tags
    });
    static BRACE: LazyLock<[HighlightTag; 1]> = LazyLock::new(|| [HighlightTag::new("brace")]);
    let index = match kind {
        Import | Charset | Namespace | Keyframes | Media | Supports | When => 0,
        From | To | Selector | KeywordQuery => 1,
        NamespaceName => 2,
        KeyframeName | IdName => 3,
        TagName => 4,
        ClassName => 5,
        PseudoClassName => 6,
        FeatureName | PropertyName | PropertyVariable => 7,
        AttributeName => 8,
        NumberLiteral => 9,
        CallTag | ValueName => 10,
        VariableName => 11,
        AtKeyword | Interpolation => 12,
        UnaryQueryOp => 13,
        Unit => 14,
        UniversalSelector | NestingSelector => 15,
        MatchOp => 16,
        // The upstream style key is `SiblingOp,`, not the node name `SiblingOp`.
        ChildOp | LogicOp => 17,
        BinOp => 18,
        Important => 19,
        Comment | LineComment => 20,
        ColorLiteral => 21,
        ParenthesizedContent | StringLiteral => 22,
        Escape => 23,
        Colon => 24,
        Hash => 25,
        Semicolon | Comma => 26,
        OpenParen | CloseParen => 27,
        OpenBracket | CloseBracket => 28,
        OpenBrace | CloseBrace => return Some(BRACE.as_slice()),
        _ => return None,
    };
    Some(&TAGS[index..=index])
}
