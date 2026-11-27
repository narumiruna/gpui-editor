use std::{borrow::Cow, ops::Range, sync::LazyLock};

use regex::Regex;

use super::{MarkdownDialect, MarkdownNode as Node, MarkdownNodeKind::*};

mod links;
pub(super) use links::definition;

// Keep recursive tree queries bounded on input with unusually deep markup.
const MAX_INLINE_DEPTH: u8 = 96;

pub(super) fn scan_tree(source: &str, node: &mut Node, dialect: MarkdownDialect, quotes: &[usize]) {
    let range = node.range.clone();
    let start = range.start;
    let end = range.end;
    let body = if matches!(
        node.kind,
        ATXHeading1 | ATXHeading2 | ATXHeading3 | ATXHeading4 | ATXHeading5 | ATXHeading6
    ) {
        let first = node.children[0].range.end;
        let last = node.children.get(1).map_or(end, |mark| mark.range.start);
        Some((first + 1).min(last)..last)
    } else if matches!(node.kind, SetextHeading1 | SetextHeading2) {
        Some(start..super::parser::line_start(source, node.children[0].range.start) - 1)
    } else if matches!(node.kind, Paragraph | TableCell) {
        Some(range)
    } else if node.kind == Task {
        Some(start + 3..end)
    } else {
        None
    };
    if let Some(body) = body {
        let quotes = &quotes[quotes.partition_point(|&pos| pos < body.start)
            ..quotes.partition_point(|&pos| pos < body.end)];
        let mut input = Cow::Borrowed(&source[body.clone()]);
        if !quotes.is_empty() {
            let mut scrubbed = input.as_bytes().to_vec();
            for quote in quotes {
                scrubbed[*quote - body.start] = b' ';
            }
            input = Cow::Owned(String::from_utf8(scrubbed).expect("quote marks are ASCII"));
        }
        let children = parse(&input, dialect);
        let at = node
            .children
            .partition_point(|child| child.range.end <= body.start);
        node.children.splice(
            at..at,
            children.into_iter().map(|mut child| {
                child.shift(body.start as isize);
                child
            }),
        );
    } else {
        for child in &mut node.children {
            if dialect == MarkdownDialect::Extended
                && node.kind == ListItem
                && child.kind == Paragraph
                && matches!(
                    source
                        .as_bytes()
                        .get(child.range.start..child.range.start + 4),
                    Some([b'[', b' ' | b'x' | b'X', b']', b' ' | b'\t'])
                )
            {
                child.kind = Task;
                let at = child.range.start;
                child.children.push(Node::leaf(TaskMarker, at..at + 3));
            }
            scan_tree(source, child, dialect, quotes);
        }
    }
}

fn parse(source: &str, dialect: MarkdownDialect) -> Vec<Node> {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^&(?:#[0-9]+|#[xX][a-fA-F0-9]+|[a-zA-Z_0-9]+);").unwrap());
    static EMOJI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^:[a-zA-Z_0-9]+:").unwrap());
    let extended = dialect == MarkdownDialect::Extended;
    let bytes = source.as_bytes();
    let mut parts = Vec::new();
    let mut brackets = Vec::new();
    let mut last_link_start = 0;
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let width = source[at..].chars().next().unwrap().len_utf8();
        let token = match byte {
            b'\\' if bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) => {
                Some(Node::leaf(Escape, at..at + 2))
            }
            b'\\' if bytes.get(at + 1) == Some(&b'\n') => Some(Node::leaf(HardBreak, at..at + 2)),
            b'&' => ENTITY
                .find(&source[at..])
                .filter(|m| m.end() <= 31)
                .map(|m| Node::leaf(Entity, at..at + m.end())),
            b'`' if at == 0 || bytes[at - 1] != b'`' => code(source, at),
            b'<' => html(source, at),
            b'*' | b'_' => {
                let end = run(bytes, at, byte);
                parts.push(Part::Delimiter(delimiter(source, byte, at..end)));
                at = end;
                continue;
            }
            b'~' if extended
                && bytes.get(at + 1) == Some(&b'~')
                && bytes.get(at + 2) != Some(&b'~') =>
            {
                parts.push(Part::Delimiter(delimiter(source, byte, at..at + 2)));
                at += 2;
                continue;
            }
            b' ' => {
                let end = run(bytes, at, byte);
                if end >= at + 2 && bytes.get(end) == Some(&b'\n') {
                    Some(Node::leaf(HardBreak, at..end + 1))
                } else {
                    at = end;
                    continue;
                }
            }
            b'[' | b'!' if byte == b'[' || bytes.get(at + 1) == Some(&b'[') => {
                let end = at + if byte == b'[' { 1 } else { 2 };
                brackets.push(parts.len());
                parts.push(Part::Delimiter(Delimiter {
                    byte,
                    range: at..end,
                    open: true,
                    close: false,
                    size: 0,
                }));
                at = end;
                continue;
            }
            b']' => {
                if let Some(index) = brackets.pop() {
                    let Part::Delimiter(open) = &parts[index] else {
                        unreachable!()
                    };
                    let start = open.range.start;
                    let content = open.range.end;
                    let kind = if open.byte == b'[' { Link } else { Image };
                    if (kind == Link && start < last_link_start)
                        || (skip_space(source, content) == at
                            && !matches!(bytes.get(at + 1), Some(b'(' | b'[')))
                    {
                        parts[index] = Part::Empty;
                    } else {
                        let (mut children, depth) = resolve(parts.drain(index + 1..));
                        parts.pop();
                        if depth == MAX_INLINE_DEPTH {
                            children.clear();
                        }
                        children.insert(0, Node::leaf(LinkMark, start..content));
                        children.push(Node::leaf(LinkMark, at..at + 1));
                        at = links::finish(source, at + 1, &mut children);
                        if kind == Link {
                            // A link invalidates all earlier open links, but not images.
                            last_link_start = start;
                        }
                        parts.push(Part::Node(
                            Node::new(kind, start..at, children),
                            (depth + 1).min(MAX_INLINE_DEPTH),
                        ));
                        continue;
                    }
                }
                None
            }
            b'^' | b'~' if extended => script(source, at, byte),
            b':' if extended => EMOJI
                .find(&source[at..])
                .map(|m| Node::leaf(Emoji, at..at + m.end())),
            b if extended && (b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-')) => {
                links::bare_url(source, at, !brackets.is_empty())
                    .map(|end| Node::leaf(URL, at..end))
            }
            _ => None,
        };
        if let Some(token) = token {
            at = token.range.end;
            parts.push(Part::Node(token, 1));
        } else {
            at += width;
        }
    }
    resolve(parts).0
}

