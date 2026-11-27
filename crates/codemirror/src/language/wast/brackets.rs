use std::ops::Range;

use crate::{Assoc, autocomplete::brackets::CloseBracketSyntax};

use super::{
    super::{
        brackets::{
            BracketConfig, BracketDirection,
            BracketDirection::{Backward, Forward},
            BracketMatch, BracketMates, bracket_pairs, directional_bracket_near,
            editor_bracket_index, match_configured_editor_text, match_editor_siblings,
            match_editor_text, node_delimiter,
        },
        line_projection::splits_crlf,
    },
    BRACKET_MATCHING_HANDLE, WastDocument, WastNode, WastNodeKind,
};

impl WastDocument {
    /// Find a stock bracket next to this UTF-8 caret, preferring the bracket
    /// on its left. Stock parentheses match syntax siblings. The plain scan
    /// matches the same syntax type and measures normalized UTF-16 distance.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        if !self.text.is_char_boundary(head) {
            return None;
        }
        let bytes = self.text.as_str().as_bytes();
        [head.checked_sub(1), Some(head)]
            .into_iter()
            .flatten()
            .find_map(|at| self.match_stock_bracket(at, editor_bracket_index(*bytes.get(at)?)?))
    }

    /// Find a bracket near a UTF-8 caret with explicit matching options.
    /// Marked delimiters use node props; only plain scans use the UTF-16 limit.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        directional_bracket_near(self.text.as_str(), head, config.after_cursor, |pos, dir| {
            self.match_bracket(pos, dir, config)
        })
    }

    /// Match from a UTF-8 position using the specified direction. A forward
    /// probe includes the start of a handle; a backward probe includes its end.
    /// Invalid positions and positions inside CRLF return `None`.
    /// `config.after_cursor` applies to caret queries only.
    pub fn match_bracket(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let source = self.text.as_str();
        if !self.text.is_char_boundary(pos) || splits_crlf(source, pos) {
            return None;
        }
        let forward = direction == Forward;
        let side = if forward { Assoc::After } else { Assoc::Before };
        let node = self.resolve_at(pos, Some(side));
        let pairs = bracket_pairs(config);
        for token in std::iter::successors(Some(node), |node| node.parent()) {
            let Some(mates) = matching_names(token, direction, pairs) else {
                continue;
            };
            if token.data().range.is_empty() {
                continue;
            }
            let Some(start) = self.bracket_handle(token).filter(|r| {
                if forward {
                    r.start <= pos && pos < r.end
                } else {
                    r.start < pos && pos <= r.end
                }
            }) else {
                continue;
            };
            return Some(self.match_marked(token, start, mates, direction, pairs));
        }
        // The plain scan can see both bytes of CRLF. Neither byte alone is
        // a valid delimiter range, even if listed in the text options.
        match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
            !splits_crlf(source, at)
                && !splits_crlf(source, at + 1)
                && self.resolve_at(at, Some(Assoc::After)).kind() == node.kind()
        })
        .filter(|hit| !splits_crlf(source, hit.start.start) && !splits_crlf(source, hit.start.end))
    }

    fn bracket_handle(&self, token: WastNode<'_>) -> Option<Range<usize>> {
        let handle = match token.prop(&BRACKET_MATCHING_HANDLE) {
            Some(Some(select)) => select(token)?,
            _ => token,
        };
        let range = handle.range();
        let source = self.text.as_str();
        (std::ptr::eq(handle.nodes, token.nodes)
            && !range.is_empty()
            && self.text.is_valid_range(range.clone())
            && !splits_crlf(source, range.start)
            && !splits_crlf(source, range.end))
        .then_some(range)
    }

    fn match_marked(
        &self,
        token: WastNode<'_>,
        start: Range<usize>,
        mates: BracketMates<'_>,
        direction: BracketDirection,
        pairs: &str,
    ) -> BracketMatch {
        let mut hit = BracketMatch {
            start,
            end: None,
            matched: false,
        };
        let Some(parent) = token.parent() else {
            return hit;
        };
        let range = &token.data().range;
        let children = &parent.data().children;
        let forward = direction == Forward;
        let opposite = if forward { Backward } else { Forward };
        // Start at the equivalent of Lezer's childAfter/childBefore. The
        // boundary belongs to the marked token, not its highlight handle.
        let mut siblings = if forward {
            let first = children.partition_point(|&id| self.nodes[id].range.end <= range.end);
            children[first..].iter()
        } else {
            let end = children.partition_point(|&id| self.nodes[id].range.start < range.start);
            children[..end].iter()
        };
        let mut depth = 0;
        while let Some(&id) = if forward {
            siblings.next()
        } else {
            siblings.next_back()
        } {
            let sibling = WastNode { id, ..token };
            let other = &sibling.data().range;
            if (forward && other.start < range.end) || (!forward && other.end > range.start) {
                continue;
            }
            if depth == 0 && !other.is_empty() && mates.contains(sibling.node_type().name()) {
                hit.matched = true;
                hit.end = self.bracket_handle(sibling);
                break;
            }
            if matching_names(sibling, direction, pairs).is_some() {
                depth += 1;
            } else if matching_names(sibling, opposite, pairs).is_some() {
                if depth == 0 {
                    hit.end = self.bracket_handle(sibling);
                    break;
                }
                depth -= 1;
            }
        }
        hit
    }

    /// The candidate indentation gap between adjacent parentheses of one app.
    /// The newline command checks the gap's whitespace and line bounds.
    pub fn bracket_gap_for_indent(&self, position: usize) -> Option<Range<usize>> {
        if !self.text.is_char_boundary(position) {
            return None;
        }
        let node = self.resolve_at(position, None);
        if node.kind() != WastNodeKind::App {
            return None;
        }
        let open = node
            .children()
            .rev()
            .find(|child| child.range().start < position)?;
        let close = node
            .children()
            .find(|child| child.range().start >= position)?;
        let start = open.range().end;
        (open.kind() == WastNodeKind::OpenParen
            && close.kind() == WastNodeKind::CloseParen
            && start <= position)
            .then(|| start..close.range().start)
    }

    /// Adapt this tree to quote input at the given byte offset in a host document.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        QuoteSyntax { doc: self, origin }
    }

    pub(super) fn resolve_at(&self, at: usize, side: Option<Assoc>) -> WastNode<'_> {
        self.root().resolve_at(at, side)
    }

    fn match_stock_bracket(&self, at: usize, index: usize) -> Option<BracketMatch> {
        let (pos, side) = if index.is_multiple_of(2) {
            (at, Assoc::After)
        } else {
            (at + 1, Assoc::Before)
        };
        let node = self.resolve_at(pos, Some(side));
        if delimiter_index(node.kind()) == Some(index) {
            let siblings = node
                .parent()?
                .children()
                .filter_map(|sibling| Some((sibling.range(), delimiter_index(sibling.kind())?)));
            return Some(match_editor_siblings(node.range(), index, siblings));
        }
        match_editor_text(self.syntax_lines(), at..at + 1, index, |other| {
            self.resolve_at(other, Some(Assoc::After)).kind() == node.kind()
        })
    }
}

fn delimiter_index(kind: WastNodeKind) -> Option<usize> {
    match kind {
        WastNodeKind::OpenParen => Some(0),
        WastNodeKind::CloseParen => Some(1),
        _ => None,
    }
}

fn matching_names<'a>(
    node: WastNode<'a>,
    direction: BracketDirection,
    pairs: &'a str,
) -> Option<BracketMates<'a>> {
    node_delimiter(node.node_type(), direction, pairs)
}

struct QuoteSyntax<'a> {
    doc: &'a WastDocument,
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
