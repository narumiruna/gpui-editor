use std::sync::LazyLock;

use super::{
    super::{
        HighlightSpan, HighlightTag, TagModifier,
        lezer::{NodePropSource, style_tags},
    },
    LezerNodeKind::*,
    Node,
};

fn tags() -> &'static [[HighlightTag; 1]; 18] {
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
    &TAGS
}

pub(super) fn stock_styles() -> NodePropSource {
    let selectors = [
        "LineComment",
        "BlockComment",
        "AnyChar",
        "Literal",
        "tokens from grammar as empty prop extend specialize AtName",
        "@top @left @right @cut @external",
        "@precedence @tokens @context @dialects @skip @detectDelim @conflict",
        "@extend @specialize",
        "CharSet InvertedCharSet",
        "CharClass",
        "RuleName",
        "RuleDeclaration/RuleName InlineRule/RuleName TokensBody/RuleName",
        "PrecedenceName",
        "Name",
        "( )",
        "[ ]",
        "{ }",
        "\"!\" ~ \"*\" + ? |",
    ];
    style_tags(
        selectors
            .into_iter()
            .zip(tags())
            .map(|(path, tags)| (path, tags.to_vec())),
    )
    .expect("verified Lezer highlight paths")
}

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    let tags = tags();
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
                tags: &tags[tag],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::Text;

    use super::{
        super::{LezerDocument, parser, stock_node_types},
        *,
    };

    #[test]
    fn local_fallback_keeps_stock_highlights_without_verified_terms() {
        let source = r#"@top Main { "字" }"#;
        let properties =
            [style_tags([("Grammar/...", vec![HighlightTag::new("custom-root")])]).unwrap()];
        let configured = LezerDocument::from_nodes(Text::new(source), parser::parse(source), None)
            .with_node_types(stock_node_types().extend(&properties));
        assert!(
            configured
                .nodes()
                .skip(1)
                .all(|node| node.node_type().is_none())
        );
        let start = source.find('"').unwrap();
        let end = source.rfind('"').unwrap() + 1;
        let spans: Vec<_> = configured.highlights(start..end).unwrap().collect();
        assert_eq!(
            spans,
            [HighlightSpan {
                range: start..end,
                tags: &tags()[3]
            }]
        );
    }
}
