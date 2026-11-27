use std::ops::Range;

use super::{JavaDocument, JavaNode};
use crate::{
    Assoc,
    autocomplete::brackets::CloseBracketSyntax,
    language::brackets::{
        BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch, bracket_pairs,
        directional_bracket_near, editor_bracket_index, match_configured_editor_text,
        match_editor_siblings, match_editor_text, match_named_siblings, named_delimiter,
    },
};

impl JavaDocument {
    /// Match a delimiter or a bracket in a comment or literal near a caret.
    /// Positions and results use original UTF-8 bytes. Prefer the left bracket.
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
    /// Grammar delimiters and configured node names match siblings. The
    /// UTF-16 scan limit applies only to plain text fallback.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let pairs = bracket_pairs(config);
        directional_bracket_near(
            self.text().as_str(),
            head,
            config.after_cursor,
            |pos, direction| {
                let side = if direction == Forward {
                    Assoc::After
                } else {
                    Assoc::Before
                };
                let node = self.resolve(pos, Some(side));
                for token in std::iter::successors(Some(node), |node| node.parent()) {
                    if !token.data().range.is_empty()
                        && let Some(expected) = matching_name(token, direction, pairs)
                    {
                        return Some(match_named_siblings(
                            token.range(),
                            direction,
                            expected,
                            token.parent().into_iter().flat_map(JavaNode::children),
                            |sibling| (sibling.range(), sibling.name()),
                            |sibling, dir| matching_name(sibling, dir, pairs).is_some(),
                        ));
                    }
                }
                match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
                    self.resolve(at, Some(Assoc::After)).term() == node.term()
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
        let node = self.resolve(position, Some(side));
        if delimiter(node) == Some(index) {
            let siblings = node
                .parent()?
                .children()
                .filter_map(|sibling| Some((sibling.range(), delimiter(sibling)?)));
            return Some(match_editor_siblings(node.range(), index, siblings));
        }
        match_editor_text(self.syntax_lines(), at..at + 1, index, |other| {
            self.resolve(other, Some(Assoc::After)).term() == node.term()
        })
    }

    /// Adapt the parsed nodes to quote input in a host document. `origin`
    /// is the byte offset of the Java body in that document.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        Quotes { doc: self, origin }
    }
}

struct Quotes<'a> {
    doc: &'a JavaDocument,
    origin: usize,
}

impl CloseBracketSyntax for Quotes<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        let position = pos.checked_sub(self.origin)?;
        self.doc
            .text()
            .is_char_boundary(position)
            .then(|| self.doc.resolve(position, side).id)
    }

    fn range(&self, node: usize) -> Range<usize> {
        let range = &self.doc.editor_nodes()[node].range;
        self.origin + range.start..self.origin + range.end
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.doc.editor_nodes()[node].parent
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.doc.editor_nodes()[node].children.first().copied()
    }
}

fn matching_name<'a>(
    node: JavaNode<'_>,
    direction: BracketDirection,
    pairs: &'a str,
) -> Option<Option<&'a str>> {
    let expected = if direction == Forward {
        node.closed_by()
    } else {
        node.opened_by()
    };
    // Only parentheses and braces have props. Array bracket nodes use names.
    expected.map_or_else(
        || named_delimiter(node.name(), direction, pairs),
        |name| Some(Some(name)),
    )
}

fn delimiter(node: JavaNode<'_>) -> Option<usize> {
    let &[byte] = node.name().as_bytes() else {
        return None;
    };
    editor_bracket_index(byte)
}
