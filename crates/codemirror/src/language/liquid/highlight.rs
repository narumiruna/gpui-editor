use std::sync::LazyLock;

use super::{
    super::{HighlightTag, TagModifier},
    HighlightSpan,
    LiquidNodeKind::*,
    Node,
};

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    static TAGS: LazyLock<[HighlightTag; 22]> = LazyLock::new(|| {
        let mut tags = [
            "keyword",
            "atom",
            "controlKeyword",
            "definitionKeyword",
            "operatorKeyword",
            "moduleKeyword",
            "variableName",
            "variableName",
            "propertyName",
            "attributeName",
            "compareOperator",
            "definitionOperator",
            "logicOperator",
            "number",
            "string",
            "bool",
            "lineComment",
            "blockComment",
            "brace",
            "paren",
            "derefOperator",
            "punctuation",
        ]
        .map(HighlightTag::new);
        tags[7] = tags[7].clone().with_modifier(TagModifier::Function);
        tags
    });
    nodes
        .iter()
        .filter_map(|node| {
            if node.range.is_empty() {
                return None;
            }
            let index = match node.kind {
                Cycle | CommentKeyword | EndComment | Raw | EndRaw | Echo | Increment
                | Decrement | Liquid | In | With | As | TagName => 0,
                Empty | ForLoop | TableRowLoop => 1,
                If | Elsif | Else | EndIf | Unless | EndUnless | Case | EndCase | For | EndFor
                | TableRow | EndTableRow | Break | Continue => 2,
                Assign | Capture | EndCapture => 3,
                Contains => 4,
                Render | Include => 5,
                VariableName => 6,
                FilterName => 7,
                PropertyName => 8,
                ParameterName => 9,
                CompareOp => 10,
                AssignOp => 11,
                LogicOp => 12,
                NumberLiteral => 13,
                StringLiteral => 14,
                BooleanLiteral => 15,
                InlineComment => 16,
                CommentText => 17,
                TagStart | TagEnd | InterpolationStart | InterpolationEnd => 18,
                OpenParen | CloseParen => 19,
                Dot => 20,
                Comma | Dots | Colon | Pipe => 21,
                // The upstream style table does not give `when` a tag.
                _ => return None,
            };
            Some(HighlightSpan {
                range: node.range.clone(),
                tags: std::slice::from_ref(&TAGS[index]),
            })
        })
        .collect()
}
