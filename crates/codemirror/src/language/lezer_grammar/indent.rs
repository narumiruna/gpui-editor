use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    LezerDocument, LezerNode,
    LezerNodeKind::*,
    trim_js_space,
};

pub(super) fn indentation(
    doc: &LezerDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.syntax_lines(), position, settings, options)?;
    let strict = doc.path(position, false).last()?;
    let inner = enter_unfinished(strict, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        let Some(open) = node.children().next() else {
            continue;
        };
        let Some(closer) = open.kind().closed_by() else {
            continue;
        };
        let last = node.children().next_back()?;
        let after = cx.text_after_pos(position, Assoc::After)?;
        let leading = after.len() - trim_js_space(after).len();
        let closed = last.kind() == closer
            && last.data().range.start == position + leading
            && !(options.simulate_double_break && cx.simulated_break() == Some(position));
        if let Some(column_at) = aligned(node, open, &cx) {
            let at = if closed {
                open.data().range.start
            } else {
                column_at
            };
            return Some(cx.column(at, Assoc::After)?.max(0) as usize);
        }
        let base = base_indent(doc, node, &cx)?;
        return Some(if closed {
            base
        } else {
            base.saturating_add(cx.unit())
        });
    }
    Some(0)
}

fn enter_unfinished(mut node: LezerNode<'_>, position: usize) -> LezerNode<'_> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.document.nodes[id].range.start < position);
    let mut scan = index.checked_sub(1).map(|index| LezerNode {
        id: children[index],
        ..node
    });
    while let Some(candidate) = scan {
        let mut children = candidate.children();
        let Some(last) = children
            .next_back()
            .filter(|last| last.data().range.end == candidate.data().range.end)
        else {
            break;
        };
        if last.kind() == Error && last.data().range.is_empty() {
            node = candidate;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn aligned(node: LezerNode<'_>, open: LezerNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let range = &open.data().range;
    let line = cx.text().line_at(range.start)?;
    let line_end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    let mut interior = node.children();
    interior.next();
    interior.next_back();
    let next = interior.find(|next| {
        let range_end = next.data().range.end;
        range_end > range.end && !matches!(next.kind(), LineComment | BlockComment | Error)
    });
    if next.is_some_and(|next| next.data().range.start < line_end) {
        let spaces = cx
            .text()
            .slice(range.end..line.range.end)?
            .bytes()
            .take_while(|&byte| byte == b' ')
            .count();
        Some(range.end + spaces)
    } else {
        None
    }
}

fn base_indent(doc: &LezerDocument, node: LezerNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let start = node.data().range.start;
    let mut line = cx.text().line_at(start)?.range.start;
    // If another node crosses this line's start, use its opening line.
    loop {
        let mut at_break = doc.path(line, false).last()?;
        while let Some(parent) = at_break
            .parent()
            .filter(|parent| parent.data().range.start == at_break.data().range.start)
        {
            at_break = parent;
        }
        if at_break.data().range.end > start || at_break.data().range.start >= line {
            return cx.line_indent(line, Assoc::After);
        }
        line = cx.text().line_at(at_break.data().range.start)?.range.start;
    }
}
