// Style tags from @lezer/sass 1.1.0 src/highlight.js. See docs/licenses/codemirror-lang-sass.txt.
use std::sync::LazyLock;

use super::super::{HighlightSpan, HighlightTag, TagModifier};
use super::{Node, parser};

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    let mut spans = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut emit = |id: usize, start, end| {
        if start < end
            && let Some(tags) = tags(parser::name(nodes[id].term))
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
            let (id, start) = open.pop().unwrap();
            emit(id, start, nodes[id].range.end);
        }
        if let Some((parent, start)) = open.last_mut() {
            emit(*parent, *start, node.range.start);
            *start = node.range.end;
        }
        open.push((id, node.range.start));
    }
    for (id, start) in open.into_iter().rev() {
        emit(id, start, nodes[id].range.end);
    }
    spans
}

fn tags(name: &str) -> Option<&'static [HighlightTag]> {
    static TAGS: LazyLock<[HighlightTag; 32]> = LazyLock::new(|| {
        let mut tags = [
            "definitionKeyword",
            "keyword",
            "controlKeyword",
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
            "variableName",
            "unit",
            "definitionOperator",
            "compareOperator",
            "logicOperator",
            "arithmeticOperator",
            "modifier",
            "blockComment",
            "lineComment",
            "color",
            "string",
            "meta",
            "punctuation",
            "derefOperator",
            "separator",
            "paren",
            "squareBracket",
            "brace",
        ]
        .map(HighlightTag::new);
        tags[8] = HighlightTag::new("className").with_modifier(TagModifier::Constant);
        tags[14] = HighlightTag::new("variableName").with_modifier(TagModifier::Special);
        tags
    });
    let i = match name {
        "AtKeyword" | "import" | "charset" | "namespace" | "keyframes" | "media" | "supports"
        | "include" | "mixin" | "use" | "forward" | "extend" | "at-root" => 0,
        "Keyword" | "KeywordQuery" | "selector" => 1,
        "ControlKeyword" => 2,
        "NamespaceName" => 3,
        "KeyframeName" | "IdName" => 4,
        "KeyframeRangeName" | "UnaryQueryOp" | "Callee" => 5,
        "TagName" => 6,
        "ClassName" | "Suffix" => 7,
        "PseudoClassName" => 8,
        "FeatureName" | "PropertyName" => 9,
        "AttributeName" => 10,
        "NumberLiteral" => 11,
        "CallTag" | "ValueName" => 12,
        "VariableName" => 13,
        "SassVariableName" => 14,
        "Unit" => 15,
        "UniversalSelector" | "NestingSelector" | "IndentedMixin" | "IndentedInclude" => 16,
        "MatchOp" => 17,
        // The upstream selector names `SiblingOp,` (with a comma), not `SiblingOp`.
        "ChildOp" | "LogicOp" => 18,
        "BinOp" => 19,
        "Important" | "Global" | "Default" => 20,
        "Comment" => 21,
        "LineComment" => 22,
        "ColorLiteral" => 23,
        "ParenthesizedContent" | "StringLiteral" => 24,
        "InterpolationStart" | "InterpolationContinue" | "InterpolationEnd" => 25,
        ":" | "..." => 26,
        "#" | "PseudoOp" => 27,
        ";" | "," => 28,
        "(" | ")" => 29,
        "[" | "]" => 30,
        "{" | "}" => 31,
        _ => return None,
    };
    Some(&TAGS[i..=i])
}
