use std::ops::Range;

use crate::Text;

use super::{
    super::{
        brackets::{
            BracketConfig, BracketDirection, BracketMatch, directional_bracket_near,
            editor_bracket_index, match_configured_editor_text, match_configured_editor_text_in,
            match_editor_text,
        },
        line_projection::editor_lines,
    },
    HtmlDocument, HtmlEmbeddedDocument, HtmlMount, HtmlNode,
    HtmlNodeKind::{self, *},
    lexer,
};

impl HtmlDocument {
    /// Match HTML host syntax next to a caret. The caret and result ranges
    /// use UTF-8 bytes in the original source. Mounted languages are not
    /// matched here. Return `None` for an invalid caret.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        let source = self.text.as_str();
        let before = source.get(..head)?;
        let after = &source[head..];
        let prev = || {
            if before.ends_with("\r\n") {
                Some(head - 2)
            } else {
                before.chars().next_back().map(|ch| head - ch.len_utf8())
            }
        };
        let next = || {
            if after.starts_with("\r\n") {
                Some(head + 2)
            } else {
                after.chars().next().map(|ch| head + ch.len_utf8())
            }
        };
        self.match_bracket(head, false)
            .or_else(|| self.match_bracket(prev()?, true))
            .or_else(|| self.match_bracket(head, true))
            .or_else(|| self.match_bracket(next()?, false))
    }

    /// Match configured brackets in HTML or its mounts at an original UTF-8 caret.
    /// Each probe checks only the language on its side of a mount boundary.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        directional_bracket_near(
            self.text.as_str(),
            head,
            config.after_cursor,
            |pos, direction| self.match_configured_at(pos, direction, config),
        )
    }

    fn match_configured_at(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let forward = direction == BracketDirection::Forward;
        if let Some(mount) = self.bracket_mount_at(pos, forward) {
            let range = mount.range();
            let offset = range.start;
            let local = pos - offset;
            let hit = match mount.document() {
                HtmlEmbeddedDocument::Javascript(js) => {
                    js.match_configured_bracket(local, direction, config)
                }
                HtmlEmbeddedDocument::Css(css) => {
                    css.match_configured_bracket(local, direction, config)
                }
                HtmlEmbeddedDocument::Custom(_) => {
                    return match_configured_editor_text_in(
                        self.bracket_lines(),
                        range,
                        pos,
                        direction,
                        config,
                        |_| true,
                    );
                }
            }?;
            let shift = |range: Range<usize>| range.start + offset..range.end + offset;
            return Some(BracketMatch {
                start: shift(hit.start),
                end: hit.end.map(shift),
                matched: hit.matched,
            });
        }
        let node = self.bracket_node_at(pos, forward);
        if let Some(mut hit) = self.match_host_marked(node, pos, forward) {
            // A custom body overlay can cover a closing tag in the host tree.
            if hit
                .end
                .as_ref()
                .is_some_and(|end| self.bracket_in_mount(end.start, true))
            {
                hit.end = None;
                hit.matched = false;
            }
            return Some(hit);
        }
        match_configured_editor_text(self.bracket_lines(), pos, direction, config, |at| {
            !self.bracket_in_mount(at, true) && self.bracket_node_at(at, true).kind() == node.kind()
        })
    }

    fn match_bracket(&self, pos: usize, forward: bool) -> Option<BracketMatch> {
        if self.bracket_in_mount(pos, forward) {
            return None;
        }
        let node = self.bracket_node_at(pos, forward);
        if let Some(hit) = self.match_host_marked(node, pos, forward) {
            return Some(hit);
        }

        let at = if forward { pos } else { pos.checked_sub(1)? };
        let index = editor_bracket_index(*self.text.as_str().as_bytes().get(at)?)?;
        if index.is_multiple_of(2) != forward {
            return None;
        }
        match_editor_text(self.bracket_lines(), at..at + 1, index, |candidate| {
            !self.bracket_in_mount(candidate, true)
                && self.bracket_node_at(candidate, true).kind() == node.kind()
        })
    }

    fn match_host_marked(
        &self,
        node: HtmlNode<'_>,
        pos: usize,
        forward: bool,
    ) -> Option<BracketMatch> {
        for token in std::iter::successors(Some(node), |node| node.parent()) {
            let mates = self.bracket_mates(token, forward);
            if mates.is_empty() || token.data().range.is_empty() {
                continue;
            }
            let Some(handle) = handle(token).filter(|&node| touches(node, pos, forward)) else {
                continue;
            };
            return Some(self.match_marked(token, handle, mates, forward));
        }
        None
    }

    fn bracket_lines(&self) -> &Text {
        self.bracket_lines
            .get_or_init(|| editor_lines(self.text.as_str()))
            .as_ref()
            .unwrap_or(&self.text)
    }

    fn bracket_in_mount(&self, pos: usize, forward: bool) -> bool {
        self.bracket_mount_at(pos, forward).is_some()
    }

    fn bracket_mount_at(&self, pos: usize, forward: bool) -> Option<&HtmlMount> {
        let at = self.mounts.partition_point(|mount| {
            let end = mount.range().end;
            end < pos || (forward && end == pos)
        });
        self.mounts.get(at).filter(|mount| {
            let start = mount.range().start;
            start < pos || (forward && start == pos)
        })
    }

    fn bracket_node_at(&self, pos: usize, forward: bool) -> HtmlNode<'_> {
        let mut node = self.root();
        loop {
            let children = &node.data().children;
            let index = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < pos || (forward && end == pos)
            });
            let Some(&id) = children.get(index) else {
                break;
            };
            let child = HtmlNode { id, ..node };
            if !touches(child, pos, forward) {
                break;
            }
            node = child;
        }
        node
    }

    fn match_marked(
        &self,
        token: HtmlNode<'_>,
        handle_node: HtmlNode<'_>,
        mates: &[HtmlNodeKind],
        forward: bool,
    ) -> BracketMatch {
        let mut hit = BracketMatch {
            start: handle_node.range(),
            end: None,
            matched: false,
        };
        let Some(parent) = token.parent() else {
            return hit;
        };
        let range = &token.data().range;
        let children = &parent.data().children;
        let mut siblings = if forward {
            let index = children.partition_point(|&id| self.nodes[id].range.end <= range.end);
            children[index..].iter()
        } else {
            let index = children.partition_point(|&id| self.nodes[id].range.start < range.start);
            children[..index].iter()
        };
        let mut depth = 0;
        while let Some(&id) = if forward {
            siblings.next()
        } else {
            siblings.next_back()
        } {
            let sibling = HtmlNode { id, ..token };
            let other = &sibling.data().range;
            if (forward && other.start < range.end) || (!forward && other.end > range.start) {
                continue;
            }
            if depth == 0 && mates.contains(&sibling.kind()) && !other.is_empty() {
                hit.matched = true;
                hit.end = handle(sibling).map(HtmlNode::range);
                break;
            }
            if !self.bracket_mates(sibling, forward).is_empty() {
                depth += 1;
            } else if !self.bracket_mates(sibling, !forward).is_empty() {
                if depth == 0 {
                    hit.end = handle(sibling)
                        .map(HtmlNode::range)
                        .filter(|r| !r.is_empty());
                    break;
                }
                depth -= 1;
            }
        }
        hit
    }

    fn bracket_mates(&self, token: HtmlNode<'_>, forward: bool) -> &'static [HtmlNodeKind] {
        // The pinned grammar's closedBy and openedBy properties. Several
        // LR terms share a visible kind, but not the same properties.
        match (forward, token.kind()) {
            (true, OpenTag) => &[CloseTag],
            (true, StartTag) if self.start_tag_accepts_self_closing_end(token) => {
                &[EndTag, SelfClosingEndTag]
            }
            (true, StartTag | StartCloseTag) => &[EndTag],
            (false, CloseTag) if self.can_start_close_match(token) => &[OpenTag],
            (false, EndTag) => &[StartTag, StartCloseTag],
            (false, SelfClosingEndTag) => &[StartTag],
            _ => &[],
        }
    }

    fn start_tag_accepts_self_closing_end(&self, token: HtmlNode<'_>) -> bool {
        // Only the ordinary StartTag term lists SelfClosingEndTag. Void
        // and raw-text opener terms can still be its backward target.
        let name = lexer::context_name(&self.text.as_str()[token.range().end..]);
        !lexer::is_void(&name) && lexer::raw_kind(&name).is_none()
    }

    fn can_start_close_match(&self, close: HtmlNode<'_>) -> bool {
        // The noMatch close term is a forward target by visible name, but
        // has no openedBy property. Context names follow the HTML tokenizer.
        if self.options.match_closing_tags {
            return true;
        }
        let open = close.parent().and_then(|parent| parent.child(OpenTag));
        let Some(start) = open.and_then(|tag| tag.child(StartTag)) else {
            return false;
        };
        let Some(end) = close.child(StartCloseTag) else {
            return false;
        };
        let source = self.text.as_str();
        lexer::context_name(&source[start.range().end..])
            == lexer::context_name(&source[end.range().end..])
    }
}

fn handle(node: HtmlNode<'_>) -> Option<HtmlNode<'_>> {
    match node.kind() {
        OpenTag | CloseTag => node.bracket_matching_handle(),
        _ => Some(node),
    }
}

fn touches(node: HtmlNode<'_>, pos: usize, forward: bool) -> bool {
    let range = &node.data().range;
    if forward {
        range.start <= pos && pos < range.end
    } else {
        range.start < pos && pos <= range.end
    }
}
