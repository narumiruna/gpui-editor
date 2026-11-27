// Indent rules from @codemirror/lang-java 6.0.2 and the Lezer delimiter properties.
use crate::Assoc;

use super::{
    super::{IndentContext, IndentOptions, StreamSettings},
    JavaDocument, JavaNode, tables, trim_js_space,
};

pub(super) enum IndentRule {
    Comment,
    Label,
    If,
    Try,
    Switch,
    Block,
    Statement,
}

pub(super) fn rule(node: JavaNode<'_>) -> Option<IndentRule> {
    Some(match node.name() {
        "BlockComment" => IndentRule::Comment,
        "LabeledStatement" => IndentRule::Label,
        "IfStatement" => IndentRule::If,
        "TryStatement" => IndentRule::Try,
        "SwitchBlock" => IndentRule::Switch,
        "Block" => IndentRule::Block,
        _ if tables::STATEMENTS.contains(&node.term()) => IndentRule::Statement,
        _ => return None,
    })
}

pub(super) fn indentation(
    doc: &JavaDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let context = IndentContext::new(doc.syntax_lines(), position, settings, options)?;
    let after = context.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_js_space(after);
    let inner = enter_unfinished(doc.resolve(position, None), position);
    for node in std::iter::successors(Some(inner), |node| node.parent()) {
        let units = match rule(node) {
            Some(IndentRule::Comment) => return None,
            Some(IndentRule::Label) => 0,
            Some(IndentRule::If) => {
                usize::from(!(trimmed.starts_with('{') || starts_word(trimmed, "else")))
            }
            // The word boundary in /({|catch|finally)\b/ also applies to `{`.
            Some(IndentRule::Try) => usize::from(
                !(starts_word(trimmed, "catch")
                    || starts_word(trimmed, "finally")
                    || trimmed.strip_prefix('{').is_some_and(is_word_start)),
            ),
            Some(IndentRule::Switch) => {
                if trimmed.starts_with('}') {
                    0
                } else if starts_word(trimmed, "case") || starts_word(trimmed, "default") {
                    1
                } else {
                    2
                }
            }
            Some(IndentRule::Block) => {
                return delimited(doc, node, &context, trimmed.starts_with('}'));
            }
            Some(IndentRule::Statement) => usize::from(!after.starts_with('{')),
            None => {
                let mut children = node.children();
                let Some(closer) = children.next().and_then(JavaNode::closed_by) else {
                    continue;
                };
                let closed = children.next_back().is_some_and(|last| {
                    last.name() == closer
                        && last.data().range.start == position + (after.len() - trimmed.len())
                }) && !(options.simulate_double_break
                    && context.simulated_break() == Some(position));
                return delimited(doc, node, &context, closed);
            }
        };
        return base_indent(doc, node, &context)
            .map(|base| base.saturating_add(units.saturating_mul(context.unit())));
    }
    Some(0)
}

fn is_word_start(text: &str) -> bool {
    text.bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn starts_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word)
        .is_some_and(|rest| !is_word_start(rest))
}

fn enter_unfinished(mut node: JavaNode<'_>, position: usize) -> JavaNode<'_> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.nodes[id].range.start < position);
    let mut scan = index.checked_sub(1).map(|index| JavaNode {
        id: children[index],
        ..node
    });
    while let Some(candidate) = scan {
        let mut children = candidate.children();
        let Some(last) = children.next_back() else {
            break;
        };
        if last.data().range.end != candidate.data().range.end {
            break;
        }
        if last.term() == 0 && last.data().range.is_empty() {
            node = candidate;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn delimited(
    doc: &JavaDocument,
    node: JavaNode<'_>,
    context: &IndentContext<'_>,
    closed: bool,
) -> Option<usize> {
    if let Some((open, after)) = aligned(node, context) {
        let column = context.column(if closed { open } else { after }, Assoc::After)?;
        return Some(column.max(0) as usize);
    }
    base_indent(doc, node, context)
        .map(|base| base.saturating_add(if closed { 0 } else { context.unit() }))
}

fn aligned(node: JavaNode<'_>, context: &IndentContext<'_>) -> Option<(usize, usize)> {
    let mut children = node.children();
    let start = node.data().range.start;
    let open = children
        .find(|child| child.data().range.end > start)?
        .range();
    let line = context.text().line_at(open.start)?;
    let line_end = context
        .simulated_break()
        .filter(|&point| point > line.range.start)
        .map_or(line.range.end, |point| point.min(line.range.end));
    // Lezer's childAfter and lastChild return separate node wrappers. The
    // closing token can itself establish alignment, as in `call()`.
    let next = children.find(|child| child.data().range.end > open.end && !child.is_skipped())?;
    if next.data().range.start >= line_end {
        return None;
    }
    let spaces = context
        .text()
        .slice(open.end..line.range.end)?
        .bytes()
        .take_while(|&byte| byte == b' ')
        .count();
    Some((open.start, open.end + spaces))
}

fn base_indent(
    doc: &JavaDocument,
    node: JavaNode<'_>,
    context: &IndentContext<'_>,
) -> Option<usize> {
    let mut line = context.text().line_at(node.data().range.start)?.range.start;
    loop {
        let mut at_break = doc.resolve(line, None);
        while let Some(parent) = at_break
            .parent()
            .filter(|parent| parent.data().range.start == at_break.data().range.start)
        {
            at_break = parent;
        }
        if std::iter::successors(Some(node), |node| node.parent()).any(|n| n.id == at_break.id) {
            return context.line_indent(line, Assoc::After);
        }
        line = context
            .text()
            .line_at(at_break.data().range.start)?
            .range
            .start;
    }
}
