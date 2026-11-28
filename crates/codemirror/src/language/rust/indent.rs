use std::sync::{Arc, LazyLock};

use crate::Assoc;

use super::{
    super::{
        IndentContext, IndentOptions, StreamSettings,
        lezer::{INDENT, LrIndentStrategy},
        markdown::MarkdownSourceMap,
    },
    RustDocument, RustNode,
    RustNodeKind::*,
    stock_node_types, trim_js_space,
};

pub(super) fn stock_strategy(term: u16) -> Option<&'static LrIndentStrategy> {
    static RULES: LazyLock<[LrIndentStrategy; 4]> = LazyLock::new(|| {
        [
            Arc::new(|cx| {
                let lines = cx.line_context();
                let after = lines.text_after_pos(lines.position(), Assoc::After)?;
                Some(
                    cx.base_indent()?
                        .saturating_add(if_extra(trim_js_space(after), lines.unit())),
                )
            }),
            Arc::new(|_| None),
            Arc::new(|cx| cx.continue_indent()),
            Arc::new(|cx| Some(cx.base_indent()?.saturating_add(cx.line_context().unit()))),
        ]
    });
    // Exact rows from @codemirror/lang-rust 6.0.2. AttributeItem has
    // its own rule, even where it also belongs to the Statement group.
    let index = match term {
        64 => 0,
        7 | 35 => 1,
        182 | 206 => 2,
        45 | 115 | 165 | 172 | 173 | 175 | 178 | 179 | 183 | 207 | 212 | 214 | 218 | 220 | 222
        | 223 | 225 | 226 | 227 | 235 | 236 | 237 => 3,
        _ => return None,
    };
    Some(&RULES[index])
}

pub(super) fn has_configured_indentation(doc: &RustDocument) -> bool {
    let rows = doc.node_types();
    !rows.same_set(stock_node_types())
        && rows.iter().any(|row| {
            match (
                row.prop(&INDENT).and_then(Option::as_ref),
                stock_strategy(row.term()),
            ) {
                (Some(rule), Some(stock)) => !Arc::ptr_eq(rule, stock),
                (None, None) => false,
                _ => true,
            }
        })
}

pub(super) fn indentation(
    doc: &RustDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.syntax_lines(), position, settings, options)?;
    indentation_from(doc, Some(indentation_node(doc, position)?), &cx, None, None)
}

pub(in crate::language) fn indentation_node(
    doc: &RustDocument,
    position: usize,
) -> Option<RustNode<'_>> {
    Some(enter_unfinished(doc.path(position, None).last()?, position))
}

