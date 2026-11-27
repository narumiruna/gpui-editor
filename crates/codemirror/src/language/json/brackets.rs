use crate::Assoc;

use super::{
    super::{
        brackets::{
            BracketConfig, BracketDirection, BracketDirection::Forward, BracketMatch,
            bracket_pairs, directional_bracket_near, editor_bracket_index,
            match_configured_editor_text, match_editor_siblings, match_editor_text,
            match_named_siblings, node_delimiter,
        },
        line_projection::splits_crlf,
    },
    JsonDocument, JsonNode, JsonNodeKind,
};

impl JsonDocument {
    /// Find a bracket next to this UTF-8 caret. Prefer the bracket on the
    /// left. Structural delimiters match siblings; other brackets can match
    /// only characters in the same syntax type.
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

    /// Find a bracket next to this UTF-8 caret using explicit matching options.
    /// Grammar delimiters take precedence over configured text pairs. The
    /// distance limit applies only to plain scans and counts logical UTF-16 units.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        directional_bracket_near(
            self.text().as_str(),
            head,
            config.after_cursor,
            |pos, dir| self.match_bracket_at(pos, dir, config),
        )
    }

    /// Match from this UTF-8 byte position and direction. `after_cursor`
    /// applies only to caret queries. Positions inside CRLF return `None`.
    pub fn match_bracket_at(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let source = self.text().as_str();
        if splits_crlf(source, pos) {
            return None;
        }
        let side = if direction == Forward {
            Assoc::After
        } else {
            Assoc::Before
        };
        let start = self.node_at(pos, Some(side))?;
        let pairs = bracket_pairs(config);
        for node in std::iter::successors(Some(start), |node| node.parent()) {
            if !node.range().is_empty()
                && let Some(expected) = node_delimiter(node.node_type(), direction, pairs)
            {
                return Some(match_named_siblings(
                    node.range(),
                    direction,
                    expected,
                    node.parent().into_iter().flat_map(JsonNode::children),
                    |sibling| (sibling.range(), sibling.kind().name()),
                    |sibling, dir| node_delimiter(sibling.node_type(), dir, pairs).is_some(),
                ));
            }
        }
        match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
            !splits_crlf(source, at)
                && !splits_crlf(source, at + 1)
                && self
                    .node_at(at, Some(Assoc::After))
                    .is_some_and(|candidate| candidate.kind() == start.kind())
        })
        .filter(|hit| !splits_crlf(source, hit.start.start) && !splits_crlf(source, hit.start.end))
    }

    fn match_bracket(&self, at: usize, index: usize) -> Option<BracketMatch> {
        let forward = index.is_multiple_of(2);
        let (position, side) = if forward {
            (at, Assoc::After)
        } else {
            (at + 1, Assoc::Before)
        };
        let node = self.node_at(position, Some(side))?;
        if delimiter(node.kind()) == Some(index) {
            let siblings = node
                .parent()?
                .children()
                .filter_map(|sibling| Some((sibling.range(), delimiter(sibling.kind())?)));
            return Some(match_editor_siblings(node.range(), index, siblings));
        }
        match_editor_text(self.syntax_lines(), at..at + 1, index, |other| {
            self.node_at(other, Some(Assoc::After))
                .is_some_and(|candidate| candidate.kind() == node.kind())
        })
    }
}

// The pinned grammar's openedBy and closedBy properties.
fn delimiter(kind: JsonNodeKind) -> Option<usize> {
    match kind {
        JsonNodeKind::OpenBracket => Some(2),
        JsonNodeKind::CloseBracket => Some(3),
        JsonNodeKind::OpenBrace => Some(4),
        JsonNodeKind::CloseBrace => Some(5),
        _ => None,
    }
}
