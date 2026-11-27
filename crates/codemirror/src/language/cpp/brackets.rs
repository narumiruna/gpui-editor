use std::ops::Range;

use crate::{
    Assoc,
    autocomplete::brackets::CloseBracketSyntax,
    language::brackets::{
        BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch, bracket_pairs,
        directional_bracket_near, match_configured_editor_text, match_named_siblings,
        named_delimiter,
    },
};

use super::{CppDocument, CppNode, CppNodeKind};

impl CppDocument {
    /// Match C or C++ delimiters near a UTF-8 caret. Marked delimiters use
    /// syntax siblings; plain scans compare exact grammar types and meter
    /// the distance in logical UTF-16 chunks.
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
            for token in std::iter::successors(Some(node), |node| node.parent()) {
                if !token.data().range.is_empty()
                    && let Some(expected) = matching_name(token.kind(), dir, pairs)
                {
                    return Some(match_named_siblings(
                        token.range(),
                        dir,
                        expected,
                        token.parent().into_iter().flat_map(CppNode::children),
                        |sibling| (sibling.range(), sibling.kind().name()),
                        |sibling, dir| matching_name(sibling.kind(), dir, pairs).is_some(),
                    ));
                }
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

fn matching_name(
    kind: CppNodeKind,
    direction: BracketDirection,
    pairs: &str,
) -> Option<Option<&str>> {
    use CppNodeKind::*;
    let mate = match (kind, direction == Forward) {
        (OpenParen, true) => CloseParen,
        (CloseParen, false) => OpenParen,
        (OpenBrace, true) => CloseBrace,
        (CloseBrace, false) => OpenBrace,
        (OpenBracket, true) => CloseBracket,
        (CloseBracket, false) => OpenBracket,
        _ => return named_delimiter(kind.name(), direction, pairs),
    };
    Some(Some(mate.name()))
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
