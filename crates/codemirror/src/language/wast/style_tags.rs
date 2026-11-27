use std::{fmt, ops::Range};

use crate::language::{
    HighlightTag,
    lezer::{self, NodeType, StyleRules, StyledNode, SyntaxNode, select_tags},
};

use super::{WastNode, WastNodeKind, WastPropSource};

pub use lezer::StyleTags as WastStyleTags;

/// An invalid WAST style selector path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WastStyleSelectorError(lezer::StyleSelectorError);

impl fmt::Display for WastStyleSelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid WAST style path: {}", self.0.path())
    }
}

impl std::error::Error for WastStyleSelectorError {}

/// Build a WAST style property source. See [`lezer::style_tags`] for paths.
pub fn style_tags(
    spec: impl IntoIterator<Item = (impl AsRef<str>, Vec<HighlightTag>)>,
) -> Result<WastPropSource, WastStyleSelectorError> {
    lezer::style_tags(spec).map_err(WastStyleSelectorError)
}

pub(super) fn stock_rules(name: &str) -> Option<StyleRules> {
    use WastNodeKind::*;
    let kind = [
        OpenParen,
        CloseParen,
        Identifier,
        Type,
        Keyword,
        Number,
        String,
        LineComment,
        BlockComment,
    ]
    .into_iter()
    .find(|kind| kind.name() == name)?;
    Some(StyleRules::normal(kind.highlight()?))
}

impl<'a> WastNode<'a> {
    /// Select the first matching rule. An explicit empty rule returns `Some`.
    pub fn style_tags(self) -> Option<WastStyleTags<'a>> {
        let ancestors = std::iter::successors(self.parent(), |node| node.parent())
            .map(|node| node.kind().name());
        select_tags(self.node_type(), ancestors)
    }
}

impl<'a> SyntaxNode<'a> {
    /// Read a raw WAST node's style with ancestor names in root-to-parent order.
    pub fn wast_style_tags(self, ancestors: &[&str]) -> Option<WastStyleTags<'a>> {
        self.wast_node_type()?;
        self.style_tags(ancestors)
    }
}

impl<'a> StyledNode<'a> for WastNode<'a> {
    fn child(self, index: usize) -> Option<Self> {
        self.children().nth(index)
    }

    fn range(self) -> Range<usize> {
        self.range()
    }

    fn name(self) -> &'a str {
        self.kind().name()
    }

    fn node_type(self) -> Option<&'a NodeType> {
        Some(self.node_type())
    }
}
