use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    CppDocument, CppNode,
    CppNodeKind::*,
    tables, trim_js_space,
};

pub(super) fn indentation(
    doc: &CppDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.syntax_lines(), position, settings, options)?;
    let after = cx.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_js_space(after);
    let inner = enter_unfinished(doc.path(position, None).last()?, position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        let flat = match node.kind() {
            BlockComment => return None,
            IfStatement => trimmed.starts_with('{') || starts_word(trimmed, "else"),
            // The boundary in the upstream regexp also applies to `{`.
            TryStatement => {
                starts_word(trimmed, "catch")
                    || (trimmed.starts_with('{') && word_byte(trimmed.as_bytes().get(1)))
            }
            LabeledStatement => true,
            CaseStatement => false,
            CompoundStatement => return delimited(doc, node, &cx, trimmed.starts_with('}')),
            _ if tables::STATEMENTS.contains(&node.data().term) => after.starts_with('{'),
            _ => {
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
                return delimited(doc, node, &cx, closed);
            }
        };
        return base_indent(doc, node, &cx)
            .map(|base| base.saturating_add(if flat { 0 } else { cx.unit() }));
    }
    Some(0)
}

fn word_byte(byte: Option<&u8>) -> bool {
    byte.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn starts_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word)
        .is_some_and(|rest| !word_byte(rest.as_bytes().first()))
}

fn enter_unfinished(mut node: CppNode<'_>, position: usize) -> CppNode<'_> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.doc.nodes[id].range.start < position);
    let mut scan = index.checked_sub(1).map(|i| CppNode {
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

fn delimited(
    doc: &CppDocument,
    node: CppNode<'_>,
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

fn aligned(node: CppNode<'_>, cx: &IndentContext<'_>) -> Option<(usize, usize)> {
    let mut children = node.children();
    let open = children.next()?.range();
    let line = cx.text().line_at(open.start)?;
    let line_end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    let next = children.find(|next| {
        next.data().range.end > open.end && !tables::SKIPPED.contains(&next.data().term)
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

fn base_indent(doc: &CppDocument, node: CppNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let mut line = cx.text().line_at(node.data().range.start)?.range.start;
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
        line = cx.text().line_at(at_break.data().range.start)?.range.start;
    }
}
