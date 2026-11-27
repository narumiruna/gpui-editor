use std::ops::Range;

use crate::{Assoc, autocomplete::brackets::CloseBracketSyntax};

use super::{Side, XmlDocument};

impl XmlDocument {
    /// Adapt the XML syntax tree to quote input at the host byte offset `origin`.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        Quotes { doc: self, origin }
    }
}

struct Quotes<'a> {
    doc: &'a XmlDocument,
    origin: usize,
}

impl CloseBracketSyntax for Quotes<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        let at = pos.checked_sub(self.origin)?;
        if !self.doc.text().is_char_boundary(at) {
            return None;
        }
        let side = match side {
            Some(Assoc::Before) => Side::Left,
            Some(Assoc::After) => Side::Right,
            None => Side::Strict,
        };
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
