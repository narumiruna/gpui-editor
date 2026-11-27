use std::ops::Range;

use crate::{
    Assoc, ChangeSet,
    language::line_projection::{keeps_separate_breaks, splits_crlf},
};

use super::{MarkdownDialect, MarkdownNode as Node, MarkdownNodeKind::*, parse_root};

pub(super) fn changes_line_seam(source: &str, changes: &ChangeSet) -> bool {
    // A change can alter the projection of a CR outside its range. A reused
    // block boundary can then fall inside a CRLF pair.
    changes
        .changes()
        .any(|(range, _)| splits_crlf(source, range.start) || splits_crlf(source, range.end))
        || !keeps_separate_breaks(source, changes)
}

pub(super) fn reparse(
    old: &Node,
    old_source: &str,
    source: &str,
    changes: &ChangeSet,
    changed: Range<usize>,
    dialect: MarkdownDialect,
) -> (Node, usize, usize) {
    // The block parser stores link definitions in a document-wide map. A
    // possible definition anywhere can affect which spans it reports.
    if old_source.contains("]:") || source.contains("]:") {
        return (parse_root(source, dialect), source.len(), 0);
    }
    let first = old
        .children
        .partition_point(|node| node.range.end <= changed.start);
    let (prefix, from) = old.children[..first]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, node)| {
            boundary(old_source, node)
                .filter(|&end| end <= changed.start)
                .map(|end| (i + 1, end))
        })
        .unwrap_or((0, 0));
    let last = old
        .children
        .partition_point(|node| node.range.start < changed.end)
        .max(prefix);
    let anchor = old.children[last..]
        .iter()
        .enumerate()
        .find_map(|(i, node)| {
            let i = i + last;
            (i + 1 < old.children.len())
                .then(|| boundary(old_source, node))
                .flatten()
                .map(|end| (i, end))
        });

    let mut to = source.len();
    let mut suffix = old.children.len();
    let mut parsed_bytes = 0;
    let middle = if let Some((index, old_end)) = anchor {
        let new_end = changes.map_pos(old_end, Assoc::After);
        let new_start = changes.map_pos(old.children[index].range.start, Assoc::After);
        let node = &old.children[index];
        parsed_bytes += new_end - from;
        let middle = parse_root(&source[from..new_end], dialect);
        // A new open fence, HTML block, or container may swallow the old
        // anchor. Reuse the suffix only if the root-level boundary survives.
        if middle.children.last().is_some_and(|last| {
            last.kind == node.kind
                && last.range == (new_start - from..new_start - from + node.range.len())
        }) {
            to = new_end;
            suffix = index + 1;
            middle
        } else {
            parsed_bytes += source.len() - from;
            parse_root(&source[from..], dialect)
        }
    } else {
        parsed_bytes += source.len() - from;
        parse_root(&source[from..], dialect)
    };
    let reused = prefix + old.children.len() - suffix;
    let mut children = old.children[..prefix].to_vec();
    children.extend(middle.children.into_iter().map(|mut child| {
        child.shift(from as isize);
        child
    }));
    let shift = source.len() as isize - old_source.len() as isize;
    debug_assert!(
        old.children
            .get(suffix)
            .is_none_or(|node| node.range.start.wrapping_add_signed(shift) >= to)
    );
    children.extend(old.children[suffix..].iter().cloned().map(|mut child| {
        child.shift(shift);
        child
    }));
    (
        Node::new(Document, 0..source.len(), children),
        parsed_bytes,
        reused,
    )
}

fn boundary(source: &str, node: &Node) -> Option<usize> {
    // Only cut after blocks that cannot resume across this separator. A blank
    // line alone is not enough after lists, indented code, or open fences.
    let mut end = after_line_break(source, node.range.end)?;
    if matches!(
        node.kind,
        ATXHeading1 | ATXHeading2 | ATXHeading3 | ATXHeading4 | ATXHeading5 | ATXHeading6
    ) {
        return Some(end);
    }
    if !matches!(
        node.kind,
        Paragraph | SetextHeading1 | SetextHeading2 | HorizontalRule | Table
    ) {
        return None;
    }
    end += source.as_bytes()[end..]
        .iter()
        .take_while(|&&byte| matches!(byte, b' ' | b'\t'))
        .count();
    after_line_break(source, end)
}

fn after_line_break(source: &str, at: usize) -> Option<usize> {
    match source.as_bytes().get(at..) {
        Some([b'\r', b'\n', ..]) => Some(at + 2),
        Some([b'\n', ..]) => Some(at + 1),
        _ => None,
    }
}