struct Delimiter {
    byte: u8,
    range: Range<usize>,
    open: bool,
    close: bool,
    size: usize,
}

enum Part {
    Empty,
    Node(Node, u8),
    Delimiter(Delimiter),
}

fn resolve(parts: impl IntoIterator<Item = Part>) -> (Vec<Node>, u8) {
    // Index unmatched openers by character, original length modulo three,
    // and whether they can also close. Only the latest in each class matters.
    let mut opens: [[Vec<usize>; 6]; 3] = std::array::from_fn(|_| Default::default());
    let mut active = Vec::new();
    for part in parts {
        let mut close = match part {
            Part::Delimiter(delimiter) => delimiter,
            Part::Node(_, _) => {
                active.push(part);
                continue;
            }
            Part::Empty => continue,
        };
        let group = match close.byte {
            b'*' => 0,
            b'_' => 1,
            b'~' => 2,
            _ => continue,
        };
        let emphasis = close.byte != b'~';
        loop {
            let opening = if close.close {
                opens[group]
                    .iter()
                    .enumerate()
                    .filter_map(|(class, stack)| {
                        let size = class % 3;
                        let valid = !emphasis
                            || !(close.open || class >= 3)
                            || (size + close.size) % 3 != 0
                            || (size == 0 && close.size % 3 == 0);
                        valid.then(|| stack.last().copied()).flatten()
                    })
                    .max()
            } else {
                None
            };
            let Some(index) = opening else {
                if close.open {
                    let class = close.size % 3 + 3 * usize::from(close.close);
                    opens[group][class].push(active.len());
                    active.push(Part::Delimiter(close));
                }
                break;
            };
            let (mut children, depth) = take_nodes(active.drain(index + 1..));
            if depth == MAX_INLINE_DEPTH {
                children.clear();
            }
            let Some(Part::Delimiter(open)) = active.last_mut() else {
                unreachable!()
            };
            let size = 2.min(open.range.len()).min(close.range.len());
            let (kind, mark) = match (emphasis, size) {
                (false, _) => (Strikethrough, StrikethroughMark),
                (true, 1) => (Emphasis, EmphasisMark),
                (true, _) => (StrongEmphasis, EmphasisMark),
            };
            let start = open.range.end - size;
            let end = close.range.start + size;
            children.insert(0, Node::leaf(mark, start..open.range.end));
            children.push(Node::leaf(mark, close.range.start..end));
            open.range.end = start;
            if open.range.is_empty() {
                active.pop();
            }
            for stack in opens.iter_mut().flatten() {
                while stack.last().is_some_and(|&i| i >= active.len()) {
                    stack.pop();
                }
            }
            active.push(Part::Node(
                Node::new(kind, start..end, children),
                (depth + 1).min(MAX_INLINE_DEPTH),
            ));
            if end == close.range.end {
                break;
            }
            close.range.start = end;
        }
    }
    take_nodes(active)
}

