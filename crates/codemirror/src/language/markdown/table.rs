use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use super::{
    MarkdownNode as Node,
    MarkdownNodeKind::*,
    parser::{column, line_end, line_start},
};

/// Apply the GFM leaf parser inside a CommonMark paragraph or Setext heading.
pub(super) fn convert(
    source: &str,
    mut block: Node,
    line_head: impl Fn(usize) -> (usize, usize),
) -> (Option<Node>, Node) {
    static DELIMITER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[>\s]*\|?(\s*:?-+:?\s*\|)+(\s*:?-+:?\s*)?$").unwrap());
    let setext = matches!(block.kind, SetextHeading1 | SetextHeading2);
    let range = if setext {
        let Some(mark) = block.children.first() else {
            return (None, block);
        };
        block.range.start..line_start(source, mark.range.start) - 1
    } else if block.kind == Paragraph {
        block.range.clone()
    } else {
        return (None, block);
    };
    if !source[range.clone()].contains('|') {
        return (None, block);
    }
    let mut start = range.start;
    let mut end = line_end(source, start).min(range.end);
    let mut can_interrupt = true;
    // If the first line contains a pipe, it already starts a table candidate.
    // Even if the second line rejects it, another candidate cannot interrupt it.
    let first_has_pipe = has_pipe(&source[start..end]);
    while end < range.end {
        let line = end + 1;
        let (base, next_start) = line_head(line);
        let next_end = line_end(source, line).min(range.end);
        if can_interrupt
            && next_start <= next_end
            && matches!(source.as_bytes().get(next_start), Some(b'-' | b':' | b'|'))
            && DELIMITER.is_match(&source[next_start..next_end])
            && has_pipe(&source[start..end])
            && row(source, start..end, None) == row(source, next_start..next_end, None)
        {
            let before = (start != range.start).then(|| {
                let preceding = source[..start].rfind('\n').unwrap();
                Node::leaf(Paragraph, range.start..preceding)
            });
            // A Setext underline still takes precedence once a table has
            // started. A table start can interrupt an earlier paragraph.
            if setext {
                block.range.start = start;
                return (before, block);
            }
            let mut children = vec![
                row_node(source, TableHeader, start..end),
                Node::leaf(TableDelimiter, next_start..next_end),
            ];
            end = next_end;
            while end < range.end {
                let (_, row_start) = line_head(end + 1);
                end = line_end(source, end + 1).min(range.end);
                if row_start <= end {
                    children.push(row_node(source, TableRow, row_start..end));
                }
            }
            return (before, Node::new(Table, start..range.end, children));
        }
        if first_has_pipe {
            break;
        }
        if next_start > next_end {
            break;
        }
        can_interrupt = column(source, line, next_start) - column(source, line, base) < 4;
        start = next_start;
        end = next_end;
    }
    (None, block)
}

fn has_pipe(line: &str) -> bool {
    let mut escaped = false;
    for b in line.bytes() {
        if b == b'|' && !escaped {
            return true;
        }
        escaped = !escaped && b == b'\\';
    }
    false
}

fn row_node(source: &str, kind: super::MarkdownNodeKind, range: Range<usize>) -> Node {
    let mut children = Vec::new();
    row(source, range.clone(), Some(&mut children));
    Node::new(kind, range, children)
}

fn row(source: &str, range: Range<usize>, mut children: Option<&mut Vec<Node>>) -> usize {
    let mut count = 0;
    let mut first = true;
    let mut cell: Option<Range<usize>> = None;
    let mut escaped = false;
    for (index, byte) in source[range.clone()].bytes().enumerate() {
        let pos = range.start + index;
        if byte == b'|' && !escaped {
            count += usize::from(!first || cell.is_some());
            first = false;
            if let Some(children) = children.as_deref_mut() {
                children.extend(cell.take().map(|cell| Node::leaf(TableCell, cell)));
                children.push(Node::leaf(TableDelimiter, pos..pos + 1));
            } else {
                cell = None;
            }
        } else if escaped || !matches!(byte, b' ' | b'\t') {
            cell.get_or_insert(pos..pos).end = pos + 1;
        }
        escaped = !escaped && byte == b'\\';
    }
    if let Some(cell) = cell {
        count += 1;
        if let Some(children) = children {
            children.push(Node::leaf(TableCell, cell));
        }
    }
    count
}
