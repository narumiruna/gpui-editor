use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag};

use super::{MarkdownDialect, MarkdownNode as Node, MarkdownNodeKind::*, inline, table};

pub(super) fn parse(source: &str, dialect: MarkdownDialect) -> Node {
    let parser = Parser::new(source);
    let definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, def)| def.span.clone())
        .collect();
    let mut stack = vec![Frame::new(None, 0..source.len())];
    let mut skipped_depth = 0;
    for (event, range) in parser.into_offset_iter() {
        // Keep the native tree bounded even for generated input with enormous
        // container or delimiter depth. The enclosing source range survives.
        if skipped_depth > 0 {
            match event {
                Event::Start(_) => skipped_depth += 1,
                Event::End(_) => skipped_depth -= 1,
                _ => (),
            }
            continue;
        }
        if stack.last().is_some_and(|frame| frame.tight)
            && (block_event(&event) || matches!(event, Event::End(_)))
        {
            finish(&mut stack, source);
        }
        if matches!(
            stack.last().and_then(|frame| frame.tag.as_ref()),
            Some(Tag::Item)
        ) && !block_event(&event)
            && !matches!(event, Event::End(_))
        {
            let start = if matches!(event, Event::Text(_))
                && range.start > 0
                && source.as_bytes()[range.start - 1] == b'\\'
                && source
                    .as_bytes()
                    .get(range.start)
                    .is_some_and(u8::is_ascii_punctuation)
            {
                range.start - 1
            } else {
                range.start
            };
            let mut frame = Frame::new(Some(Tag::Paragraph), start..start);
            frame.tight = true;
            stack.push(frame);
        }
        let frame = stack.last_mut().unwrap();
        if frame.tight && !matches!(event, Event::SoftBreak | Event::HardBreak) {
            frame.range.end = range.end;
        }
        match event {
            Event::Start(_) if !block_event(&event) => {
                // The native inline scanner supplies the syntax. Retain only
                // the full range for tight-list paragraphs with no block event.
                skipped_depth = 1;
            }
            Event::Start(tag) => {
                if stack.len() < 128 {
                    stack.push(Frame::new(Some(tag), range));
                } else {
                    skipped_depth = 1;
                }
            }
            Event::End(_) => finish(&mut stack, source),
            Event::Rule => frame
                .children
                .push(Node::leaf(HorizontalRule, strip_lf(source, range))),
            Event::Text(_)
                if matches!(frame.tag, Some(Tag::CodeBlock(CodeBlockKind::Indented))) =>
            {
                let mut start = range.start;
                for part in source[range].split_inclusive('\n') {
                    let end = start + part.len();
                    let from = if part.ends_with('\n')
                        && source[line_start(source, start)..end - 1]
                            .bytes()
                            .all(|b| matches!(b, b' ' | b'\t'))
                    {
                        end - 1
                    } else {
                        start
                    };
                    push_code(&mut frame.children, from..end);
                    start = end;
                }
            }
            _ => (),
        }
    }
    if stack.last().is_some_and(|frame| frame.tight) {
        finish(&mut stack, source);
    }
    let mut root = stack.pop().unwrap().build(source);
    for range in definitions {
        if let Some(mut definition) = inline::definition(source, range.clone()) {
            let end = definition.range.end;
            definition.range.end = line_end(source, end);
            inject(&mut root, definition);
            if end < range.end {
                let start = line_end(source, end) + 1;
                if start < range.end {
                    let start = skip_indent(source, start);
                    if start < range.end {
                        inject(&mut root, Node::leaf(Paragraph, start..range.end));
                    }
                }
            }
        }
    }
    let mut marks = Vec::new();
    blocks(source, &mut root, &mut Vec::new(), &mut marks, dialect);
    marks.sort_unstable();
    marks.dedup();
    inline::scan_tree(source, &mut root, dialect, &marks);
    for mark in marks {
        inject(&mut root, Node::leaf(QuoteMark, mark..mark + 1));
    }
    root
}

fn block_event(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Rule
            | Event::Start(
                Tag::Paragraph
                    | Tag::Heading { .. }
                    | Tag::BlockQuote(_)
                    | Tag::CodeBlock(_)
                    | Tag::HtmlBlock
                    | Tag::List(_)
                    | Tag::Item
            )
    )
}

struct Frame<'a> {
    tag: Option<Tag<'a>>,
    range: Range<usize>,
    children: Vec<Node>,
    tight: bool,
}

