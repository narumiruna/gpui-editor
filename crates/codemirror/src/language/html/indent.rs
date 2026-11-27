use crate::Assoc;

use super::{
    super::IndentContext, HtmlDocument, HtmlNode, HtmlNodeKind::*, IndentOptions, StreamSettings,
    nesting, trim_js_space,
};

pub(super) fn indentation(
    doc: &HtmlDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    if let Some(indent) = nesting::indentation(doc, position, settings, options) {
        return Some(indent);
    }
    let node = enter_unfinished(doc.path(position, None).last()?, position);
    let after = cx.text_after_pos(position, Assoc::After)?;
    let trimmed = trim_js_space(after);
    let closing = trimmed.starts_with("</");
    let whitespace = after.len() - trimmed.len();
    let reference = position + whitespace;
    for node in std::iter::successors(Some(node), |node| node.parent()) {
        let range = &node.data().range;
        match node.kind() {
            Element if range.end > reference + 2 * usize::from(closing) => {
                let indent = cx.line_indent(range.start, Assoc::After)?;
                return Some(if closing {
                    indent
                } else {
                    indent.saturating_add(cx.unit())
                });
            }
            OpenTag | CloseTag | SelfClosingTag => {
                let column = cx.column(range.start, Assoc::After)?;
                return Some(if column < 0 {
                    cx.unit().saturating_sub(column.unsigned_abs())
                } else {
                    (column as usize).saturating_add(cx.unit())
                });
            }
            Document if range.end <= reference => {
                let mut last = node.children().next_back()?;
                if last.kind() != Element || last.data().range.end != range.end {
                    return None;
                }
                while let Some(child) = last
                    .children()
                    .next_back()
                    .filter(|child| child.kind() == Element && child.data().range.end == range.end)
                {
                    last = child;
                }
                return (!last
                    .children()
                    .next_back()
                    .is_some_and(|node| matches!(node.kind(), CloseTag | SelfClosingTag)))
                .then(|| {
                    cx.line_indent(last.data().range.start, Assoc::After)
                        .map(|indent| indent.saturating_add(cx.unit()))
                })
                .flatten();
            }
            _ => {}
        }
    }
    Some(0)
}

fn enter_unfinished(mut node: HtmlNode<'_>, position: usize) -> HtmlNode<'_> {
    let children = &node.data().children;
    let i = children.partition_point(|&id| node.nodes[id].range.start < position);
    let mut scan = i.checked_sub(1).map(|i| HtmlNode {
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
