use std::ops::Range;

use super::{
    LezerNodeKind::{self as Kind, *},
    Node,
    lexer::{Lexeme, TokenKind, scan, symbol},
};

mod declaration;
mod expression;

// Bound recursive parsing of nested inline rules, parentheses and arguments.
const MAX_NESTING: usize = 64;

pub(super) fn parse(source: &str) -> Vec<Node> {
    let (tokens, comments) = scan(source);
    let mut parser = Parser {
        source,
        tokens,
        cursor: 0,
        closers: Vec::new(),
        roots: Vec::new(),
        nodes: Vec::new(),
    };
    while !parser.eof() {
        parser.declaration();
    }
    let id = parser.nodes.len();
    parser.nodes.push(Node {
        kind: Grammar,
        term: None,
        range: 0..source.len(),
        parent: None,
        children: parser.roots,
    });
    preorder(parser.nodes, id, comments)
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Lexeme>,
    cursor: usize,
    closers: Vec<char>,
    roots: Vec<usize>,
    nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
struct Mark {
    cursor: usize,
    root: usize,
}

impl Parser<'_> {
    fn peek(&self) -> TokenKind {
        self.tokens[self.cursor].kind
    }

    fn at(&self, ch: char) -> bool {
        self.peek() == symbol(ch)
    }

    fn at_node(&self, kind: Kind) -> bool {
        self.peek() == TokenKind::Node(kind)
    }

    fn eof(&self) -> bool {
        self.peek() == TokenKind::Eof
    }

    fn boundary(&self) -> bool {
        self.eof() || self.closers.iter().rev().any(|&close| self.at(close))
    }

    fn current_range(&self) -> Range<usize> {
        let token = self.tokens[self.cursor];
        token.from..token.to
    }

    fn mark(&self) -> Mark {
        Mark {
            cursor: self.cursor,
            root: self.roots.len(),
        }
    }

    fn finish(&mut self, mark: Mark, kind: Kind) {
        let children = self.roots.split_off(mark.root);
        let start = self.tokens[mark.cursor].from;
        let consumed = if self.cursor > mark.cursor {
            self.tokens[self.cursor - 1].to
        } else {
            start
        };
        let end = children
            .last()
            .map_or(consumed, |&id| consumed.max(self.nodes[id].range.end));
        self.push(kind, start..end, children);
    }

    fn push(&mut self, kind: Kind, range: Range<usize>, children: Vec<usize>) {
        self.roots.push(self.nodes.len());
        self.nodes.push(Node {
            kind,
            term: None,
            range,
            children,
            parent: None,
        });
    }

    fn bump(&mut self, kind: Option<Kind>) {
        debug_assert!(!self.eof());
        if let Some(kind) = kind {
            self.push(kind, self.current_range(), Vec::new());
        }
        self.cursor += 1;
    }

    fn take(&mut self, ch: char) -> bool {
        if self.at(ch) {
            self.bump(match self.peek() {
                TokenKind::Node(kind) => Some(kind),
                _ => None,
            });
            true
        } else {
            false
        }
    }

    fn take_node(&mut self, kind: Kind) -> bool {
        if !self.at_node(kind) {
            return false;
        }
        self.bump(Some(kind));
        true
    }

    fn take_name(&mut self, kind: Kind) -> bool {
        if self.peek() != TokenKind::Word {
            return false;
        }
        self.bump(Some(kind));
        true
    }

    fn keyword(&mut self, kind: Kind) -> bool {
        if self.peek() != TokenKind::Word || &self.source[self.current_range()] != kind.name() {
            return false;
        }
        self.bump(Some(kind));
        true
    }

    fn require(&mut self, matched: impl FnOnce(&mut Self) -> bool) {
        if !matched(self) {
            self.missing();
        }
    }

    fn missing(&mut self) {
        let from = self.tokens[self.cursor].from;
        self.push(Error, from..from, Vec::new());
    }

    fn unexpected(&mut self) {
        let start = self.mark();
        let kind = match self.peek() {
            TokenKind::Eof => return self.missing(),
            TokenKind::Node(kind) => Some(kind),
            _ => None,
        };
        self.bump(kind);
        if self.tokens[start.cursor].kind == TokenKind::Invalid {
            while self.peek() == TokenKind::Invalid
                && self.tokens[self.cursor - 1].to == self.tokens[self.cursor].from
            {
                self.bump(None);
            }
        }
        self.finish(start, Error);
    }

    fn delimited(&mut self, kind: Kind, open: char, close: char, content: impl FnOnce(&mut Self)) {
        let mark = self.mark();
        if self.at(open) && self.closers.len() == MAX_NESTING {
            self.skip_group(open, close);
            self.finish(mark, Error);
            return;
        }
        if !self.take(open) {
            self.missing();
            // Do not take a closer that belongs to a containing node.
            if self.at(close) && !self.closers.contains(&close) {
                self.take(close);
                self.finish(mark, kind);
            }
            return;
        }
        self.closers.push(close);
        content(self);
        self.closers.pop();
        self.require(|p| p.take(close));
        self.finish(mark, kind);
    }

    fn skip_group(&mut self, open: char, close: char) {
        self.bump(None);
        let mut depth = 1;
        while !self.eof() && depth > 0 {
            if self.at(open) {
                depth += 1;
            } else if self.at(close) {
                depth -= 1;
            }
            self.bump(None);
        }
    }
}

// The parser skips comments. Insert them into the innermost node that spans
// them while converting the post-order nodes to pre-order. Both inputs advance
// in source order, so a long comment list does not multiply the tree depth.
fn preorder(mut parsed: Vec<Node>, root: usize, comments: Vec<Lexeme>) -> Vec<Node> {
    let mut result = Vec::with_capacity(parsed.len() + comments.len());
    let mut comments = comments.into_iter().peekable();
    let mut stack = Vec::new();
    let enter = |result: &mut Vec<Node>, stack: &mut Vec<_>, node: &mut Node, parent| {
        let children = std::mem::take(&mut node.children).into_iter().peekable();
        stack.push((result.len(), children));
        result.push(Node {
            kind: node.kind,
            term: node.term,
            range: node.range.clone(),
            parent,
            children: Vec::new(),
        });
    };
    enter(&mut result, &mut stack, &mut parsed[root], None);
    while let Some((parent, children)) = stack.last_mut() {
        let parent = *parent;
        let id = result.len();
        let limit = children
            .peek()
            .map_or(result[parent].range.end, |&child| parsed[child].range.start);
        if let Some(comment) = comments.next_if(|comment| comment.to <= limit) {
            let TokenKind::Node(kind) = comment.kind else {
                unreachable!()
            };
            result[parent].children.push(id);
            result.push(Node {
                kind,
                term: None,
                range: comment.from..comment.to,
                parent: Some(parent),
                children: Vec::new(),
            });
        } else if let Some(child) = children.next() {
            result[parent].children.push(id);
            enter(&mut result, &mut stack, &mut parsed[child], Some(parent));
        } else {
            stack.pop();
        }
    }
    result
}
