use std::{borrow::Cow, ops::Range};

use super::{JavascriptDocument, JavascriptNode, Node, path};
use crate::{
    Assoc, Text,
    language::{
        brackets::{
            BracketConfig, BracketDirection,
            BracketDirection::{Backward, Forward},
            BracketMatch, bracket_pairs, editor_bracket_index, match_configured_editor_text,
            match_editor_text,
        },
        line_projection::{NormalizedInput, editor_lines, splits_crlf},
    },
};

#[derive(Debug)]
pub(super) struct Projection {
    nodes: Vec<Node>,
    lines: Text,
}

impl Projection {
    fn new(doc: &JavascriptDocument) -> Option<Box<Self>> {
        // CodeMirror parses LF line breaks. A raw CR can extend a line
        // comment or split a CRLF string escape. Keep the normal tree intact.
        let input = NormalizedInput::new(doc.text.as_str());
        let Cow::Owned(source) = &input.text else {
            return None;
        };
        let (mut nodes, _, _) =
            JavascriptDocument::parse_nodes(source, doc.options, doc.expression, None);
        for node in &mut nodes {
            node.range = input.original_byte(node.range.start)..input.original_byte(node.range.end);
        }
        Some(Box::new(Self {
            nodes,
            lines: editor_lines(doc.text.as_str()).unwrap_or_else(|| doc.text.clone()),
        }))
    }
}

impl JavascriptDocument {
    /// Match a grammar-marked token or a default `()[]{}` bracket next to
    /// `head`. Prefer the bracket before the caret. Ranges are original UTF-8
    /// byte offsets, including for interpolation and JSX punctuation tokens.
    pub fn bracket_near(&self, head: usize) -> Option<BracketMatch> {
        if !self.text.is_char_boundary(head) {
            return None;
        }
        let bytes = self.text.as_str().as_bytes();
        [head.checked_sub(1), Some(head)]
            .into_iter()
            .flatten()
            .find_map(|at| {
                // All supported handles consist of ASCII bytes and match in
                // one direction only. The four upstream probes therefore
                // reduce to checking the byte on the left, then the right.
                let byte = *bytes.get(at)?;
                if !b"()[]{}$<>/".contains(&byte) {
                    return None;
                }
                let (nodes, lines) = self.projected_bracket_syntax();
                match_bracket(nodes, lines, at, byte)
            })
    }

    /// Find a bracket next to this UTF-8 caret using explicit matching options.
    /// Grammar delimiters take precedence over configured text pairs. The
    /// distance limit applies only to plain scans and counts logical UTF-16 units.
    pub fn bracket_near_with_config(
        &self,
        head: usize,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let source = self.text.as_str();
        if !source.is_char_boundary(head) || splits_crlf(source, head) {
            return None;
        }
        // Treat CRLF as one logical step without changing the byte offsets.
        let (before, after) = source.split_at(head);
        let previous = || {
            Some(
                head - if before.ends_with("\r\n") {
                    2
                } else {
                    before.chars().next_back()?.len_utf8()
                },
            )
        };
        let next = || {
            Some(
                head + if after.starts_with("\r\n") {
                    2
                } else {
                    after.chars().next()?.len_utf8()
                },
            )
        };
        let (nodes, lines) = self.projected_bracket_syntax();
        let scan = |pos, direction| match_configured_bracket(nodes, lines, pos, direction, config);
        scan(head, Backward)
            .or_else(|| scan(previous()?, Forward))
            .or_else(|| {
                config
                    .after_cursor
                    .then(|| scan(head, Forward).or_else(|| scan(next()?, Backward)))?
            })
    }

    pub(in crate::language) fn match_configured_bracket(
        &self,
        pos: usize,
        direction: BracketDirection,
        config: BracketConfig<'_>,
    ) -> Option<BracketMatch> {
        let (nodes, lines) = self.projected_bracket_syntax();
        match_configured_bracket(nodes, lines, pos, direction, config)
    }

    pub(super) fn projected_bracket_syntax(&self) -> (&[Node], &Text) {
        self.bracket_projection
            .get_or_init(|| Projection::new(self))
            .as_deref()
            .map_or((&self.nodes, &self.text), |p| (&p.nodes, &p.lines))
    }
}

fn match_bracket(nodes: &[Node], lines: &Text, at: usize, byte: u8) -> Option<BracketMatch> {
    let resolve = |byte| {
        path(nodes, lines, byte, Some(Assoc::After))
            .last()
            .expect("JavaScript root")
    };
    let node = resolve(at);
    if let Some((partner, forward)) = delimiter(node) {
        return match_marked(node, partner, forward);
    }
    let index = editor_bracket_index(byte)?;
    match_editor_text(lines, at..at + 1, index, |other| {
        let other = resolve(other);
        // Term IDs distinguish same-named grammar nodes (such as Escape).
        // If the parser reached its recovery fallback, only the name is set.
        (other.data().term, other.kind()) == (node.data().term, node.kind())
    })
}

