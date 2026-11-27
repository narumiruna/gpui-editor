use crate::{
    Assoc,
    language::{IndentContext, IndentOptions, StreamSettings},
};

use super::{LessDocument, LessNode, LessNodeKind::*, Side, trim_js_space};

pub(super) fn indentation(
    doc: &LessDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let inner = enter_unfinished(doc.path(position, Side::Inner).last()?, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        if node.kind() == Declaration {
            return Some(base_indent(doc, node, &cx)?.saturating_add(cx.unit()));
        }
        let mut children = node.children();
        let Some(open) = children.next() else {
            continue;
        };
        let Some(closer) = open.data().closing else {
            continue;
        };
        let Some(last) = children.next_back() else {
            continue;
        };
        let after = cx.text_after_pos(position, Assoc::After)?;
        let leading = after.len() - trim_js_space(after).len();
        let closed = last.kind() == closer
            && last.data().range.start == position + leading
            && !(options.simulate_double_break && cx.simulated_break() == Some(position));
        if let Some(aligned) = alignment(open, children, &cx) {
            let at = if closed {
                open.data().range.start
            } else {
                aligned
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

// Continue into a repaired node at its end when a last child is missing.
fn enter_unfinished(mut node: LessNode<'_>, position: usize) -> LessNode<'_> {
    let children = &node.data().children;
    let i = children.partition_point(|&id| node.nodes[id].range.start < position);
    let mut scan = i.checked_sub(1).map(|i| LessNode {
        id: children[i],
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

fn alignment<'a>(
    open: LessNode<'a>,
    mut interior: impl Iterator<Item = LessNode<'a>>,
    cx: &IndentContext<'_>,
) -> Option<usize> {
    let range = &open.data().range;
    let line = cx.text().line_at(range.start)?;
    let end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    let next = interior.find(|next| {
        next.data().range.end > range.end && !matches!(next.kind(), Comment | LineComment | Error)
    })?;
    if next.data().range.start >= end {
        return None;
    }
    let spaces = cx
        .text()
        .slice(range.end..line.range.end)?
        .bytes()
        .take_while(|&byte| byte == b' ')
        .count();
    Some(range.end + spaces)
}

fn base_indent(doc: &LessDocument, node: LessNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let start = node.data().range.start;
    let mut line = doc.text().line_at(start)?.range.start;
    loop {
        let mut at_break = doc.path(line, Side::Inner).last()?;
        while let Some(parent) = at_break
            .parent()
            .filter(|parent| parent.data().range.start == at_break.data().range.start)
        {
            at_break = parent;
        }
        if at_break.data().range.end > start || at_break.data().range.start >= line {
            return cx.line_indent(line, Assoc::After);
        }
        line = doc.text().line_at(at_break.data().range.start)?.range.start;
    }
}