type RunCustom<'tree, 'run> = dyn Fn(RustNode<'tree>, &LrIndentStrategy) -> Option<usize> + 'run;

pub(in crate::language) fn indentation_from<'a>(
    doc: &'a RustDocument,
    start: Option<RustNode<'a>>,
    cx: &IndentContext<'_>,
    map: Option<MarkdownSourceMap<'_>>,
    run_custom: Option<&RunCustom<'a, '_>>,
) -> Option<usize> {
    let source = |at| map.map_or(at, |map| map.source_offset(at));
    let base = |node| match map {
        Some(_) => callback_base_indent(doc, node, cx, map),
        None => base_indent(doc, node, cx),
    };
    let position = cx.position();
    let after = cx.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_js_space(after);
    for node in std::iter::successors(start, |node| node.parent()) {
        let mut use_stock = true;
        if let Some(run) = run_custom
            && let Some(row) = node.node_type()
            && let Some(rule) = row.prop(&INDENT)
        {
            // Keep the specialized Rust behavior when the stock rule is unchanged.
            match rule {
                Some(rule)
                    if stock_strategy(row.term()).is_some_and(|stock| Arc::ptr_eq(stock, rule)) => {
                }
                Some(rule) => return run(node, rule),
                None => use_stock = false,
            }
        }
        if use_stock {
            match node.kind() {
                String | BlockComment => return None,
                AttributeItem => continue,
                IfExpression => {
                    return Some(base(node)?.saturating_add(if_extra(trimmed, cx.unit())));
                }
                _ if node.kind() == MatchArm || node.data().statement => {
                    return Some(base(node)?.saturating_add(cx.unit()));
                }
                _ => {}
            }
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
            && source(last.data().range.start) == position + (after.len() - trimmed.len())
            && !cx.at_double_break();
        if let Some(column_at) = aligned(node, open, cx, source) {
            let at = if closed {
                source(open.data().range.start)
            } else {
                column_at
            };
            return Some(cx.column(at, Assoc::After)?.max(0) as usize);
        }
        let base = base(node)?;
        return Some(if closed {
            base
        } else {
            base.saturating_add(cx.unit())
        });
    }
    map.is_none().then_some(0)
}

fn if_extra(trimmed: &str, unit: usize) -> usize {
    let else_keyword = trimmed.strip_prefix("else").is_some_and(|rest| {
        rest.bytes()
            .next()
            .is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_')
    });
    if trimmed.starts_with('{') || else_keyword {
        0
    } else {
        unit
    }
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

fn aligned(
    node: RustNode<'_>,
    open: RustNode<'_>,
    cx: &IndentContext<'_>,
    source: impl Fn(usize) -> usize,
) -> Option<usize> {
    let range = &open.data().range;
    let end = source(range.end);
    let line = cx.text().line_at(source(range.start))?;
    let line_end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    let next = node.children().skip(1).find(|next| {
        let range_end = next.data().range.end;
        range_end > range.end
            && !matches!(next.kind(), LineComment | BlockComment | Shebang | Error)
    });
    if next.is_some_and(|next| source(next.data().range.start) < line_end) {
        let spaces = cx
            .text()
            .slice(end..line.range.end)?
            .bytes()
            .take_while(|&b| b == b' ')
            .count();
        Some(end + spaces)
    } else {
        None
    }
}

fn base_indent(doc: &RustDocument, node: RustNode<'_>, cx: &IndentContext<'_>) -> Option<usize> {
    let start = node.data().range.start;
    let mut line = doc.text().line_at(start)?.range.start;
    loop {
        let at_break = covering_node(doc, line)?;
        if at_break.data().range.end > start || at_break.data().range.start >= line {
            return cx.line_indent(line, Assoc::After);
        }
        line = doc.text().line_at(at_break.data().range.start)?.range.start;
    }
}

pub(in crate::language) fn callback_base_indent(
    doc: &RustDocument,
    node: RustNode<'_>,
    cx: &IndentContext<'_>,
    map: Option<MarkdownSourceMap<'_>>,
) -> Option<usize> {
    let source = |at| map.map_or(at, |map| map.source_offset(at));
    let text = cx.text();
    let mut line = text.line_at(source(node.data().range.start))?.range.start;
    loop {
        let local = match map {
            None => Some(line),
            Some(map) => map
                .segment_at(line, Assoc::After)
                .map(|(segment, start)| start + (line - segment.start)),
        };
        // A Markdown prefix can put the physical line start outside Rust.
        let Some(at_break) = local.and_then(|at| covering_node(doc, at)) else {
            return cx.line_indent(line, Assoc::After);
        };
        let earlier = text
            .line_at(source(at_break.data().range.start))?
            .range
            .start;
        if earlier >= line
            || std::iter::successors(Some(node), |node| node.parent())
                .any(|node| node.id == at_break.id)
        {
            return cx.line_indent(line, Assoc::After);
        }
        line = earlier;
    }
}

fn covering_node(doc: &RustDocument, line: usize) -> Option<RustNode<'_>> {
    let mut at_break = doc.path(line, None).last()?;
    while let Some(parent) = at_break
        .parent()
        .filter(|parent| parent.data().range.start == at_break.data().range.start)
    {
        at_break = parent;
    }
    Some(at_break)
}
