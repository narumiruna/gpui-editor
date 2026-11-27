use std::ops::Range;

use crate::Assoc;

use super::{
    super::IndentContext,
    IndentOptions, LiquidDocument, LiquidNode,
    LiquidNodeKind::{self, *},
    StreamSettings, trim_space,
};

pub(super) fn fold_range(node: LiquidNode<'_>) -> Option<Range<usize>> {
    if !matches!(
        node.kind(),
        IfDirective
            | UnlessDirective
            | CaseDirective
            | ForDirective
            | TableDirective
            | CaptureDirective
            | RawDirective
            | Comment
    ) {
        return None;
    }
    let first = node.children().next().filter(|child| child.kind() == Tag)?;
    let last = node.children().next_back()?;
    let start = first.range().end;
    let end = if last.kind() == EndTag {
        last.range().start
    } else {
        node.range().end
    };
    (start < end).then_some(start..end)
}

pub(super) fn foldable(
    doc: &LiquidDocument,
    line_start: usize,
    line_end: usize,
) -> Option<Range<usize>> {
    doc.text().slice(line_start..line_end)?;
    let liquid = doc
        .path(line_end, true)
        .filter_map(fold_range)
        .find(|range| line_start <= range.start && range.start <= line_end && line_end < range.end);
    let html = doc
        .html()
        .and_then(|html| html.foldable(line_start, line_end));
    match (liquid, html) {
        (Some(a), Some(b)) => Some(if a.start <= b.start { a } else { b }),
        (a, b) => a.or(b),
    }
}

pub(super) fn indentation(
    doc: &LiquidDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let after = trim_space(cx.text_after_pos(position, Assoc::After)?);
    let command = trim_space(
        after
            .strip_prefix("{%-")
            .or_else(|| after.strip_prefix("{%"))
            .unwrap_or(after),
    );
    let deepest = enter_unfinished(doc.path(position, false).last()?, position);
    let mut directive = None;
    for node in std::iter::successors(Some(deepest), |node| node.parent()) {
        match node.kind() {
            Tag | EndTag | Interpolation | RangeExpression => {
                return delimited(node, &cx, after, options.simulate_double_break);
            }
            IfDirective | UnlessDirective | CaseDirective | ForDirective | TableDirective
            | CaptureDirective => {
                let base = cx.line_indent(node.range().start, Assoc::After)?;
                if directive_closing(node.kind(), command) {
                    return Some(base);
                }
                directive = Some(base.saturating_add(cx.unit()));
                break;
            }
            _ => {}
        }
    }
    if let Some(html) = doc.html()
        && let Some(indent) = html.indentation(doc.text(), position, settings, options)
        && (directive.is_none() || indent > 0 || after.starts_with("</"))
    {
        return Some(indent);
    }
    directive.or(Some(0))
}

fn directive_closing(kind: LiquidNodeKind, after: &str) -> bool {
    fn word(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }
    let matches = |start| {
        after
            .strip_prefix(start)
            .is_some_and(|rest| !rest.starts_with(word))
    };
    match kind {
        IfDirective => ["endif", "else", "elsif"].into_iter().any(matches),
        CaseDirective => ["endcase", "when"].into_iter().any(matches),
        _ => after
            .strip_prefix("end")
            .is_some_and(|rest| rest.starts_with(word)),
    }
}

fn delimited(
    node: LiquidNode<'_>,
    cx: &IndentContext<'_>,
    after: &str,
    double: bool,
) -> Option<usize> {
    let first = node.children().next()?;
    let last = node.children().next_back()?;
    let position = cx.position();
    let closing = if node.kind() == Tag {
        after.starts_with("%}")
    } else if double && cx.simulated_break() == Some(position) {
        false
    } else {
        let expected = match first.kind() {
            TagStart => TagEnd,
            InterpolationStart => InterpolationEnd,
            OpenParen => CloseParen,
            _ => return None,
        };
        last.kind() == expected
            && cx
                .text_after_pos(position, Assoc::After)
                .is_some_and(|text| position + text.len() - after.len() == last.range().start)
    };
    let start = first.range().start;
    let end = first.range().end;
    let line = cx.text().line_at(start)?;
    let line_end = cx
        .simulated_break()
        .filter(|point| *point > line.range.start)
        .map_or(line.range.end, |point| point.min(line.range.end));
    let next = node
        .children()
        .skip(1)
        .take(node.children().len().saturating_sub(2))
        .find(|child| !matches!(child.kind(), InlineComment | Error));
    if next.is_some_and(|child| child.range().start < line_end) {
        let space = cx.text().as_str()[end..]
            .bytes()
            .take_while(|byte| *byte == b' ')
            .count();
        return cx
            .column(if closing { start } else { end + space }, Assoc::Before)
            .map(|column| column.max(0) as usize);
    }
    let base = cx.line_indent(node.range().start, Assoc::Before)?;
    Some(if closing {
        base
    } else {
        base.saturating_add(cx.unit())
    })
}

fn enter_unfinished(mut node: LiquidNode<'_>, position: usize) -> LiquidNode<'_> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.nodes[id].range.start < position);
    let mut scan = index.checked_sub(1).map(|i| LiquidNode {
        id: children[i],
        ..node
    });
    while let Some(candidate) = scan {
        let mut children = candidate.children();
        let Some(last) = children
            .next_back()
            .filter(|child| child.range().end == candidate.range().end)
        else {
            break;
        };
        if last.kind() == Error && last.range().is_empty() {
            node = candidate;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}
