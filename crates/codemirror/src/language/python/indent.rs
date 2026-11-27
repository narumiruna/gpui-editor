// Port of @codemirror/lang-python 6.2.1's indent rules. See docs/licenses/codemirror-lang-python.txt.
use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    PythonDocument, PythonNode, js_space, tables, trim_space,
};

pub(super) fn indentation(
    doc: &PythonDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let after = cx.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_space(after);
    let inner = enter_unfinished(doc.path(position).last()?, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        match node.name() {
            "Body" | "MatchBody" | "Script" => {
                let body = if node.name() != "Body" || blank(trimmed) {
                    inner_body(node, &cx)
                } else {
                    None
                }
                .or((node.name() != "Script").then_some(node));
                if let Some(body) = body
                    && let Some(column) = indent_body(node, body, &cx, trimmed)
                {
                    return Some(column);
                }
            }
            "IfStatement" if trimmed.starts_with("else:") || trimmed.starts_with("elif ") => {
                return base(node, &cx);
            }
            "ForStatement" | "WhileStatement" if trimmed.starts_with("else:") => {
                return base(node, &cx);
            }
            "TryStatement"
                if ["except ", "except:", "finally:", "else:"]
                    .iter()
                    .any(|prefix| trimmed.starts_with(prefix)) =>
            {
                return base(node, &cx);
            }
            "MatchStatement" if trimmed.starts_with("case ") => {
                return base(node, &cx).map(|base| base.saturating_add(cx.unit()));
            }
            "TupleExpression"
            | "ComprehensionExpression"
            | "ParamList"
            | "ArgList"
            | "ParenthesizedExpression" => return delimited(node, &cx, trimmed.starts_with(')')),
            "DictionaryExpression"
            | "DictionaryComprehensionExpression"
            | "SetExpression"
            | "SetComprehensionExpression" => {
                return delimited(node, &cx, trimmed.starts_with('}'));
            }
            "ArrayExpression" | "ArrayComprehensionExpression" => {
                return delimited(node, &cx, trimmed.starts_with(']'));
            }
            "MemberExpression" => {
                return base(node, &cx).map(|base| base.saturating_add(cx.unit()));
            }
            "String" | "FormatString" => return None,
            _ => {
                let close = match node.children().next().map(PythonNode::name) {
                    Some("(") => ")",
                    Some("[") => "]",
                    Some("{") => "}",
                    _ => continue,
                };
                let closed = node.children().next_back().is_some_and(|last| {
                    last.name() == close
                        && last.data().range.start == position + after.len() - trimmed.len()
                        && !(options.simulate_double_break
                            && cx.simulated_break() == Some(position))
                });
                return delimited(node, &cx, closed);
            }
        }
    }
    if doc.root().children().next_back().is_some_and(|last| {
        last.name() == "⚠" && !last.range().is_empty() && last.range().start <= position
    }) {
        return fallback_indent(&cx, trimmed);
    }
    Some(0)
}

fn blank(text: &str) -> bool {
    text.is_empty() || text.starts_with('#')
}

fn child_before(node: PythonNode<'_>, position: usize) -> Option<PythonNode<'_>> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.doc.nodes[id].range.start < position);
    index.checked_sub(1).map(|index| PythonNode {
        id: children[index],
        ..node
    })
}

