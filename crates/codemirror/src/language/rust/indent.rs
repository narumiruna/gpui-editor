use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    RustDocument, RustNode,
    RustNodeKind::*,
    trim_js_space,
};

pub(super) fn indentation(
    doc: &RustDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.syntax_lines(), position, settings, options)?;
    let after = cx.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_js_space(after);
    let inner = enter_unfinished(doc.path(position, None).last()?, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        match node.kind() {
            String | BlockComment => return None,
            AttributeItem => continue,
            IfExpression => {
                let block = trimmed.starts_with('{');
                let else_keyword = trimmed.strip_prefix("else").is_some_and(|rest| {
                    rest.bytes()
                        .next()
                        .is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_')
                });
                let base = base_indent(doc, node, &cx)?;
                return Some(if block || else_keyword {
                    base
                } else {
                    base.saturating_add(cx.unit())
                });
            }
            _ if node.kind() == MatchArm || node.data().statement => {
                return Some(base_indent(doc, node, &cx)?.saturating_add(cx.unit()));
            }
            _ => {}
        }
        let Some(open) = node.children().next() else {
            continue;
        };
        let closer = match open.kind() {
            OpenBrace => CloseBrace,
            OpenBracket => CloseBracket,
            OpenParen => CloseParen,
            _ => continue,
        };
        let last = node.children().next_back()?;
        let closed = last.kind() == closer
            && last.data().range.start == position + (after.len() - trimmed.len())
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

fn enter_unfinished(mut node: RustNode<'_>, position: usize) -> RustNode<'_> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.nodes[id].range.start < position);
    let mut scan = index.checked_sub(1).map(|i| RustNode {
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

fn aligned(node: RustNode<'_>, open: RustNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
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
        range_end > range.end
            && !matches!(next.kind(), LineComment | BlockComment | Shebang | Error)
    });
    if next.is_some_and(|next| next.data().range.start < line_end) {
        let spaces = cx
            .text()
            .slice(range.end..line.range.end)?
            .bytes()
            .take_while(|&b| b == b' ')
            .count();
        Some(range.end + spaces)
    } else {
        None
    }
}

fn base_indent(doc: &RustDocument, node: RustNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let start = node.data().range.start;
    let mut line = doc.text().line_at(start)?.range.start;
    loop {
        let mut at_break = doc.path(line, None).last()?;
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
