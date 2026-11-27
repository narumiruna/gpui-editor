use std::ops::Range;

use super::{RustDocument, RustNode, RustNodeKind};
use crate::{
    Assoc,
    autocomplete::brackets::CloseBracketSyntax,
    language::brackets::{
        BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch, bracket_pairs,
        directional_bracket_near, editor_bracket_index, match_configured_editor_text,
        match_editor_siblings, match_editor_text, match_named_siblings, named_delimiter,
    },
};

impl RustDocument {
    /// Match a bracket next to `head`, preferring the character on the left.
    /// Structural delimiters match siblings. Brackets in comments or literals
    /// can only pair with characters of the same syntax type. All offsets use
    /// original UTF-8 bytes.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        if !self.text().is_char_boundary(head) {
            return None;
        }
        let bytes = self.text().as_str().as_bytes();
        [head.checked_sub(1), Some(head)]
            .into_iter()
            .flatten()
            .find_map(|at| self.match_bracket(at, editor_bracket_index(*bytes.get(at)?)?))
    }

    /// Find a bracket near a UTF-8 caret with explicit matching options.
    /// Marked delimiters use the grammar. Only plain scans use the UTF-16 limit.
    /// A plain scan does not pair distinct nodes whose kind represents several terms.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let pairs = bracket_pairs(config);
        directional_bracket_near(
            self.text.as_str(),
            head,
            config.after_cursor,
            |pos, direction| {
                let side = if direction == Forward {
                    Assoc::After
                } else {
                    Assoc::Before
                };
                let node = self.path(pos, Some(side)).last()?;
                for token in std::iter::successors(Some(node), |node| node.parent()) {
                    if !token.data().range.is_empty()
                        && let Some(expected) = matching_name(token.kind(), direction, pairs)
                    {
                        return Some(match_named_siblings(
                            token.range(),
                            direction,
                            expected,
                            token.parent().into_iter().flat_map(RustNode::children),
                            |sibling| (sibling.range(), sibling.kind().name()),
                            |sibling, dir| matching_name(sibling.kind(), dir, pairs).is_some(),
                        ));
                    }
                }
                let kind = node.kind();
                let unique = has_unique_term(kind);
                match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
                    self.path(at, Some(Assoc::After))
                        .last()
                        .is_some_and(|candidate| {
                            candidate.id == node.id || (unique && candidate.kind() == kind)
                        })
                })
            },
        )
    }

    fn match_bracket(&self, at: usize, index: usize) -> Option<BracketMatch> {
        let (position, side) = if index.is_multiple_of(2) {
            (at, Assoc::After)
        } else {
            (at + 1, Assoc::Before)
        };
        let node = self.path(position, Some(side)).last()?;
        if delimiter(node.kind()) == Some(index) {
            let siblings = node
                .parent()?
                .children()
                .filter_map(|sibling| Some((sibling.range(), delimiter(sibling.kind())?)));
            return Some(match_editor_siblings(node.range(), index, siblings));
        }
        match_editor_text(self.syntax_lines(), at..at + 1, index, |other| {
            self.path(other, Some(Assoc::After))
                .last()
                .is_some_and(|candidate| candidate.kind() == node.kind())
        })
    }

    /// Adapt native Rust nodes to quote input in a host document. `origin`
    /// is the byte offset of the Rust body in the host.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        Quotes { doc: self, origin }
    }
}

struct Quotes<'a> {
    doc: &'a RustDocument,
    origin: usize,
}

impl CloseBracketSyntax for Quotes<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        let at = pos.checked_sub(self.origin)?;
        if !self.doc.text().is_char_boundary(at) {
            return None;
        }
        self.doc.path(at, side).last().map(|node| node.id)
    }

    fn range(&self, node: usize) -> Range<usize> {
        let range = &self.doc.nodes[node].range;
        self.origin + range.start..self.origin + range.end
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.doc.nodes[node].parent
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.doc.nodes[node].children.first().copied()
    }
}

fn matching_name(
    kind: RustNodeKind,
    direction: BracketDirection,
    pairs: &str,
) -> Option<Option<&str>> {
    use RustNodeKind::*;
    // Only these terms have delimiter props in @lezer/rust 1.0.3.
    // Angle brackets and closure pipes only match by configured name.
    let mate = match (kind, direction) {
        (OpenParen, Forward) => CloseParen,
        (OpenBracket, Forward) => CloseBracket,
        (OpenBrace, Forward) => CloseBrace,
        (CloseParen, BracketDirection::Backward) => OpenParen,
        (CloseBracket, BracketDirection::Backward) => OpenBracket,
        (CloseBrace, BracketDirection::Backward) => OpenBrace,
        _ => return named_delimiter(kind.name(), direction, pairs),
    };
    Some(Some(mate.name()))
}

fn has_unique_term(kind: RustNodeKind) -> bool {
    use RustNodeKind::*;
    // Native nodes retain names, not the distinct term IDs behind these names.
    !matches!(
        kind,
        Identifier
            | ScopedIdentifier
            | ScopedTypeIdentifier
            | ArithOp
            | LetDeclaration
            | BitOp
            | LogicOp
            | MatchArm
            | MacroInvocation
            | ParamList
            | Parameter
            | DeclarationList
            | AttributeItem
            | FunctionItem
            | VariadicParameter
            | StaticItem
            | ExpressionStatement
    )
}

fn delimiter(kind: RustNodeKind) -> Option<usize> {
    let &[byte] = kind.name().as_bytes() else {
        return None;
    };
    editor_bracket_index(byte)
}