fn enter_unfinished(mut node: PythonNode<'_>, position: usize) -> PythonNode<'_> {
    let mut scan = child_before(node, position);
    while let Some(candidate) = scan {
        let mut children = candidate.children();
        let Some(last) = children
            .next_back()
            .filter(|last| last.data().range.end == candidate.data().range.end)
        else {
            break;
        };
        if last.name() == "⚠" && last.data().range.is_empty() {
            node = candidate;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn inner_body<'a>(mut node: PythonNode<'a>, cx: &IndentContext<'_>) -> Option<PythonNode<'a>> {
    let mut pos = cx.position();
    let line_indent = cx.line_indent(pos, Assoc::Before)?;
    let mut found = None;
    while let Some(before) = child_before(node, pos) {
        match before.name() {
            "Comment" => pos = before.data().range.start,
            "Body" | "MatchBody" => {
                if base(before, cx)?.saturating_add(cx.unit()) <= line_indent {
                    found = Some(before);
                }
                node = before;
            }
            "MatchClause" => node = before,
            _ if tables::STATEMENTS.contains(&before.data().term) => node = before,
            _ => break,
        }
    }
    found
}

fn indent_body(
    node: PythonNode<'_>,
    body: PythonNode<'_>,
    cx: &IndentContext<'_>,
    trimmed: &str,
) -> Option<usize> {
    let base = base(body, cx)?;
    let line = cx.line_at(cx.position(), Assoc::Before)?;
    let current = cx.line_indent(cx.position(), Assoc::Before)?;
    let node_end = node.data().range.end;
    if blank(trim_space(line.text))
        && node_end < line.range.end.saturating_add(100)
        && cx
            .text()
            .slice(line.range.end..node_end.max(line.range.end))?
            .chars()
            .all(js_space)
        && current <= base
    {
        return None;
    }
    let case = trimmed.strip_prefix("case").is_some_and(|rest| {
        rest.starts_with(js_space)
            && trim_space(rest)
                .split_once(':')
                .is_some_and(|(value, _)| !value.is_empty() && !value.contains('='))
    });
    if current > base
        && (case
            || ["else:", "elif ", "except ", "finally:"]
                .iter()
                .any(|prefix| trimmed.starts_with(prefix)))
    {
        return None;
    }
    Some(base.saturating_add(cx.unit()))
}

fn delimited(node: PythonNode<'_>, cx: &IndentContext<'_>, closed: bool) -> Option<usize> {
    let mut children = node.children();
    if let Some(open) = children.next() {
        children.next_back();
        let open = open.range();
        let line = cx.text().line_at(open.start)?;
        let line_end = cx
            .simulated_break()
            .filter(|&at| at > line.range.start)
            .map_or(line.range.end, |at| at.min(line.range.end));
        if let Some(next) = children.find(|next| {
            next.data().range.end > open.end && !matches!(next.name(), "Comment" | "⚠")
        }) && next.data().range.start < line_end
            && let Some(rest) = cx.text().slice(open.end..line.range.end)
        {
            let aligned = if closed {
                open.start
            } else {
                open.end + rest.bytes().take_while(|&byte| byte == b' ').count()
            };
            return cx
                .column(aligned, Assoc::After)
                .map(|column| column.max(0) as usize);
        }
    }
    base(node, cx).map(|base| base.saturating_add(if closed { 0 } else { cx.unit() }))
}

fn base(node: PythonNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let mut line = cx.text().line_at(node.data().range.start)?.range.start;
    loop {
        let mut at_break = node.doc.path(line).last()?;
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
        line = cx.text().line_at(at_break.data().range.start)?.range.start;
    }
}

fn fallback_indent(cx: &IndentContext<'_>, after: &str) -> Option<usize> {
    let current = cx.line_at(cx.position(), Assoc::After)?;
    if current.range.start == 0 {
        return Some(0);
    }
    if let Some(column) = recovered_clause_indent(cx, &current, after) {
        return Some(column);
    }
    let mut number = current.number;
    // Do not scan an unbounded run of blank lines on a single indent query.
    for _ in 0..32 {
        let line = cx.text().line(number)?;
        let end = current.range.start.min(line.range.end);
        if line.range.start == end {
            number = number.checked_sub(1)?;
            continue;
        }
        let previous = cx.text().slice(line.range.start..end)?.trim_end();
        if blank(trim_space(previous)) {
            number = number.checked_sub(1)?;
            continue;
        }
        let column = cx.line_indent(line.range.start, Assoc::Before)?;
        return Some(if after.starts_with([')', ']', '}']) {
            column.saturating_sub(cx.unit())
        } else if previous.ends_with([':', '(', '[', '{']) {
            column.saturating_add(cx.unit())
        } else {
            column
        });
    }
    Some(0)
}

fn recovered_clause_indent(
    cx: &IndentContext<'_>,
    current: &crate::Line<'_>,
    after: &str,
) -> Option<usize> {
    let headers: &[&str] = if after.starts_with("elif ") {
        &["if", "elif"]
    } else if after.starts_with("else:") {
        &["if", "elif", "for", "async for", "while", "try", "except"]
    } else if after.starts_with("except ") || after.starts_with("finally:") {
        &["try", "except"]
    } else {
        return None;
    };
    // The strict parser can lose an enclosing statement when its clause
    // starts at an unexpected indentation. Look back only across dedents.
    let mut limit = cx.line_indent(current.range.start, Assoc::Before)?;
    for number in (current.number.saturating_sub(32).max(1)..current.number).rev() {
        let line = cx.text().line(number)?;
        let text = trim_space(line.text).split('#').next()?.trim_end();
        if !blank(text) {
            let column = cx.line_indent(line.range.start, Assoc::Before)?;
            if column <= limit
                && text.ends_with(':')
                && headers.iter().any(|header| {
                    text.strip_prefix(header)
                        .is_some_and(|tail| tail.starts_with([' ', '\t', '(', ':', '*']))
                })
            {
                return Some(column);
            }
            limit = limit.min(column);
        }
    }
    None
}