fn match_configured_bracket(
    nodes: &[Node],
    lines: &Text,
    pos: usize,
    direction: BracketDirection,
    config: BracketConfig<'_>,
) -> Option<BracketMatch> {
    let forward = direction == Forward;
    let side = if forward { Assoc::After } else { Assoc::Before };
    let resolve = |at, side| {
        path(nodes, lines, at, Some(side))
            .last()
            .expect("JavaScript root")
    };
    let start = resolve(pos, side);
    let pairs = bracket_pairs(config);
    for node in std::iter::successors(Some(start), |node| node.parent()) {
        let range = node.range();
        if (if forward {
            range.start <= pos && pos < range.end
        } else {
            range.start < pos && pos <= range.end
        }) && let Some(partner) = matching_name(node, forward, pairs)
        {
            return Some(match_marked_siblings(
                range,
                node.parent().into_iter().flat_map(JavascriptNode::children),
                partner,
                forward,
                |sibling, forward| matching_name(sibling, forward, pairs).is_some(),
            ));
        }
    }
    // Term IDs distinguish same-named types such as CompareOp. The kind
    // also distinguishes nodes without a term in fallback trees.
    let start_type = (start.data().term, start.kind());
    match_configured_editor_text(lines, pos, direction, config, |at| {
        let candidate = resolve(at, Assoc::After);
        (candidate.data().term, candidate.kind()) == start_type
    })
}

// The requested-side grammar prop wins over the one-character name fallback.
fn matching_name<'a>(
    node: JavascriptNode<'_>,
    forward: bool,
    pairs: &'a str,
) -> Option<Option<&'a str>> {
    let partner = match (node.data().term, forward) {
        (Some(23), false) => "<",
        (Some(24 | 169), true) => ">",
        (Some(38), false) => "InterpolationStart",
        (Some(40), true) => "InterpolationEnd",
        (Some(50), true) => "]",
        (Some(56), false) => "[",
        (Some(60), false) => "{",
        (Some(61), true) => "}",
        (Some(73), false) => "(",
        (Some(74), true) => ")",
        (Some(161), false) => "JSXStartCloseTag",
        (Some(166), true) => "JSXEndTag",
        _ => {
            let mut chars = node.kind().name().chars();
            let name = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            let index = pairs.chars().position(|ch| ch == name)?;
            if index.is_multiple_of(2) != forward {
                return None;
            }
            // An unpaired alternative still counts as a delimiter.
            return Some(
                pairs
                    .char_indices()
                    .nth(index ^ 1)
                    .map(|(at, ch)| &pairs[at..at + ch.len_utf8()]),
            );
        }
    };
    Some(Some(partner))
}

// Partner names and directions from the pinned grammar's openedBy/closedBy
// props, with the default ()[]{} name fallback used by CodeMirror.
fn delimiter(node: JavascriptNode<'_>) -> Option<(&'static str, bool)> {
    Some(match node.data().term {
        Some(23) => ("<", false),
        // Term 96 is also named `<`, but has no closedBy prop.
        Some(24 | 169) => (">", true),
        Some(38) => ("InterpolationStart", false),
        Some(40) => ("InterpolationEnd", true),
        Some(161) => ("JSXStartCloseTag", false),
        Some(166) => ("JSXEndTag", true),
        _ => match node.kind().name() {
            "(" => (")", true),
            ")" => ("(", false),
            "[" => ("]", true),
            "]" => ("[", false),
            "{" => ("}", true),
            "}" => ("{", false),
            _ => return None,
        },
    })
}

fn match_marked(node: JavascriptNode<'_>, partner: &str, forward: bool) -> Option<BracketMatch> {
    Some(match_marked_siblings(
        node.range(),
        node.parent()?.children(),
        Some(partner),
        forward,
        |sibling, forward| delimiter(sibling).is_some_and(|(_, other)| other == forward),
    ))
}

fn match_marked_siblings<'a>(
    start: Range<usize>,
    mut siblings: impl DoubleEndedIterator<Item = JavascriptNode<'a>>,
    partner: Option<&str>,
    forward: bool,
    is_delimiter: impl Fn(JavascriptNode<'a>, bool) -> bool,
) -> BracketMatch {
    let mut hit = BracketMatch {
        start,
        end: None,
        matched: false,
    };
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
        // The partner may be unmarked. Check its name before its direction.
        if depth == 0 && !range.is_empty() && Some(sibling.kind().name()) == partner {
            hit.end = Some(range);
            hit.matched = true;
            break;
        }
        if is_delimiter(sibling, forward) {
            depth += 1;
        } else if is_delimiter(sibling, !forward) {
            if depth == 0 {
                hit.end = (!range.is_empty()).then_some(range);
                break;
            }
            depth -= 1;
        }
    }
    hit
}
