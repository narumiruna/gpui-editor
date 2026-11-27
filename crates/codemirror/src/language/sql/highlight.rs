use std::sync::LazyLock;

use super::{HighlightSpan, Node, SqlNodeKind};
use crate::language::{HighlightTag, TagModifier};

static TAGS: LazyLock<[Option<[HighlightTag; 1]>; 31]> = LazyLock::new(|| {
    std::array::from_fn(|term| {
        use SqlNodeKind::*;
        let (name, modifier) = match SqlNodeKind::from_term(term as u16) {
            Keyword => ("keyword", None),
            Type => ("typeName", None),
            Builtin => ("name", Some(TagModifier::Standard)),
            Bits | Number => ("number", None),
            Bytes | String => ("string", None),
            Bool => ("bool", None),
            Null => ("null", None),
            Identifier => ("name", None),
            QuotedIdentifier => ("string", Some(TagModifier::Special)),
            SpecialVar => ("name", Some(TagModifier::Special)),
            LineComment => ("lineComment", None),
            BlockComment => ("blockComment", None),
            Operator => ("operator", None),
            Punctuation => ("punctuation", None),
            OpenParen | CloseParen => ("paren", None),
            OpenBrace | CloseBrace => ("brace", None),
            OpenBracket | CloseBracket => ("squareBracket", None),
            // The published style rule names `Semi`, but the visible node
            // is named `;`, so semicolons and dots receive no tag.
            _ => return None,
        };
        let tag = HighlightTag::new(name);
        Some([if let Some(modifier) = modifier {
            tag.with_modifier(modifier)
        } else {
            tag
        }])
    })
});

pub(super) fn collect(nodes: &[Node]) -> Vec<HighlightSpan<'static>> {
    nodes
        .iter()
        .filter_map(|node| {
            let tags = TAGS[node.kind as usize].as_ref()?;
            Some(HighlightSpan {
                range: node.range.clone(),
                tags,
            })
        })
        .collect()
}
