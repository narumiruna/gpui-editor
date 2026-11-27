// Indentation rules from @codemirror/lang-javascript 6.2.5.
use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    JavascriptDocument, JavascriptNode, tables, trim_js_space,
};

pub(super) fn indentation(
    doc: &JavascriptDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
    outer: impl FnOnce() -> Option<usize>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let after = cx.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_js_space(after);
    let inner = enter_unfinished(doc.path(position, None).last()?, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        let units = match node.kind().name() {
            "TemplateString" | "BlockComment" => return None,
            "LabeledStatement" => 0,
            "IfStatement" => {
                usize::from(!(trimmed.starts_with('{') || starts_word(trimmed, "else")))
            }
            "TryStatement" => usize::from(
                !(trimmed.starts_with('{')
                    || starts_word(trimmed, "catch")
                    || starts_word(trimmed, "finally")),
            ),
            "SwitchBody" => {
                if trimmed.starts_with('}') {
                    0
                } else if starts_word(trimmed, "case") || starts_word(trimmed, "default") {
                    1
                } else {
                    2
                }
            }
            "ArrowFunction" => 1,
            "Block" => return delimited(doc, node, &cx, trimmed.starts_with('}')),
            "JSXElement" | "JSXEscape" => {
                let closed = if node.kind().name() == "JSXElement" {
                    trimmed.starts_with("</")
                } else {
                    trimmed.contains('}')
                };
                return cx
                    .line_indent(node.data().range.start, Assoc::After)
                    .map(|column| column.saturating_add(if closed { 0 } else { cx.unit() }));
            }
            "JSXOpenTag" | "JSXSelfClosingTag" => {
                return cx
                    .column(node.data().range.start, Assoc::After)
                    .map(|column| (column.max(0) as usize).saturating_add(cx.unit()));
            }
            "Property" => usize::from(!trimmed.starts_with('{')),
            _ if is_statement(node) => usize::from(!trimmed.starts_with('{')),
            _ => {
                let Some(open) = node.children().next() else {
                    continue;
                };
                let closer = match open.kind().name() {
                    "{" => "}",
                    "[" => "]",
                    "(" => ")",
                    "<" if matches!(open.data().term, Some(24 | 169)) => ">",
                    "InterpolationStart" => "InterpolationEnd",
                    "JSXStartCloseTag" => "JSXEndTag",
                    _ => continue,
                };
                let last = node.children().next_back()?;
                let closed = last.kind().name() == closer
                    && last.data().range.start == position + (after.len() - trimmed.len())
                    && !(options.simulate_double_break && cx.simulated_break() == Some(position));
                return delimited(doc, node, &cx, closed);
            }
        };
        return base_indent(doc, node, &cx)
            .map(|base| base.saturating_add(units.saturating_mul(cx.unit())));
    }
    outer()
}

fn is_statement(node: JavascriptNode<'_>) -> bool {
    node.data()
        .term
        .is_some_and(|term| tables::STATEMENTS.contains(&term))
}

fn starts_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word).is_some_and(|rest| {
        rest.bytes()
            .next()
            .is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_')
    })
}

fn enter_unfinished(mut node: JavascriptNode<'_>, position: usize) -> JavascriptNode<'_> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.nodes[id].range.start < position);
    let mut scan = index.checked_sub(1).map(|i| JavascriptNode {
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
        if last.kind().is_error() && last.data().range.is_empty() {
            node = candidate;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn delimited(
    doc: &JavascriptDocument,
    node: JavascriptNode<'_>,
    cx: &IndentContext<'_>,
    closed: bool,
) -> Option<usize> {
    if let Some((open, after)) = aligned(node, cx) {
        return Some(
            cx.column(if closed { open } else { after }, Assoc::After)?
                .max(0) as usize,
        );
    }
    let base = base_indent(doc, node, cx)?;
    Some(if closed {
        base
    } else {
        base.saturating_add(cx.unit())
    })
}

fn aligned(node: JavascriptNode<'_>, cx: &IndentContext<'_>) -> Option<(usize, usize)> {
    let mut children = node.children();
    let open = children.next()?.range();
    let line = cx.text().line_at(open.start)?;
    let line_end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    children.next_back();
    let next = children.find(|next| {
        next.data().range.start >= open.end
            && !next.kind().is_error()
            && !matches!(next.kind().name(), "BlockComment" | "LineComment")
    })?;
    if next.data().range.start >= line_end {
        return None;
    }
    let spaces = cx
        .text()
        .slice(open.end..line.range.end)?
        .bytes()
        .take_while(|&b| b == b' ')
        .count();
    Some((open.start, open.end + spaces))
}

fn base_indent(
    doc: &JavascriptDocument,
    node: JavascriptNode<'_>,
    cx: &IndentContext<'_>,
) -> Option<usize> {
    let mut line = doc.text().line_at(node.data().range.start)?.range.start;
    loop {
        let mut at_break = doc.path(line, None).last()?;
        while let Some(parent) = at_break
            .parent()
            .filter(|parent| parent.data().range.start == at_break.data().range.start)
        {
            at_break = parent;
        }
        if std::iter::successors(Some(node), |node| node.parent())
            .any(|ancestor| ancestor.id == at_break.id)
        {
            return cx.line_indent(line, Assoc::After);
        }
        line = doc.text().line_at(at_break.data().range.start)?.range.start;
    }
}
