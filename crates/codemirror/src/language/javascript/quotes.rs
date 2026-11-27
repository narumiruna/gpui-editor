use std::ops::Range;

use crate::{Assoc, autocomplete::brackets::CloseBracketSyntax};

use super::{JavascriptDocument, Node, path};

impl JavascriptDocument {
    /// Adapt JavaScript-family syntax to quote input at the host byte offset
    /// `origin`. Logical line breaks do not change the returned byte ranges.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        Quotes {
            doc: self,
            nodes: self.projected_bracket_syntax().0,
            origin,
        }
    }
}

struct Quotes<'a> {
    doc: &'a JavascriptDocument,
    nodes: &'a [Node],
    origin: usize,
}

impl CloseBracketSyntax for Quotes<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        let at = pos.checked_sub(self.origin)?;
        if !self.doc.text().is_char_boundary(at) {
            return None;
        }
        path(self.nodes, at, side).last().map(|node| node.id)
    }

    fn range(&self, node: usize) -> Range<usize> {
        let range = &self.nodes[node].range;
        self.origin + range.start..self.origin + range.end
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.nodes[node].parent
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.nodes[node].children.first().copied()
    }
}