fn finish(stack: &mut Vec<Frame<'_>>, source: &str) {
    let node = stack.pop().unwrap().build(source);
    if !node.range.is_empty() {
        stack.last_mut().unwrap().children.push(node);
    }
}

impl<'a> Frame<'a> {
    fn new(tag: Option<Tag<'a>>, range: Range<usize>) -> Self {
        Self {
            tag,
            range,
            children: Vec::new(),
            tight: false,
        }
    }

    fn build(mut self, source: &str) -> Node {
        let mut range = self.range;
        let kind = match self.tag {
            None => Document,
            Some(Tag::Paragraph) => {
                if self.tight {
                    range.end = line_end(source, range.end);
                } else {
                    range = strip_lf(source, range);
                }
                Paragraph
            }
            Some(Tag::Heading { level, .. }) => {
                range = strip_lf(source, range);
                let text = &source[range.clone()];
                let opening = text.bytes().take_while(|&b| b == b'#').count();
                let atx = opening > 0
                    && opening <= 6
                    && text
                        .as_bytes()
                        .get(opening)
                        .is_none_or(|b| matches!(b, b' ' | b'\t'));
                if atx {
                    self.children.insert(
                        0,
                        Node::leaf(HeaderMark, range.start..range.start + opening),
                    );
                    let last = skip_space_back(source, range.start + opening, range.end);
                    let start = source.as_bytes()[range.start + opening..last]
                        .iter()
                        .rposition(|&b| b != b'#')
                        .map_or(range.start + opening, |i| range.start + opening + i + 1);
                    if start < last
                        && start > range.start + opening
                        && matches!(source.as_bytes()[start - 1], b' ' | b'\t')
                    {
                        self.children.push(Node::leaf(HeaderMark, start..last));
                    }
                    match level {
                        HeadingLevel::H1 => ATXHeading1,
                        HeadingLevel::H2 => ATXHeading2,
                        HeadingLevel::H3 => ATXHeading3,
                        HeadingLevel::H4 => ATXHeading4,
                        HeadingLevel::H5 => ATXHeading5,
                        HeadingLevel::H6 => ATXHeading6,
                    }
                } else {
                    let last = line_start(source, range.end);
                    let mark = source.as_bytes()[last..range.end]
                        .iter()
                        .position(|&b| matches!(b, b'=' | b'-'))
                        .map_or(range.end, |i| last + i);
                    let end = skip_space_back(source, mark, range.end);
                    if mark < end {
                        self.children.push(Node::leaf(HeaderMark, mark..end));
                    }
                    if level == HeadingLevel::H1 {
                        SetextHeading1
                    } else {
                        SetextHeading2
                    }
                }
            }
            Some(Tag::BlockQuote(_)) => {
                range = strip_lf(source, range);
                Blockquote
            }
            Some(Tag::List(start)) => {
                range = container_range(source, range, &self.children);
                if let Some(last) = self.children.last() {
                    range.end = last.range.end;
                }
                if start.is_some() {
                    OrderedList
                } else {
                    BulletList
                }
            }
            Some(Tag::Item) => {
                range = container_range(source, range, &self.children);
                let start = skip_indent(source, range.start).min(range.end);
                let len = if source.as_bytes().get(start).is_some_and(u8::is_ascii_digit) {
                    source[start..range.end]
                        .bytes()
                        .take_while(u8::is_ascii_digit)
                        .count()
                        + 1
                } else {
                    1
                };
                if start < range.end {
                    self.children
                        .insert(0, Node::leaf(ListMark, start..(start + len).min(range.end)));
                }
                ListItem
            }
            Some(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => FencedCode,
            Some(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                range = strip_lf(source, range);
                if let Some(last) = self.children.last_mut() {
                    last.range.end = last.range.end.min(range.end);
                }
                CodeBlock
            }
            Some(Tag::HtmlBlock) => {
                let content = &source[range.clone()];
                if !unterminated_html(content) {
                    range = strip_lf(source, range);
                }
                if content.starts_with("<!--") {
                    CommentBlock
                } else if content.starts_with("<?") {
                    ProcessingInstructionBlock
                } else {
                    HTMLBlock
                }
            }
            Some(_) => Paragraph,
        };
        Node::new(kind, range, self.children)
    }
}

fn unterminated_html(source: &str) -> bool {
    if source.starts_with("<!--") {
        !source.contains("-->")
    } else if source.starts_with("<?") {
        !source.contains("?>")
    } else if source.starts_with("<![CDATA[") {
        !source.contains("]]>")
    } else if source
        .strip_prefix("<!")
        .and_then(|s| s.as_bytes().first())
        .is_some_and(u8::is_ascii_uppercase)
    {
        !source.contains('>')
    } else {
        let tags = ["script", "pre", "style", "textarea"];
        let name = source[1..]
            .split(|ch: char| ch.is_ascii_whitespace() || ch == '>')
            .next()
            .unwrap_or_default();
        tags.iter().any(|tag| name.eq_ignore_ascii_case(tag))
            && !tags.iter().any(|tag| {
                let close = format!("</{tag}>");
                source
                    .as_bytes()
                    .windows(close.len())
                    .any(|token| token.eq_ignore_ascii_case(close.as_bytes()))
            })
    }
}

fn container_range(source: &str, mut range: Range<usize>, children: &[Node]) -> Range<usize> {
    range = strip_lf(source, range);
    while let Some((line, _)) = source[range.clone()]
        .rsplit_once('\n')
        .filter(|(_, tail)| tail.bytes().all(|b| matches!(b, b' ' | b'\t')))
    {
        range.end = range.start + line.len();
    }
    if let Some(last) = children.last().filter(|child| child.kind != FencedCode) {
        range.end = range.end.max(last.range.end);
    }
    range
}

fn strip_lf(source: &str, mut range: Range<usize>) -> Range<usize> {
    if source[range.clone()].ends_with('\n') {
        range.end -= 1;
    }
    range
}

pub(super) fn line_start(source: &str, position: usize) -> usize {
    source[..position].rfind('\n').map_or(0, |i| i + 1)
}

pub(super) fn line_end(source: &str, position: usize) -> usize {
    source[position..]
        .find('\n')
        .map_or(source.len(), |i| position + i)
}

fn skip_indent(source: &str, start: usize) -> usize {
    start
        + source.as_bytes()[start..]
            .iter()
            .take_while(|&&b| matches!(b, b' ' | b'\t'))
            .count()
}

fn skip_space_back(source: &str, start: usize, end: usize) -> usize {
    end - source.as_bytes()[start..end]
        .iter()
        .rev()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\r'))
        .count()
}

fn push_code(nodes: &mut Vec<Node>, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = nodes
        .last_mut()
        .filter(|last| last.kind == CodeText && last.range.end == range.start)
    {
        last.range.end = range.end;
    } else {
        nodes.push(Node::leaf(CodeText, range));
    }
}

fn inject(parent: &mut Node, node: Node) {
    let start = node.range.start;
    let i = parent
        .children
        .partition_point(|child| child.range.end <= start);
    if let Some(child) = parent
        .children
        .get_mut(i)
        .filter(|child| child.range.start <= start && child.range.end >= node.range.end)
    {
        if child.range == node.range && !(child.kind == Blockquote && node.kind == QuoteMark) {
            return;
        }
        inject(child, node);
    } else if parent
        .children
        .get(i)
        .is_none_or(|child| node.range.end <= child.range.start)
    {
        parent.children.insert(i, node);
    }
}

#[derive(Clone, Copy)]
enum Context {
    Quote(usize),
    Item {
        start: usize,
        content: usize,
        width: usize,
    },
}

pub(super) fn column(source: &str, line: usize, to: usize) -> usize {
    source.as_bytes()[line..to].iter().fold(0, |col, &b| {
        if b == b'\t' {
            col + 4 - col % 4
        } else {
            col + 1
        }
    })
}

fn prefix(source: &str, line: usize, contexts: &[Context]) -> (usize, Option<usize>) {
    let mut pos = line;
    let mut quote = None;
    for context in contexts {
        match *context {
            Context::Quote(start) => {
                let next = if start >= line && start <= skip_indent(source, pos) {
                    start
                } else {
                    skip_indent(source, pos)
                };
                if source.as_bytes().get(next) != Some(&b'>')
                    || column(source, line, next) - column(source, line, pos) >= 4
                {
                    return (pos, None);
                }
                quote = Some(next);
                pos = next + 1;
                if matches!(source.as_bytes().get(pos), Some(b' ' | b'\t')) {
                    pos += 1;
                }
            }
            Context::Item {
                start,
                content,
                width,
            } => {
                if start >= pos && start <= skip_indent(source, pos) {
                    pos = content;
                } else {
                    let mut at = column(source, line, pos);
                    let target = at + width;
                    while at < target {
                        at += match source.as_bytes().get(pos) {
                            Some(b' ') => 1,
                            Some(b'\t') => 4 - at % 4,
                            _ => break,
                        };
                        pos += 1;
                    }
                }
                quote = None;
            }
        }
    }
    (pos, quote)
}

fn blocks(
    source: &str,
    node: &mut Node,
    contexts: &mut Vec<Context>,
    marks: &mut Vec<usize>,
    dialect: MarkdownDialect,
) {
    let context = match node.kind {
        Blockquote => Some(Context::Quote(node.range.start)),
        ListItem => node
            .children
            .first()
            .filter(|n| n.kind == ListMark)
            .map(|mark| {
                let line = line_start(source, mark.range.start);
                let after = skip_indent(source, mark.range.end);
                let pad = column(source, line, after) - column(source, line, mark.range.end);
                let content = if pad > 4 { mark.range.end + 1 } else { after };
                let width = column(source, line, content) - column(source, line, node.range.start)
                    + usize::from(pad == 0);
                Context::Item {
                    start: node.range.start,
                    content,
                    width,
                }
            }),
        _ => None,
    };
    if let Some(context) = context {
        contexts.push(context);
        if node.kind == Blockquote {
            let mut line = line_start(source, node.range.start);
            while line < node.range.end {
                if let Some(mark) = prefix(source, line, contexts)
                    .1
                    .filter(|&mark| node.range.contains(&mark))
                {
                    marks.push(mark);
                }
                line = line_end(source, line) + 1;
            }
        }
    }
    if node.kind == FencedCode {
        fenced(source, node, contexts);
    } else if matches!(
        node.kind,
        HTMLBlock | CommentBlock | ProcessingInstructionBlock
    ) && (!contexts.is_empty() || node.range.end < source.len())
    {
        node.range = strip_lf(source, node.range.clone());
    }
    if dialect == MarkdownDialect::Extended
        && node.children.iter().any(|child| {
            matches!(child.kind, Paragraph | SetextHeading1 | SetextHeading2)
                && source[child.range.clone()].contains('|')
        })
    {
        node.children = std::mem::take(&mut node.children)
            .into_iter()
            .flat_map(|child| {
                let (before, after) = table::convert(source, child, |line| {
                    let base = prefix(source, line, contexts).0;
                    (base, skip_indent(source, base))
                });
                before.into_iter().chain(std::iter::once(after))
            })
            .collect();
    }
    for child in &mut node.children {
        blocks(source, child, contexts, marks, dialect);
    }
    if let Some(last) = node.children.last() {
        node.range.end = node.range.end.max(last.range.end);
    }
    if context.is_some() {
        contexts.pop();
    }
}

fn fenced(source: &str, node: &mut Node, contexts: &[Context]) {
    let start = node.range.start;
    let fence = source.as_bytes()[start];
    let width = source[start..].bytes().take_while(|&b| b == fence).count();
    let open_end = line_end(source, start);
    let info_start = skip_indent(source, start + width).min(open_end);
    let info_end = skip_space_back(source, info_start, open_end);
    node.children
        .push(Node::leaf(CodeMark, start..start + width));
    if info_start < info_end {
        node.children
            .push(Node::leaf(CodeInfo, info_start..info_end));
    }
    let mut line = open_end + 1;
    let mut first = true;
    while line < node.range.end {
        let (base, _) = prefix(source, line, contexts);
        let end = line_end(source, base).min(node.range.end);
        let mark = skip_indent(source, base).min(end);
        let run = source.as_bytes()[mark..end]
            .iter()
            .take_while(|&&b| b == fence)
            .count();
        let is_close = run >= width
            && column(source, line, mark) - column(source, line, base) < 4
            && skip_indent(source, mark + run) == end;
        if is_close {
            if !first && node.children.last().is_none_or(|n| n.kind != CodeText) {
                push_code(&mut node.children, line - 1..line);
            }
            node.children.push(Node::leaf(CodeMark, mark..mark + run));
            node.range.end = end;
            return;
        }
        if !first {
            push_code(&mut node.children, line - 1..line);
        }
        push_code(&mut node.children, base..end);
        line = end + 1;
        first = false;
    }
    if node.range.end < source.len()
        || contexts
            .iter()
            .any(|context| matches!(context, Context::Quote(_)))
    {
        node.range = strip_lf(source, node.range.clone());
    } else if !first && source[..node.range.end].ends_with('\n') {
        push_code(&mut node.children, node.range.end - 1..node.range.end);
    }
    if node.range.end < open_end {
        node.range.end = open_end;
    }
}
