use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    LezerNodeKind::*,
    Node,
};

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    static TAGS: LazyLock<[[HighlightTag; 1]; 18]> = LazyLock::new(|| {
        let mut tags = [
            "lineComment",
            "blockComment",
            "character",
            "string",
            "keyword",
            "modifier",
            "definitionKeyword",
            "operatorKeyword",
            "regexp",
            "atom",
            "variableName",
            "variableName",
            "labelName",
            "name",
            "paren",
            "squareBracket",
            "brace",
            "operator",
        ]
        .map(|name| [HighlightTag::new(name)]);
        tags[11] = [HighlightTag::new("variableName").with_modifier(TagModifier::Definition)];
        tags
    });
    nodes
        .iter()
        .filter(|node| !node.range.is_empty())
        .filter_map(|node| {
            let tag = match node.kind {
                LineComment => 0,
                BlockComment => 1,
                AnyChar => 2,
                Literal => 3,
                Tokens | From | As | PropKeyword | Extend | Specialize | AtName => 4,
                AtTop | AtLeft | AtRight | AtCut | AtExternal => 5,
                AtPrecedence | AtTokens | AtContext | AtDialects | AtSkip | AtDetectDelim
                | AtConflict => 6,
                AtExtend | AtSpecialize => 7,
                CharSet | InvertedCharSet => 8,
                CharClass => 9,
                RuleName
                    if node.parent.is_some_and(|id| {
                        matches!(nodes[id].kind, RuleDeclaration | InlineRule | TokensBody)
                    }) =>
                {
                    11
                }
                RuleName => 10,
                PrecedenceName => 12,
                Name => 13,
                OpenParen | CloseParen => 14,
                OpenBracket | CloseBracket => 15,
                OpenBrace | CloseBrace => 16,
                Bang | Tilde | Star | Plus | Question | Bar => 17,
                _ => return None,
            };
            Some(HighlightSpan {
                range: node.range.clone(),
                tags: &TAGS[tag],
            })
        })
        .collect()
}
