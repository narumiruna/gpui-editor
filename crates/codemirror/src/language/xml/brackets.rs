use super::{
    super::{
        brackets::{
            BracketConfig, BracketDirection,
            BracketDirection::{Backward, Forward},
            BracketMatch, bracket_pairs, directional_bracket_near, editor_bracket_index,
            match_configured_editor_text, match_editor_text, named_delimiter,
        },
        line_projection::editor_lines,
    },
    Side, XmlDocument, XmlNode,
    XmlNodeKind::{self, *},
};

impl XmlDocument {
    /// Match XML syntax at a collapsed caret with default editor settings.
    /// The caret and result ranges use UTF-8 bytes in the original text.
    /// Return `None` if the caret is invalid or splits a CRLF line break.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        directional_bracket_near(self.text.as_str(), head, true, |pos, direction| {
            self.match_bracket(pos, direction, None)
        })
    }

    /// Match XML at a UTF-8 caret using explicit matching options. Tag names
    /// and punctuation use the grammar; only plain scans use the UTF-16 limit.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        directional_bracket_near(
            self.text.as_str(),
            head,
            config.after_cursor,
            |pos, direction| self.match_bracket(pos, direction, Some(config)),
        )
    }

    fn match_bracket(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: Option<BracketConfig<'_>>,
    ) -> Option<BracketMatch> {
        let forward = direction == Forward;
        let side = if forward { Side::Right } else { Side::Left };
        let node = self.path(pos, side).last()?;
        let pairs = config.map(bracket_pairs);
        for token in std::iter::successors(Some(node), |node| node.parent()) {
            let Some(mates) = matching(token.kind(), direction, pairs) else {
                continue;
            };
            if token.data().range.is_empty() {
                continue;
            }
            let Some(handle) = handle(token).filter(|node| {
                let range = &node.data().range;
                if forward {
                    range.start <= pos && pos < range.end
                } else {
                    range.start < pos && pos <= range.end
                }
            }) else {
                continue;
            };
            return Some(match_marked(token, handle, mates, direction, pairs));
        }

        let same_syntax = |candidate| {
            self.path(candidate, Side::Right)
                .last()
                .is_some_and(|candidate| candidate.kind() == node.kind())
        };
        if let Some(config) = config {
            return match_configured_editor_text(
                self.bracket_lines(),
                pos,
                direction,
                config,
                same_syntax,
            );
        }
        let at = if forward { pos } else { pos.checked_sub(1)? };
        let index = editor_bracket_index(*self.text.as_str().as_bytes().get(at)?)?;
        if index.is_multiple_of(2) != forward {
            return None;
        }
        match_editor_text(self.bracket_lines(), at..at + 1, index, same_syntax)
    }

    fn bracket_lines(&self) -> &crate::Text {
        self.bracket_lines
            .get_or_init(|| editor_lines(self.text.as_str()))
            .as_ref()
            .unwrap_or(&self.text)
    }
}

// The XML grammar uses lists of target kinds, not fixed pairs. In particular,
// `</` is a target for a backward scan from `>`, but cannot start a scan.
fn matching_kinds(kind: XmlNodeKind, forward: bool) -> &'static [XmlNodeKind] {
    match (forward, kind) {
        (true, OpenTag) => &[CloseTag, MissingCloseTag],
        (true, StartTag) => &[SelfCloseEndTag, EndTag],
        (false, CloseTag) => &[OpenTag],
        (false, EndTag) => &[StartTag, StartCloseTag],
        (false, SelfCloseEndTag) => &[StartTag],
        _ => &[],
    }
}

enum Mates<'a> {
    Grammar(&'static [XmlNodeKind]),
    Named(Option<&'a str>),
}

impl Mates<'_> {
    fn contains(&self, kind: XmlNodeKind) -> bool {
        match self {
            Self::Grammar(kinds) => kinds.contains(&kind),
            Self::Named(name) => *name == Some(kind.name()),
        }
    }
}

fn matching(
    kind: XmlNodeKind,
    direction: BracketDirection,
    pairs: Option<&str>,
) -> Option<Mates<'_>> {
    let kinds = matching_kinds(kind, direction == Forward);
    if !kinds.is_empty() {
        return Some(Mates::Grammar(kinds));
    }
    named_delimiter(kind.name(), direction, pairs?).map(Mates::Named)
}

fn handle(node: XmlNode<'_>) -> Option<XmlNode<'_>> {
    match node.kind() {
        OpenTag | CloseTag => node.bracket_matching_handle(),
        _ => Some(node),
    }
}

fn match_marked(
    token: XmlNode<'_>,
    start: XmlNode<'_>,
    mates: Mates<'_>,
    direction: BracketDirection,
    pairs: Option<&str>,
) -> BracketMatch {
    let forward = direction == Forward;
    let opposite = if forward { Backward } else { Forward };
    let mut hit = BracketMatch {
        start: start.range(),
        end: None,
        matched: false,
    };
    let Some(parent) = token.parent() else {
        return hit;
    };
    let range = &token.data().range;
    let children = &parent.data().children;
    let mut siblings = if forward {
        let first = children.partition_point(|&id| token.nodes[id].range.end <= range.end);
        children[first..].iter()
    } else {
        let end = children.partition_point(|&id| token.nodes[id].range.start < range.start);
        children[..end].iter()
    };
    let mut depth = 0;
    while let Some(&id) = if forward {
        siblings.next()
    } else {
        siblings.next_back()
    } {
        let sibling = XmlNode { id, ..token };
        let other = &sibling.data().range;
        if (forward && other.start < range.end) || (!forward && other.end > range.start) {
            continue;
        }
        let kind = sibling.kind();
        if depth == 0 && mates.contains(kind) && !other.is_empty() {
            hit.end = handle(sibling).map(XmlNode::range);
            hit.matched = true;
            break;
        }
        if matching(kind, direction, pairs).is_some() {
            depth += 1;
        } else if matching(kind, opposite, pairs).is_some() {
            if depth == 0 {
                hit.end = handle(sibling)
                    .map(XmlNode::range)
                    .filter(|r| !r.is_empty());
                break;
            }
            depth -= 1;
        }
    }
    hit
}
