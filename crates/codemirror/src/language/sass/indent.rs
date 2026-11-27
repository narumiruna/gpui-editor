use crate::Assoc;

use super::super::{IndentContext, IndentOptions, StreamSettings};
use super::{SassDialect, SassDocument, SassNode};

pub(super) fn indentation(
    doc: &SassDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let inner = enter_unfinished(doc.path(position, 0).last()?, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        if node.name() == "Declaration"
            || doc.dialect() == SassDialect::Indented && matches!(node.name(), "Block" | "RuleSet")
        {
            return Some(base_indent(doc, node, &cx)?.saturating_add(cx.unit()));
        }
        let Some(open) = node.children().next() else {
            continue;
        };
        let closer = match open.name() {
            "{" => "}",
            "[" => "]",
            "(" => ")",
            "InterpolationStart" => "InterpolationEnd",
            "InterpolationEnd" => "InterpolationStart",
            _ => continue,
        };
        let last = node.children().next_back()?;
        let after = cx.text_after_pos(position, Assoc::After)?;
        let rest = after.trim_start_matches(|ch: char| {
            ch.is_whitespace() && ch != '\u{85}' || ch == '\u{feff}'
        });
        let closed = last.name() == closer
            && last.range().start == position + (after.len() - rest.len())
            && !(options.simulate_double_break && cx.simulated_break() == Some(position));
        if let Some(aligned) = alignment(node, open, &cx) {
            let at = if closed { open.range().start } else { aligned };
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

fn enter_unfinished(mut node: SassNode<'_>, pos: usize) -> SassNode<'_> {
    let mut scan = node.children().rfind(|child| child.range().start < pos);
    while let Some(candidate) = scan {
        let mut children = candidate.children();
        let Some(last) = children
            .next_back()
            .filter(|last| last.range().end == candidate.range().end)
        else {
            break;
        };
        if last.is_error() && last.range().is_empty() {
            node = candidate;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn alignment(node: SassNode<'_>, open: SassNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let range = open.range();
    let line = cx.text().line_at(range.start)?;
    let line_end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    let mut children = node.children();
    children.next();
    children.next_back();
    let next = children.find(|child| {
        child.range().end > range.end && !matches!(child.name(), "Comment" | "LineComment")
    })?;
    if next.range().start >= line_end {
        return None;
    }
    let gap = cx.text().slice(range.end..line.range.end)?;
    Some(range.end + gap.bytes().take_while(|&byte| byte == b' ').count())
}

fn base_indent(doc: &SassDocument, node: SassNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let mut line = doc.text().line_at(node.range().start)?.range.start;
    loop {
        let mut at_break = doc.path(line, 0).last()?;
        while let Some(parent) = at_break
            .parent()
            .filter(|parent| parent.range().start == at_break.range().start)
        {
            at_break = parent;
        }
        if std::iter::successors(Some(node), |ancestor| ancestor.parent())
            .any(|ancestor| ancestor.id == at_break.id)
        {
            return cx.line_indent(line, Assoc::After);
        }
        line = doc.text().line_at(at_break.range().start)?.range.start;
    }
}
