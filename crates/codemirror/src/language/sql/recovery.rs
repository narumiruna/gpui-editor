use std::ops::Range;

use super::{Node, SqlNodeKind as Kind, dialect::LexConfig, tokenizer};

/// Keep usable syntax when the strict runtime cannot finish. Missing tokens
/// are empty errors. Unexpected delimiters keep their token under an error.
pub(super) fn parse(source: &str, config: &LexConfig) -> Vec<Node> {
    let tokens = tokenizer::lex(source, config);
    let end = source.len();
    let mut tree = Recovery {
        tokens: &tokens,
        end,
        at: 0,
        nodes: vec![Node::new(Kind::Script, 0..end, None)],
        groups: Vec::new(),
        open_count: [0; 3],
        statement: None,
        last_end: 0,
    };
    tree.run();
    tree.nodes
}

struct Recovery<'a> {
    tokens: &'a [(Kind, Range<usize>)],
    end: usize,
    at: usize,
    nodes: Vec<Node>,
    groups: Vec<(usize, Kind)>,
    open_count: [usize; 3],
    statement: Option<usize>,
    last_end: usize,
}

impl Recovery<'_> {
    fn run(&mut self) {
        use Kind::*;
        while let Some((kind, range)) = self.tokens.get(self.at).cloned() {
            if kind.is_comment() {
                let next = self.next_element(self.at);
                if next == self.tokens.len() && self.groups.is_empty() {
                    self.finish_statement();
                }
                self.append_until(self.parent(), next);
                continue;
            }
            if let Some((group, closer)) = match kind {
                OpenParen => Some((Parens, CloseParen)),
                OpenBrace => Some((Braces, CloseBrace)),
                OpenBracket => Some((Brackets, CloseBracket)),
                _ => None,
            } {
                self.begin_statement(range.start);
                let id = self.push(self.parent(), group, range.clone());
                self.groups.push((id, closer));
                self.open_count[closer_index(closer).unwrap()] += 1;
                self.append(id);
            } else if let Some(index) = closer_index(kind) {
                if self.open_count[index] == 0 {
                    let error = self.push(self.parent(), Error, range);
                    self.append(error);
                } else {
                    while self
                        .groups
                        .last()
                        .is_some_and(|&(_, closer)| closer != kind)
                    {
                        self.missing_close(range.start);
                    }
                    self.append(self.parent());
                    self.end_group(range.end);
                }
            } else if kind == Semicolon {
                self.begin_statement(range.start);
                while !self.groups.is_empty() {
                    self.missing_close(range.start);
                }
                self.append(self.parent());
                self.finish_statement();
            } else if kind == Error {
                self.append(self.parent());
            } else {
                self.begin_statement(range.start);
                let parent = self.parent();
                if kind == Dot
                    || kind.is_identifier_part()
                        && self
                            .tokens
                            .get(self.next_element(self.at + 1))
                            .is_some_and(|(next, _)| *next == Dot)
                {
                    self.composite(parent, kind == Dot, range.start);
                } else {
                    self.append(parent);
                }
            }
        }
        while !self.groups.is_empty() {
            self.missing_close(self.end);
        }
        self.finish_statement();
    }

    fn next_element(&self, mut at: usize) -> usize {
        while self
            .tokens
            .get(at)
            .is_some_and(|(kind, _)| kind.is_comment())
        {
            at += 1;
        }
        at
    }

    fn composite(&mut self, parent: usize, leading_dot: bool, start: usize) {
        let id = self.push(parent, Kind::CompositeIdentifier, start..start);
        self.append(id);
        if leading_dot && !self.identifier_after_dot(id) {
            self.nodes[id].range.end = self.last_end;
            return;
        }
        let mut had_separator = false;
        loop {
            let next = self.next_element(self.at);
            if !self
                .tokens
                .get(next)
                .is_some_and(|(kind, _)| *kind == Kind::Dot)
            {
                if leading_dot && !had_separator {
                    self.missing_identifier(id, next);
                }
                break;
            }
            had_separator = true;
            self.append_until(id, next + 1);
            self.identifier_after_dot(id);
        }
        self.nodes[id].range.end = self.last_end;
    }

    fn identifier_after_dot(&mut self, id: usize) -> bool {
        let next = self.next_element(self.at);
        if self
            .tokens
            .get(next)
            .is_some_and(|(kind, _)| kind.is_identifier_part())
        {
            self.append_until(id, next + 1);
            true
        } else {
            self.missing_identifier(id, next);
            false
        }
    }

    fn missing_identifier(&mut self, id: usize, next: usize) {
        self.append_until(id, next);
        let pos = self
            .tokens
            .get(next)
            .map_or(self.end, |(_, range)| range.start);
        self.push(id, Kind::Error, pos..pos);
        self.last_end = pos;
    }

    fn parent(&self) -> usize {
        self.groups
            .last()
            .map(|&(id, _)| id)
            .or(self.statement)
            .unwrap_or(0)
    }

    fn begin_statement(&mut self, start: usize) {
        if self.statement.is_none() {
            self.statement = Some(self.push(0, Kind::Statement, start..start));
        }
    }

    fn finish_statement(&mut self) {
        if let Some(id) = self.statement.take() {
            self.nodes[id].range.end = self.last_end;
        }
    }

    fn end_group(&mut self, end: usize) {
        let (group, closer) = self.groups.pop().unwrap();
        self.open_count[closer_index(closer).unwrap()] -= 1;
        self.nodes[group].range.end = end;
    }

    fn missing_close(&mut self, pos: usize) {
        self.push(self.parent(), Kind::Error, pos..pos);
        self.last_end = pos;
        self.end_group(pos);
    }

    fn append_until(&mut self, parent: usize, limit: usize) {
        while self.at < limit {
            self.append(parent);
        }
    }

    fn append(&mut self, parent: usize) {
        let (kind, range) = self.tokens[self.at].clone();
        self.at += 1;
        self.last_end = range.end;
        self.push(parent, kind, range);
    }

    fn push(&mut self, parent: usize, kind: Kind, range: Range<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new(kind, range, Some(parent)));
        self.nodes[parent].children.push(id);
        id
    }
}

fn closer_index(kind: Kind) -> Option<usize> {
    match kind {
        Kind::CloseParen => Some(0),
        Kind::CloseBrace => Some(1),
        Kind::CloseBracket => Some(2),
        _ => None,
    }
}
