use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    PhpDocument, PhpNode, tables, trim_space,
};

pub(super) fn indentation(
    doc: &PhpDocument,
    pos: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<(usize, Option<usize>)> {
    let cx = IndentContext::new(doc.text(), pos, settings, options)?;
    let after = cx.text_after_pos(pos, Assoc::After)?;
    let trimmed = trim_space(after);
    let closed_at = pos + after.len() - trimmed.len();
    let ignore_closed = cx.simulated_break() == Some(pos) && options.simulate_double_break;
    for node in std::iter::successors(doc.path(pos, false).last(), |n| n.parent()) {
        let start = node.range().start;
        let units = match node.name() {
            "⚠" | "String" | "BlockComment" => return Some((start, None)),
            "IfStatement" => usize::from(
                !trimmed.starts_with('{') && !word(trimmed, &["else", "elseif", "endif"]),
            ),
            "TryStatement" => {
                usize::from(!trimmed.starts_with('{') && !word(trimmed, &["catch", "finally"]))
            }
            "SwitchBody" => {
                if trimmed.starts_with('}') {
                    0
                } else if word(trimmed, &["case", "default"]) {
                    1
                } else {
                    2
                }
            }
            "ColonBlock" | "ArrowFunction" => 1,
            "Block" | "EnumBody" | "DeclarationList" => {
                return Some((start, delimited(doc, &cx, node, trimmed.starts_with('}'))));
            }
            _ if tables::STATEMENTS.contains(&node.data().term) => usize::from(
                !after.starts_with('{')
                    && !word(after, &["endfor", "endforeach", "endswitch", "endwhile"]),
            ),
            _ => {
                let Some(first) = node.children().next() else {
                    continue;
                };
                // Pinned `closedBy` properties, keyed by term ID because
                // castOpen and the usual opening parenthesis have the same name.
                let closing = match first.data().term {
                    72 => "phpClose",
                    78 => "}",
                    88 => ")",
                    165 => "]",
                    _ => continue,
                };
                let closed = !ignore_closed
                    && node
                        .children()
                        .next_back()
                        .is_some_and(|n| n.name() == closing && n.range().start == closed_at);
                return Some((start, delimited(doc, &cx, node, closed)));
            }
        };
        let indent = base_indent(doc, &cx, node)
            .map(|base| base.saturating_add(cx.unit().saturating_mul(units)));
        return Some((start, indent));
    }
    None
}

fn word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| {
        text.strip_prefix(word).is_some_and(|rest| {
            !rest.starts_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_')
        })
    })
}

fn delimited(
    doc: &PhpDocument,
    cx: &IndentContext<'_>,
    node: PhpNode<'_>,
    closed: bool,
) -> Option<usize> {
    let mut children = node.children();
    let open = children.next()?.range();
    children.next_back();
    let first = children.find(|n| n.name() != "⚠");
    let line = doc.text().line_at(open.start)?;
    let line_end = cx
        .simulated_break()
        .filter(|&pos| pos > line.range.start)
        .map_or(line.range.end, |pos| pos.min(line.range.end));
    if first.is_some_and(|n| n.range().start < line_end) {
        let column = if closed {
            open.start
        } else {
            let tail = doc.text().slice(open.end..line.range.end)?;
            open.end + tail.bytes().take_while(|&ch| ch == b' ').count()
        };
        return cx
            .column(column, Assoc::After)
            .map(|col| usize::try_from(col).unwrap_or(0));
    }
    Some(base_indent(doc, cx, node)?.saturating_add(if closed { 0 } else { cx.unit() }))
}

fn base_indent(doc: &PhpDocument, cx: &IndentContext<'_>, node: PhpNode<'_>) -> Option<usize> {
    let mut start = node.range().start;
    loop {
        let line_start = doc.text().line_at(start)?.range.start;
        let mut at_break = doc.path(line_start, false).last()?;
        while let Some(parent) = at_break
            .parent()
            .filter(|p| p.range().start == at_break.range().start)
        {
            at_break = parent;
        }
        if std::iter::successors(Some(node), |n| n.parent()).any(|n| n.id == at_break.id)
            || at_break.range().start >= line_start
        {
            return cx.line_indent(line_start, Assoc::After);
        }
        start = at_break.range().start;
    }
}