fn take_nodes(parts: impl IntoIterator<Item = Part>) -> (Vec<Node>, u8) {
    let mut children = Vec::new();
    let mut depth = 0;
    for part in parts {
        if let Part::Node(node, node_depth) = part {
            depth = depth.max(node_depth);
            children.push(node);
        }
    }
    (children, depth)
}

fn delimiter(source: &str, byte: u8, range: Range<usize>) -> Delimiter {
    static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{P}\p{S}]").unwrap());
    let properties = |ch: Option<char>| {
        let mut buf = [0; 4];
        (
            ch.is_none_or(js_space),
            ch.is_some_and(|ch| {
                ch.len_utf16() == 1 && PUNCTUATION.is_match(ch.encode_utf8(&mut buf))
            }),
        )
    };
    let (space_before, punct_before) = properties(source[..range.start].chars().next_back());
    let (space_after, punct_after) = properties(source[range.end..].chars().next());
    let left = !space_after && (!punct_after || space_before || punct_before);
    let right = !space_before && (!punct_before || space_after || punct_after);
    Delimiter {
        byte,
        size: range.len(),
        range,
        open: left && (byte != b'_' || !right || punct_before),
        close: right && (byte != b'_' || !left || punct_after),
    }
}

fn js_space(ch: char) -> bool {
    matches!(ch, '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

fn code(source: &str, start: usize) -> Option<Node> {
    let bytes = source.as_bytes();
    let open_end = run(bytes, start, b'`');
    let mut from = open_end;
    while let Some(next) = source[from..].find('`') {
        let close = from + next;
        let end = run(bytes, close, b'`');
        if end - close == open_end - start {
            return Some(Node::new(
                InlineCode,
                start..end,
                vec![
                    Node::leaf(CodeMark, start..open_end),
                    Node::leaf(CodeMark, close..end),
                ],
            ));
        }
        from = end;
    }
    None
}

fn html(source: &str, start: usize) -> Option<Node> {
    static AUTO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:[a-zA-Z][a-zA-Z_0-9+.-]+:[^\s>]+|[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*)>").unwrap()
    });
    static COMMENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^!--(?:-?>|(?s:.*?)-->)").unwrap());
    static INSTRUCTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\?(?s:.*?)\?>").unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^(?:![A-Z](?s:.*?)>|!\[CDATA\[(?s:.*?)\]\]>|/[a-zA-Z][a-zA-Z0-9_-]*\s*>|[a-zA-Z][a-zA-Z0-9_-]*(?:\s+[a-zA-Z:_][a-zA-Z0-9_.:-]*(?:\s*=\s*(?:[^\s"'=<>`]+|'[^']*'|"[^"]*"))?)*\s*/?>)"#).unwrap()
    });
    let after = &source[start + 1..];
    if let Some(m) = AUTO.find(after) {
        let end = start + 1 + m.end();
        return Some(Node::new(
            Autolink,
            start..end,
            vec![
                Node::leaf(LinkMark, start..start + 1),
                Node::leaf(URL, start + 1..end - 1),
                Node::leaf(LinkMark, end - 1..end),
            ],
        ));
    }
    for (pattern, kind) in [
        (&*COMMENT, Comment),
        (&*INSTRUCTION, ProcessingInstruction),
        (&*TAG, HTMLTag),
    ] {
        if let Some(m) = pattern.find(after) {
            return Some(Node::leaf(kind, start..start + 1 + m.end()));
        }
    }
    None
}

fn script(source: &str, start: usize, byte: u8) -> Option<Node> {
    let bytes = source.as_bytes();
    if bytes.get(start + 1) == Some(&byte) {
        return None;
    }
    let (kind, mark) = if byte == b'~' {
        (Subscript, SubscriptMark)
    } else {
        (Superscript, SuperscriptMark)
    };
    let mut children = vec![Node::leaf(mark, start..start + 1)];
    let mut at = start + 1;
    while let Some(&next) = bytes.get(at) {
        if next == byte {
            children.push(Node::leaf(mark, at..at + 1));
            return Some(Node::new(kind, start..at + 1, children));
        }
        if inline_space(next) {
            return None;
        }
        if next == b'\\' {
            let width = source[at + 1..].chars().next()?.len_utf8();
            children.push(Node::leaf(Escape, at..at + 1 + width));
            at += width;
        }
        at += 1;
    }
    None
}

fn run(bytes: &[u8], at: usize, byte: u8) -> usize {
    at + bytes[at..].iter().take_while(|&&b| b == byte).count()
}

fn inline_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn skip_space(source: &str, from: usize) -> usize {
    from + source.as_bytes()[from..]
        .iter()
        .take_while(|&&b| inline_space(b))
        .count()
}
