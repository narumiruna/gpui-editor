use super::{YamlNode as Node, YamlNodeKind::*, lexer::Source};

// Bound recursion on untrusted source. At the limit, advance with an error node.
const MAX_DEPTH: usize = 128;

pub(super) fn parse(source: &str) -> Node {
    Source(source).stream()
}

#[derive(Clone, Copy)]
enum Mode {
    Block {
        from: usize,
        indent: Option<usize>,
        indentless_sequence: bool,
    },
    BlockKey,
    Flow,
}

#[derive(Clone, Copy)]
enum Block {
    Sequence,
    Explicit,
    Mapping(usize),
}

impl Source<'_> {
    fn stream(&self) -> Node {
        let mut at = 0;
        let mut children = Vec::new();
        while !self.eof(at) {
            let next = self.skip(at);
            if self.eof(next) || self.doc_mark(next, "...") {
                self.comments(at, next, &mut children);
                if self.eof(next) {
                    break;
                }
                at = next + 3;
                children.push(Node::leaf(DocEnd, next..at));
                continue;
            }
            let mut doc = Vec::new();
            let mut start = at;
            let mut pos = next;
            if self.byte(pos) == b'%' {
                self.comments(at, pos, &mut children);
                start = pos;
                while self.byte(pos) == b'%' {
                    self.comments(start, pos, &mut doc);
                    let directive = self.directive(pos);
                    start = directive.range.end;
                    doc.push(directive);
                    pos = self.skip(start);
                }
            }
            if self.doc_mark(pos, "---") {
                self.comments(
                    start,
                    pos,
                    if doc.is_empty() {
                        &mut children
                    } else {
                        &mut doc
                    },
                );
                start = pos + 3;
                doc.push(Node::leaf(DirectiveEnd, pos..start));
            }
            let next = self.skip(start);
            if !self.eof(next) && !self.doc_boundary(next) && self.byte(next) != b'%' {
                let mode = Mode::Block {
                    from: start,
                    indent: None,
                    indentless_sequence: false,
                };
                let node = self
                    .value(next, mode, 0)
                    .unwrap_or_else(|| self.error(next));
                if node.range.start == next {
                    let leading = if doc.is_empty() {
                        &mut children
                    } else {
                        &mut doc
                    };
                    self.comments(start, next, leading);
                }
                doc.push(node);
            }
            let start = doc.first().map_or(next, |node| node.range.start);
            at = doc.last().map_or(next, |node| node.range.end);
            // Recover non-separated top-level values as further documents.
            if !doc.is_empty() {
                children.push(Node::new(Document, start..at, doc));
            }
        }
        Node::new(Stream, 0..self.0.len(), children)
    }

    fn skip(&self, mut at: usize) -> usize {
        loop {
            let next = self.inline_space(at);
            at = match self.byte(next) {
                b'#' => self.line_end(next),
                b'\r' | b'\n' => self.after_break(next),
                _ => return next,
            };
        }
    }

    fn comments(&self, mut at: usize, to: usize, out: &mut Vec<Node>) {
        while at < to {
            if self.byte(at) == b'#' {
                let end = self.line_end(at);
                out.push(Node::leaf(Comment, at..end));
                at = end;
            } else {
                at = self.next(at);
            }
        }
    }

    fn doc_boundary(&self, at: usize) -> bool {
        self.doc_mark(at, "---") || self.doc_mark(at, "...")
    }

    fn directive(&self, at: usize) -> Node {
        let end = self.line_end(at);
        let name_end = self.0.as_bytes()[at..end]
            .iter()
            .position(|&ch| matches!(ch, b' ' | b'\t'))
            .map_or(end, |offset| at + offset);
        let content = self.inline_space(name_end);
        let comment = self.0.as_bytes()[content..end]
            .iter()
            .enumerate()
            .find(|&(offset, &ch)| {
                ch == b'#' && (offset == 0 || self.byte(content + offset - 1) == b' ')
            })
            .map_or(end, |(offset, _)| content + offset);
        let content_end = content + self.0[content..comment].trim_end_matches([' ', '\t']).len();
        let mut children = vec![Node::leaf(DirectiveName, at..name_end)];
        if content < content_end {
            children.push(Node::leaf(DirectiveContent, content..content_end));
        }
        if comment < end {
            children.push(Node::leaf(Comment, comment..end));
        }
        Node::new(Directive, at..self.after_break(end), children)
    }

    fn block_kind(&self, at: usize) -> Option<Block> {
        if self.mark(at, b'-') {
            Some(Block::Sequence)
        } else if self.mark(at, b'?') {
            Some(Block::Explicit)
        } else {
            self.key_colon(at).map(Block::Mapping)
        }
    }

    fn value(&self, at: usize, mode: Mode, depth: usize) -> Option<Node> {
        if self.eof(at) {
            return None;
        }
        if depth >= MAX_DEPTH {
            return Some(self.error(at));
        }
        if let Mode::Block { from, indent, .. } = mode {
            if let Some(kind) = self.block_kind(at) {
                return Some(match kind {
                    Block::Sequence => self.sequence(at, depth + 1),
                    Block::Explicit => self.mapping(at, at, kind, depth + 1),
                    Block::Mapping(_) => self.mapping(from, at, kind, depth + 1),
                });
            }
            if matches!(self.byte(at), b'|' | b'>') {
                return Some(self.block_literal(at, indent));
            }
        }
        let node = match self.byte(at) {
            b'[' | b'{' => self.flow(at, depth + 1),
            b'\'' | b'"' => Node::leaf(QuotedLiteral, at..self.quoted(at, false)?),
            b'*' => Node::leaf(Alias, at..self.anchor(at)),
            b'!' | b'&' => {
                let tag = self.byte(at) == b'!';
                let end = if tag { self.tag(at) } else { self.anchor(at) };
                let mut children = vec![Node::leaf(if tag { Tag } else { Anchor }, at..end)];
                if let Mode::Block {
                    indent,
                    indentless_sequence,
                    ..
                } = mode
                {
                    self.block_value(&mut children, end, indent, indentless_sequence, depth + 1);
                } else {
                    let next = if matches!(mode, Mode::BlockKey) {
                        self.inline_space(end)
                    } else {
                        self.skip(end)
                    };
                    if let Some(node) = self.value(next, mode, depth + 1) {
                        self.comments(end, next, &mut children);
                        children.push(node);
                    }
                }
                Node::ending(if tag { Tagged } else { Anchored }, at, children)
            }
            _ => {
                let end = match mode {
                    Mode::Block { indent, .. } => self.plain(at, false, false, indent)?,
                    Mode::BlockKey => self.plain(at, true, false, None)?,
                    Mode::Flow => self.plain(at, false, true, None)?,
                };
                Node::leaf(Literal, at..end)
            }
        };
        Some(node)
    }

    fn block_value(
        &self,
        out: &mut Vec<Node>,
        from: usize,
        indent: Option<usize>,
        indentless_sequence: bool,
        depth: usize,
    ) -> usize {
        let at = self.skip(from);
        if self.eof(at) || self.doc_boundary(at) {
            return from;
        }
        let line_start = self.line_start(at);
        let allowed = from >= line_start
            || indent.is_none_or(|indent| {
                let column = self.column(at);
                column > indent || column == indent && indentless_sequence && self.mark(at, b'-')
            });
        if !allowed {
            return from;
        }
        let mode = Mode::Block {
            from,
            indent,
            indentless_sequence,
        };
        let node = self
            .value(at, mode, depth)
            .unwrap_or_else(|| self.error(at));
        if node.range.start == at {
            self.comments(from, at, out);
        }
        let end = node.range.end;
        out.push(node);
        end
    }

    fn block_literal(&self, at: usize, indent: Option<usize>) -> Node {
        let mut pos = at + 1;
        let mut explicit = None;
        let mut chomp = false;
        for _ in 0..2 {
            match self.byte(pos) {
                b'1'..=b'9' if explicit.is_none() => {
                    let n = usize::from(self.byte(pos) - b'0');
                    explicit = Some(indent.map_or(n - 1, |col| col + n));
                }
                b'+' | b'-' if !chomp => chomp = true,
                _ => break,
            }
            pos += 1;
        }
        let mut children = vec![Node::leaf(BlockLiteralHeader, at..pos)];
        pos = self.inline_space(pos);
        if self.byte(pos) == b'#' {
            let end = self.line_end(pos);
            children.push(Node::leaf(Comment, pos..end));
            pos = end;
        }
        let content = self.after_break(pos);
        if content > pos {
            let end = self.block_content(content, indent.map_or(0, |col| col + 1), explicit);
            children.push(Node::leaf(BlockLiteralContent, content..end));
        } else if !self.eof(pos) {
            children.push(Node::leaf(Error, pos..self.line_end(pos)));
        }
        Node::ending(BlockLiteral, at, children)
    }

    fn sequence(&self, start: usize, depth: usize) -> Node {
        let indent = self.column(start);
        let mut children = Vec::new();
        let mut at = start;
        loop {
            let mark_end = at + 1;
            children.push(Node::leaf(Dash, at..mark_end));
            let mut item = Vec::new();
            let end = self.block_value(&mut item, mark_end, Some(indent), false, depth);
            at = self.skip(end);
            let another = self.mark(at, b'-') && self.column(at) == indent;
            let item_end = if item.is_empty() && (another || self.eof(at)) {
                at
            } else {
                end
            };
            let leading = item.iter().take_while(|node| node.kind == Comment).count();
            children.extend(item.drain(..leading));
            if item.is_empty() {
                self.comments(end, item_end, &mut children);
            }
            let item_start = item.first().map_or(item_end, |node| node.range.start);
            children.push(Node::new(Item, item_start..item_end, item));
            if !another {
                break;
            }
            self.comments(item_end, at, &mut children);
        }
        Node::ending(BlockSequence, start, children)
    }

    fn mapping(&self, from: usize, mut at: usize, mut kind: Block, depth: usize) -> Node {
        let indent = self.column(at);
        let mut children = Vec::new();
        self.comments(from, at, &mut children);
        loop {
            let mut pair = Vec::new();
            match kind {
                Block::Explicit => {
                    children.push(Node::leaf(Question, at..at + 1));
                    let mut key = Vec::new();
                    let after = self.inline_space(at + 1);
                    let end = if self.mark(after, b':') {
                        at + 1
                    } else {
                        self.block_value(&mut key, at + 1, Some(indent), false, depth)
                    };
                    if let Some(node) = key.pop() {
                        children.extend(key);
                        pair.push(Node::wrapping(Key, node));
                    }
                    let colon = self.skip(end);
                    if self.mark(colon, b':')
                        && (self.line_start(colon) == self.line_start(end)
                            || self.column(colon) == indent)
                    {
                        self.comments(end, colon, &mut pair);
                        pair.push(Node::leaf(Colon, colon..colon + 1));
                        self.block_value(&mut pair, colon + 1, Some(indent), true, depth);
                    }
                    at = pair.first().map_or(end, |node| node.range.start);
                }
                Block::Mapping(colon) => {
                    if at < colon
                        && let Some(key) = self.value(at, Mode::BlockKey, depth)
                        && key.range.end <= colon
                    {
                        pair.push(Node::wrapping(Key, key));
                    }
                    pair.push(Node::leaf(Colon, colon..colon + 1));
                    self.block_value(&mut pair, colon + 1, Some(indent), true, depth);
                }
                Block::Sequence => unreachable!(),
            }
            let pair = Node::ending(Pair, at, pair);
            let end = pair.range.end;
            children.push(pair);
            at = self.skip(end);
            if self.eof(at) || self.doc_boundary(at) || self.column(at) != indent {
                break;
            }
            match self.block_kind(at) {
                Some(next @ (Block::Explicit | Block::Mapping(_))) => kind = next,
                _ => break,
            }
            self.comments(end, at, &mut children);
        }
        Node::ending(BlockMapping, from, children)
    }

    fn flow(&self, start: usize, depth: usize) -> Node {
        let seq = self.byte(start) == b'[';
        let (open, close, closer) = if seq {
            (OpenBracket, CloseBracket, b']')
        } else {
            (OpenBrace, CloseBrace, b'}')
        };
        let mut children = vec![Node::leaf(open, start..start + 1)];
        let mut end = start + 1;
        let mut needs_value = true;
        loop {
            let at = self.skip(end);
            self.comments(end, at, &mut children);
            match self.byte(at) {
                ch if ch == closer => {
                    children.push(Node::leaf(close, at..at + 1));
                    break;
                }
                b']' | b'}' => {
                    children.push(Node::leaf(Error, at..at));
                    break;
                }
                b',' => {
                    if needs_value {
                        children.push(Node::leaf(Error, at..at));
                    }
                    end = at + 1;
                    children.push(Node::leaf(Comma, at..end));
                    needs_value = true;
                }
                _ if self.eof(at) => {
                    children.push(Node::leaf(Error, at..at));
                    break;
                }
                _ => {
                    if !needs_value {
                        children.push(Node::leaf(Error, at..at));
                    }
                    end = self.flow_entry(at, seq, &mut children, depth);
                    needs_value = false;
                }
            }
        }
        Node::ending(
            if seq { FlowSequence } else { FlowMapping },
            start,
            children,
        )
    }

    fn flow_entry(&self, start: usize, seq: bool, out: &mut Vec<Node>, depth: usize) -> usize {
        let explicit = self.mark(start, b'?');
        let mut leading = Vec::new();
        let mut at = start;
        if explicit {
            leading.push(Node::leaf(Question, at..at + 1));
            at = self.skip(at + 1);
            self.comments(start + 1, at, &mut leading);
        }
        let mut pair = Vec::new();
        let left = self.value(at, Mode::Flow, depth);
        let left_end = left.as_ref().map_or(at, |node| node.range.end);
        let colon = self.skip(left_end);
        let has_colon = self.byte(colon) == b':';
        if let Some(left) = left {
            if seq && !explicit && !has_colon {
                out.push(Node::wrapping(Item, left));
                return left_end;
            }
            pair.push(Node::wrapping(Key, left));
        } else if !has_colon
            && !(explicit && (self.eof(at) || matches!(self.byte(at), b',' | b']' | b'}')))
        {
            let error = self.error(at);
            let end = error.range.end;
            out.extend(leading);
            out.push(error);
            return end;
        }
        if has_colon {
            self.comments(left_end, colon, &mut pair);
            pair.push(Node::leaf(Colon, colon..colon + 1));
            let next = self.skip(colon + 1);
            if let Some(node) = self.value(next, Mode::Flow, depth) {
                self.comments(colon + 1, next, &mut pair);
                pair.push(if !seq && !explicit && left_end == at {
                    Node::wrapping(Key, node)
                } else {
                    node
                });
            }
        }
        if seq {
            leading.push(Node::ending(Pair, at, pair));
            let entry = Node::wrapping(Item, Node::ending(FlowMapping, start, leading));
            let end = entry.range.end;
            out.push(entry);
            end
        } else {
            out.extend(leading);
            let node = Node::ending(Pair, at, pair);
            let end = node.range.end;
            out.push(node);
            end
        }
    }

    fn error(&self, at: usize) -> Node {
        let range = at..self.next(at);
        let kind = match self.byte(at) {
            b',' => Comma,
            b':' => Colon,
            b']' => CloseBracket,
            b'}' => CloseBrace,
            _ => return Node::leaf(Error, range),
        };
        Node::wrapping(Error, Node::leaf(kind, range))
    }
}
