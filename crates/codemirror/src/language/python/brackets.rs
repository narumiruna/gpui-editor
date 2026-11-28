use crate::Assoc;

use super::{
    super::brackets::{
        BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch, bracket_pairs,
        directional_bracket_near, editor_bracket_index, match_configured_editor_text,
        match_editor_siblings, match_editor_text, match_named_siblings, named_delimiter,
    },
    PythonDocument, PythonNode, parser,
};

impl PythonDocument {
    /// Match the bracket next to `head`, preferring the bracket on the left.
    /// Marked delimiters match siblings. Other brackets match only within the
    /// same syntax type and do not match inside unparsed text. Positions and
    /// ranges use original UTF-8 bytes.
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
    /// Grammar delimiters and configured node names match siblings. The
    /// UTF-16 scan limit applies only to plain text fallback.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let pairs = bracket_pairs(config);
        let lines = self.editor_lines.as_ref().unwrap_or(&self.text);
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
                let node = self.resolve_bracket_at(pos, side)?;
                for token in std::iter::successors(Some(node), |node| node.parent()) {
                    if !token.data().range.is_empty()
                        && let Some(expected) = matching_name(token.data().term, direction, pairs)
                    {
                        return Some(match_named_siblings(
                            token.range(),
                            direction,
                            expected,
                            token.parent().into_iter().flat_map(PythonNode::children),
                            |sibling| (sibling.range(), sibling.name()),
                            |sibling, dir| matching_name(sibling.data().term, dir, pairs).is_some(),
                        ));
                    }
                }
                match_configured_editor_text(lines, pos, direction, config, |at| {
                    self.resolve_bracket_at(at, Assoc::After)
                        .is_some_and(|candidate| candidate.data().term == node.data().term)
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
        let node = self.resolve_bracket_at(position, side)?;
        let term = node.data().term;
        if delimiter_index(term) == Some(index) {
            let siblings = node.parent()?.children().filter_map(|sibling| {
                Some((sibling.range(), delimiter_index(sibling.data().term)?))
            });
            return Some(match_editor_siblings(node.range(), index, siblings));
        }
        let lines = self.editor_lines.as_ref().unwrap_or(&self.text);
        match_editor_text(lines, at..at + 1, index, |other| {
            self.resolve_bracket_at(other, Assoc::After)
                .is_some_and(|candidate| candidate.data().term == term)
        })
    }
}

fn matching_name(term: u16, direction: BracketDirection, pairs: &str) -> Option<Option<&str>> {
    use BracketDirection::Backward;
    // The outer f-string replacement opener (term 3) has no `closedBy`.
    let mate = match (term, direction) {
        (23, Backward) => "(",
        (24, Forward) => ")",
        (54, Backward) => "[",
        (55, Forward) => "]",
        (58, Backward) => "{",
        (59, Forward) => "}",
        _ => return named_delimiter(parser::name(term), direction, pairs),
    };
    Some(Some(mate))
}

fn delimiter_index(term: u16) -> Option<usize> {
    // The six marked terms use the default bracket names. The unmarked
    // f-string replacement start is also named `{` and matches by name.
    let &[byte] = parser::name(term).as_bytes() else {
        return None;
    };
    editor_bracket_index(byte)
}
