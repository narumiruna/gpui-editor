use std::ops::Range;

use crate::{
    Assoc,
    autocomplete::brackets::CloseBracketSyntax,
    language::{
        brackets::{
            BracketConfig, BracketDirection::Forward, BracketMatch, bracket_pairs,
            directional_bracket_near, match_configured_editor_text,
        },
        lezer::{LrSyntaxNode, match_lr_marked},
    },
};

use super::CppDocument;

impl CppDocument {
    /// Match C or C++ delimiters near a UTF-8 caret. Marked delimiters use
    /// configured roles and handles on syntax siblings. Plain scans compare
    /// exact grammar terms and measure distance in logical UTF-16 chunks.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let pairs = bracket_pairs(config);
        let lines = self.syntax_lines();
        directional_bracket_near(self.text.as_str(), head, config.after_cursor, |pos, dir| {
            let side = if dir == Forward {
                Assoc::After
            } else {
                Assoc::Before
            };
            let node = self.path(pos, Some(side)).last()?;
            if let Some(hit) = match_lr_marked(&self.text, LrSyntaxNode::cpp(node), pos, dir, pairs)
            {
                return Some(hit);
            }
            let term = node.data().term;
            match_configured_editor_text(lines, pos, dir, config, |at| {
                self.path(at, Some(Assoc::After))
                    .last()
                    .is_some_and(|candidate| candidate.data().term == term)
            })
        })
    }

    /// Adapt the tree to quote input at the host byte offset `origin`.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        Quotes { doc: self, origin }
    }
}

struct Quotes<'a> {
    doc: &'a CppDocument,
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
