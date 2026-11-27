use crate::Assoc;

use super::{
    super::brackets::{
        BracketConfig, BracketDirection,
        BracketDirection::{Backward, Forward},
        BracketMatch, bracket_pairs, editor_bracket_index, match_configured_editor_text,
        match_editor_siblings, match_editor_text,
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
        let text = self.text();
        let prev = text.slice(0..head)?.chars().next_back();
        let next = text.slice(head..text.len())?.chars().next();
        let scan = |pos, dir| self.match_configured_bracket(pos, dir, config);
        scan(head, Backward)
            .or_else(|| scan(head - prev?.len_utf8(), Forward))
            .or_else(|| {
                config.after_cursor.then(|| {
                    scan(head, Forward).or_else(|| scan(head + next?.len_utf8(), Backward))
                })?
            })
    }

    fn match_configured_bracket(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let side = match direction {
            Forward => Assoc::After,
            Backward => Assoc::Before,
        };
        let start = self.node_at(pos, Some(side))?;
        let pairs = bracket_pairs(config);
        for node in std::iter::successors(Some(start), |node| node.parent()) {
            if !node.range().is_empty()
                && let Some(expected) = matching_name(node.kind(), direction, pairs)
            {
                return Some(match_configured_siblings(node, direction, expected, pairs));
            }
        }
        match_configured_editor_text(self.syntax_lines(), pos, direction, config, |at| {
            self.node_at(at, Some(Assoc::After))
                .is_some_and(|candidate| candidate.kind() == start.kind())
        })
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

fn match_configured_siblings(
    node: JsonNode<'_>,
    direction: BracketDirection,
    expected: Option<char>,
    pairs: &str,
) -> BracketMatch {
    let mut hit = BracketMatch {
        start: node.range(),
        end: None,
        matched: false,
    };
    let forward = direction == Forward;
    let opposite = if forward { Backward } else { Forward };
    let mut siblings = node.parent().into_iter().flat_map(JsonNode::children);
    let mut depth = 0;
    while let Some(sibling) = if forward {
        siblings.next()
    } else {
        siblings.next_back()
    } {
        let range = sibling.range();
        if (forward && (range.start < hit.start.end || range.end <= hit.start.end))
            || (!forward && (range.end > hit.start.start || range.start >= hit.start.start))
        {
            continue;
        }
        let kind = sibling.kind();
        if depth == 0
            && expected.is_some_and(|name| node_name_char(kind) == Some(name))
            && !range.is_empty()
        {
            hit.end = Some(range);
            hit.matched = true;
            break;
        }
        if matching_name(kind, direction, pairs).is_some() {
            depth += 1;
        } else if matching_name(kind, opposite, pairs).is_some() {
            if depth == 0 {
                hit.end = (!range.is_empty()).then_some(range);
                break;
            }
            depth -= 1;
        }
    }
    hit
}

// Prefer grammar properties. A single-character node name can also act as
// a marked bracket. An unpaired alternative has no expected name.
fn matching_name(
    kind: JsonNodeKind,
    direction: BracketDirection,
    pairs: &str,
) -> Option<Option<char>> {
    use JsonNodeKind::{CloseBrace, CloseBracket, OpenBrace, OpenBracket};
    match (kind, direction) {
        (OpenBracket, Forward) => return Some(Some(']')),
        (CloseBracket, Backward) => return Some(Some('[')),
        (OpenBrace, Forward) => return Some(Some('}')),
        (CloseBrace, Backward) => return Some(Some('{')),
        _ => {}
    }
    let name = node_name_char(kind)?;
    let index = pairs.chars().position(|candidate| candidate == name)?;
    (index.is_multiple_of(2) == (direction == Forward)).then(|| pairs.chars().nth(index ^ 1))
}

fn node_name_char(kind: JsonNodeKind) -> Option<char> {
    let mut chars = kind.name().chars();
    let name = chars.next()?;
    chars.next().is_none().then_some(name)
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
