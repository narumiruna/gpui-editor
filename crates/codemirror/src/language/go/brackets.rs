use std::ops::Range;

use super::{GoDocument, GoNode, GoNodeKind};
use crate::{
    Assoc,
    autocomplete::brackets::CloseBracketSyntax,
    language::brackets::{
        BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch, bracket_pairs,
        directional_bracket_near, editor_bracket_index, match_configured_editor_text,
        match_editor_siblings, match_editor_text, match_named_siblings, named_delimiter,
    },
};

impl GoDocument {
    /// Match a bracket next to the caret, with priority to its left.
    /// Positions and ranges use original UTF-8 bytes.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        if !self.text.is_char_boundary(head) {
            return None;
        }
        let bytes = self.text.as_str().as_bytes();
        [head.checked_sub(1), Some(head)]
            .into_iter()
            .flatten()
            .find_map(|at| self.match_bracket(at, editor_bracket_index(*bytes.get(at)?)?))
    }

    /// Find a bracket near a UTF-8 caret with explicit matching options.
    /// Marked delimiters use the grammar. Only plain scans use the UTF-16 limit.
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
                            token.parent().into_iter().flat_map(GoNode::children),
                            |sibling| (sibling.range(), sibling.kind().name()),
                            |sibling, dir| matching_name(sibling.kind(), dir, pairs).is_some(),
                        ));
                    }
                }
                let term = node.data().term;
                match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
                    self.path(at, Some(Assoc::After))
                        .last()
                        .is_some_and(|candidate| candidate.data().term == term)
                })
            },
        )
    }

    fn match_bracket(&self, at: usize, index: usize) -> Option<BracketMatch> {
        // A bracket is one byte. Resolving right from its start also finds
        // the token that resolving left from its end would find.
        let node = self.path(at, Some(Assoc::After)).last()?;
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
                .is_some_and(|candidate| candidate.data().term == node.data().term)
        })
    }

    /// Adapt Go nodes to quote input in a host document. `origin` is the
    /// byte offset of the Go body in the host.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        Quotes { doc: self, origin }
    }
}

struct Quotes<'a> {
    doc: &'a GoDocument,
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
    kind: GoNodeKind,
    direction: BracketDirection,
    pairs: &str,
) -> Option<Option<&str>> {
    use GoNodeKind::*;
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

fn delimiter(kind: GoNodeKind) -> Option<usize> {
    use GoNodeKind::*;
    // These are the only Go nodes with `closedBy` or `openedBy`.
    match kind {
        OpenParen | CloseParen | OpenBracket | CloseBracket | OpenBrace | CloseBrace => {
            editor_bracket_index(kind.name().as_bytes()[0])
        }
        _ => None,
    }
}
