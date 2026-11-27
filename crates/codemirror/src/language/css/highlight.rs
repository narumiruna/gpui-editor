// Style tags from @lezer/css 1.3.9, src/highlight.js.
use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    CssNodeKind,
    CssNodeKind::*,
    Node,
};

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    let mut spans: Vec<HighlightSpan<'static>> = Vec::new();
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

fn tags(kind: CssNodeKind) -> Option<&'static [HighlightTag]> {
    static TAGS: LazyLock<[HighlightTag; 25]> = LazyLock::new(|| {
        let mut tags = [
            "definitionKeyword",
            "keyword",
            "namespace",
            "labelName",
            "operatorKeyword",
            "tagName",
            "className",
            "className",
            "propertyName",
            "attributeName",
            "number",
            "atom",
            "variableName",
            "unit",
            "definitionOperator",
            "compareOperator",
            "logicOperator",
            "arithmeticOperator",
            "modifier",
            "blockComment",
            "color",
            "string",
            "punctuation",
            "separator",
            "paren",
        ]
        .map(HighlightTag::new);
        tags[7] = HighlightTag::new("className").with_modifier(TagModifier::Constant);
        tags
    });
    static BRACKETS: LazyLock<[HighlightTag; 2]> =
        LazyLock::new(|| ["squareBracket", "brace"].map(HighlightTag::new));
    let index = match kind {
        AtKeyword | Import | Charset | Namespace | Keyframes | Media | Supports
        | FontFeatureValues => 0,
        To | Selector | Scope | MatchFlag | KeywordQuery => 1,
        NamespaceName => 2,
        KeyframeName | IdName => 3,
        KeyframeRangeName | UnaryQueryOp | Callee => 4,
        TagName => 5,
        ClassName => 6,
        PseudoClassName => 7,
        FeatureName | PropertyName => 8,
        AttributeName => 9,
        NumberLiteral => 10,
        CallTag | ValueName | FontName => 11,
        VariableName => 12,
        Unit => 13,
        UniversalSelector | NestingSelector => 14,
        MatchOp | CompareOp => 15,
        ChildOp | SiblingOp | LogicOp => 16,
        BinOp => 17,
        Important => 18,
        Comment => 19,
        ColorLiteral => 20,
        ParenthesizedContent | StringLiteral => 21,
        Colon => 22,
        Semicolon | Comma | Pipe => 23,
        OpenParen | CloseParen => 24,
        OpenBracket | CloseBracket => return Some(&BRACKETS[0..1]),
        OpenBrace | CloseBrace => return Some(&BRACKETS[1..2]),
        _ => return None,
    };
    Some(&TAGS[index..=index])
}
