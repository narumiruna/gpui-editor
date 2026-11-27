use std::ops::Range;

use crate::Assoc;

use super::{
    super::IndentContext, IndentOptions, JinjaDocument, JinjaNode, StreamSettings, trim_space,
};

pub(super) fn fold_range(node: JinjaNode<'_>) -> Option<Range<usize>> {
    if !is_statement(node.name()) && node.name() != "Comment" {
        return None;
    }
    let first = node.children().next()?;
    if !matches!(first.name(), "Tag" | "{#") {
        return None;
    }
    let last = node.children().next_back()?;
    let start = first.range().end;
    let end = if matches!(last.name(), "EndTag" | "#}") {
        last.range().start
    } else {
        node.range().end
    };
    (start < end).then_some(start..end)
}

pub(super) fn foldable(doc: &JinjaDocument, start: usize, end: usize) -> Option<Range<usize>> {
    doc.text().slice(start..end)?;
    let jinja = doc
        .path(end, true, false)
        .filter_map(fold_range)
        .find(|range| (start..=end).contains(&range.start) && end < range.end);
    let html = doc.html().and_then(|html| html.foldable(start, end));
    jinja
        .into_iter()
        .chain(html)
        .min_by_key(|range| range.start)
}

pub(super) fn indentation(
    doc: &JinjaDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let scope = doc
        .path(position, false, true)
        .filter_map(|node| {
            if is_statement(node.name()) {
                Some((node, IndentRule::Statement))
            } else if node.name() == "Tag" {
                Some((node, IndentRule::Tag))
            } else {
                let closing = match node.children().next()?.name() {
                    "{{" => "}}",
                    "{#" => "#}",
                    "{%" => "%}",
                    "(" => ")",
                    _ => return None,
                };
                Some((node, IndentRule::Delimited(closing)))
            }
        })
        .last();
    if let Some((start, result)) = doc
        .html()
        .and_then(|html| html.indentation(&cx, settings, options))
        && scope.is_none_or(|(node, _)| node.range().start < start)
    {
        return result;
    }
    let Some((node, rule)) = scope else {
        return Some(0);
    };
    let after = cx.text_after_pos(position, Assoc::After)?;
    let next = trim_space(after);
    let reference = node.range().start;
    let base = cx.line_indent(reference, Assoc::After)?;
    if matches!(rule, IndentRule::Statement) {
        let tag = next.strip_prefix("{%").map_or(next, |after| {
            trim_space(after.strip_prefix('-').unwrap_or(after))
        });
        let back = if matches!(node.name(), "IfStatement" | "ForStatement") {
            ["endif", "endfor", "else", "elif"].iter().any(|word| {
                tag.strip_prefix(word)
                    .is_some_and(|tail| !tail.as_bytes().first().is_some_and(|&b| word_byte(b)))
            })
        } else {
            tag.strip_prefix("end")
                .and_then(|tail| tail.as_bytes().first())
                .is_some_and(|&b| word_byte(b))
        };
        return Some(if back {
            base
        } else {
            base.saturating_add(cx.unit())
        });
    }
    let last = node.children().next_back()?;
    let closed = match rule {
        IndentRule::Tag => next.starts_with("%}"),
        IndentRule::Delimited(closing) => {
            let ignore = options.simulate_double_break && cx.simulated_break() == Some(position);
            !ignore
                && last.name() == closing
                && last.range().start == position + after.len() - next.len()
        }
        IndentRule::Statement => unreachable!(),
    };
    let open = node.children().next()?.range();
    let line = cx.line_at(open.start, Assoc::After)?;
    let line_end = cx
        .simulated_break()
        .filter(|&at| at > line.range.start)
        .map_or(line.range.end, |at| at.min(line.range.end));
    let aligned = node.children().nth(1).is_some_and(|child| {
        child.range().start < line_end
            && child.range().end <= last.range().start
            && child.name() != "⚠"
    });
    if aligned {
        let end = if closed {
            open.start
        } else {
            open.end
                + doc.text().as_str()[open.end..]
                    .bytes()
                    .take_while(|&b| b == b' ')
                    .count()
        };
        return Some(cx.column(end, Assoc::After)?.max(0) as usize);
    }
    Some(if closed {
        base
    } else {
        base.saturating_add(cx.unit())
    })
}

#[derive(Clone, Copy)]
enum IndentRule {
    Statement,
    Tag,
    Delimited(&'static str),
}

fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_statement(name: &str) -> bool {
    matches!(
        name,
        "IfStatement"
            | "ForStatement"
            | "RawStatement"
            | "BlockStatement"
            | "MacroStatement"
            | "CallStatement"
            | "FilterStatement"
            | "SetStatement"
            | "TransStatement"
            | "WithStatement"
            | "AutoescapeStatement"
    )
}
