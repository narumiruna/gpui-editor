use std::ops::Range;

use crate::{
    Assoc, autocomplete::brackets::CloseBracketSyntax, language::line_projection::splits_crlf,
};

use super::{
    super::brackets::{
        BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch, BracketMates,
        bracket_pairs, directional_bracket_near, editor_bracket_index,
        match_configured_editor_text, match_editor_siblings, match_editor_text,
        match_named_siblings, named_delimiter, node_delimiter,
    },
    LezerDocument, LezerNode, LezerNodeKind,
};

impl LezerNodeKind {
    // The pinned grammar's `closedBy` and `openedBy` properties. Only the
    // punctuation nodes carry them, not the characters inside other tokens.
    pub(super) const fn closed_by(self) -> Option<Self> {
        match self {
            Self::OpenParen => Some(Self::CloseParen),
            Self::OpenBracket => Some(Self::CloseBracket),
            Self::OpenBrace => Some(Self::CloseBrace),
            _ => None,
        }
    }

    const fn opened_by(self) -> Option<Self> {
        match self {
            Self::CloseParen => Some(Self::OpenParen),
            Self::CloseBracket => Some(Self::OpenBracket),
            Self::CloseBrace => Some(Self::OpenBrace),
            _ => None,
        }
    }

    fn matching(self, forward: bool) -> Option<Self> {
        if forward {
            self.closed_by()
        } else {
            self.opened_by()
        }
    }

    fn delimiter_index(self) -> Option<usize> {
        if self.closed_by().is_some() || self.opened_by().is_some() {
            editor_bracket_index(self.name().as_bytes()[0])
        } else {
            None
        }
    }
}

impl LezerDocument {
    /// Find the bracket next to this UTF-8 caret, preferring the bracket on
    /// its left. Marked delimiters match syntax siblings; other bracket
    /// characters can match only within the same syntax type.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        if !self.text.is_char_boundary(head) {
            return None;
        }
        let source = self.text.as_str().as_bytes();
        [head.checked_sub(1), Some(head)]
            .into_iter()
            .flatten()
            .find_map(|at| self.match_bracket(at, editor_bracket_index(*source.get(at)?)?))
    }

    /// Find a bracket near a UTF-8 caret with explicit matching options.
    /// Marked delimiters use the grammar; only plain scans use the UTF-16 limit.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        directional_bracket_near(self.text.as_str(), head, config.after_cursor, |pos, dir| {
            self.match_bracket_at(pos, dir, config)
        })
    }

    /// Match from this UTF-8 byte position and direction. `after_cursor`
    /// applies only to caret queries. Positions inside CRLF return `None`.
    pub fn match_bracket_at(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let source = self.text.as_str();
        if !self.text.is_char_boundary(pos) || splits_crlf(source, pos) {
            return None;
        }
        let side = if direction == Forward {
            Assoc::After
        } else {
            Assoc::Before
        };
        let node = self.resolve_at(pos, Some(side));
        let pairs = bracket_pairs(config);
        for token in std::iter::successors(Some(node), |node| node.parent()) {
            let range = token.range();
            let touches = if direction == Forward {
                range.start <= pos && pos < range.end
            } else {
                range.start < pos && pos <= range.end
            };
            if touches && let Some(expected) = matching_names(token, direction, pairs) {
                return Some(match_named_siblings(
                    range,
                    direction,
                    expected,
                    token.parent().into_iter().flat_map(LezerNode::children),
                    |sibling| (sibling.range(), sibling.kind().name()),
                    |sibling, dir| matching_names(sibling, dir, pairs).is_some(),
                ));
            }
        }
        let token = node.data();
        match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
            let candidate = self.resolve_at(at, Some(Assoc::After)).data();
            !splits_crlf(source, at)
                && !splits_crlf(source, at + 1)
                && candidate.term == token.term
                && candidate.kind == token.kind
        })
        .filter(|hit| !splits_crlf(source, hit.start.start) && !splits_crlf(source, hit.start.end))
    }

    /// Find the gap between adjacent grammar delimiters at a byte offset.
    /// The newline command checks line boundaries and whitespace.
    pub fn bracket_gap_for_indent(&self, position: usize) -> Option<Range<usize>> {
        if !self.text.is_char_boundary(position) {
            return None;
        }
        let node = self.resolve_at(position, None);
        let open = node
            .children()
            .rev()
            .find(|child| child.range().start < position)?;
        let close = node
            .children()
            .find(|child| child.range().start >= position)?;
        let start = open.range().end;
        (start <= position && open.kind().closed_by() == Some(close.kind()))
            .then(|| start..close.range().start)
    }

    /// Adapt this tree to quote input in a host document. `origin` is the
    /// byte offset of the grammar body in that document.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        QuoteSyntax { doc: self, origin }
    }

    fn resolve_at(&self, at: usize, side: Option<Assoc>) -> LezerNode<'_> {
        let mut node = self.root();
        loop {
            let children = &node.data().children;
            let i = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < at || (end == at && side != Some(Assoc::Before))
            });
            let Some(&id) = children.get(i) else {
                return node;
            };
            let start = self.nodes[id].range.start;
            if start > at || (start == at && side != Some(Assoc::After)) {
                return node;
            }
            node.id = id;
        }
    }

    fn match_bracket(&self, at: usize, index: usize) -> Option<BracketMatch> {
        let forward = index.is_multiple_of(2);
        let (pos, side) = if forward {
            (at, Assoc::After)
        } else {
            (at + 1, Assoc::Before)
        };
        let node = self.resolve_at(pos, Some(side));
        for ancestor in std::iter::successors(Some(node), |node| node.parent()) {
            if ancestor.kind().matching(forward).is_some() {
                let siblings = ancestor
                    .parent()
                    .into_iter()
                    .flat_map(LezerNode::children)
                    .filter_map(|sibling| {
                        Some((sibling.range(), sibling.kind().delimiter_index()?))
                    });
                return Some(match_editor_siblings(ancestor.range(), index, siblings));
            }
        }
        let token = node.data();
        match_editor_text(self.syntax_lines(), at..at + 1, index, |at| {
            let candidate = self.resolve_at(at, Some(Assoc::After)).data();
            candidate.term == token.term && candidate.kind == token.kind
        })
    }
}

fn matching_names<'a>(
    node: LezerNode<'a>,
    direction: BracketDirection,
    pairs: &'a str,
) -> Option<BracketMates<'a>> {
    if let Some(row) = node.node_type() {
        return node_delimiter(row, direction, pairs);
    }
    // Local fallback nodes have no verified LR terms.
    let kind = node.kind();
    if let Some(mate) = kind.matching(direction == Forward) {
        return Some(BracketMates::Text(Some(mate.name())));
    }
    named_delimiter(kind.name(), direction, pairs).map(Into::into)
}

struct QuoteSyntax<'a> {
    doc: &'a LezerDocument,
    origin: usize,
}

impl CloseBracketSyntax for QuoteSyntax<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        let at = pos.checked_sub(self.origin)?;
        self.doc
            .text
            .is_char_boundary(at)
            .then(|| self.doc.resolve_at(at, side).id)
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
