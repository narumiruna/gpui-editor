//! Local recovery with the pinned HTML tag context rules. This parser keeps
//! an explicit stack so incomplete or deeply nested input does not recurse.

use std::{collections::HashMap, ops::Range};

use super::{
    HtmlNodeKind::{self, *},
    HtmlOptions, Node, lexer,
};

pub(super) fn parse(source: &str, options: HtmlOptions) -> Vec<Node> {
    let mut parser = Parser {
        source,
        options,
        pos: 0,
        nodes: Vec::new(),
        elements: Vec::new(),
        active: HashMap::new(),
        foreign: 0,
        references: lexer::References::default(),
    };
    parser.add(None, Document, 0..source.len());
    if source.is_empty() {
        parser.leaf(0, Error, 0..0);
    }
    while parser.pos < source.len() {
        parser.entity();
    }
    while !parser.elements.is_empty() {
        let element = parser.elements.last().unwrap();
        parser.finish(parser.pos, !element.open_complete);
    }
    parser.nodes
}

struct ElementContext {
    id: usize,
    name: String,
    open_complete: bool,
}

struct Parser<'a> {
    source: &'a str,
    options: HtmlOptions,
    pos: usize,
    nodes: Vec<Node>,
    elements: Vec<ElementContext>,
    active: HashMap<String, usize>,
    foreign: usize,
    references: lexer::References,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.source[self.pos..]
    }

    fn parent(&self) -> usize {
        self.elements.last().map_or(0, |frame| frame.id)
    }

    fn add(&mut self, parent: Option<usize>, kind: HtmlNodeKind, range: Range<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            range,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        id
    }

    fn start(&mut self, parent: usize, kind: HtmlNodeKind) -> usize {
        self.add(Some(parent), kind, self.pos..self.pos)
    }

    fn leaf(&mut self, parent: usize, kind: HtmlNodeKind, range: Range<usize>) {
        self.add(Some(parent), kind, range);
    }

    fn consume(&mut self, parent: usize, kind: HtmlNodeKind, len: usize) {
        self.leaf(parent, kind, self.pos..self.pos + len);
        self.pos += len;
    }

    fn finish(&mut self, end: usize, error: bool) {
        let ElementContext { id, name, .. } = self.elements.pop().unwrap();
        if error {
            self.leaf(id, Error, end..end);
        }
        self.nodes[id].range.end = end;
        let count = self.active.get_mut(&name).unwrap();
        *count -= 1;
        if *count == 0 {
            self.active.remove(&name);
        }
        self.foreign -= usize::from(lexer::is_foreign(&name));
    }

    fn entity(&mut self) {
        let parent = self.parent();
        let rest = self.rest();
        if rest.starts_with('&') {
            self.reference(parent);
        } else if let Some(comment) = rest.strip_prefix("<!--") {
            let close = comment.find("-->");
            let end = close.map_or(rest.len(), |from| from + 7);
            self.consume(parent, Comment, end);
            if close.is_none() {
                let id = self.nodes.len() - 1;
                self.leaf(id, Error, self.pos..self.pos);
            }
        } else if let Some(instruction) = rest.strip_prefix("<?") {
            if let Some(close) = instruction.find("?>") {
                self.consume(parent, ProcessingInst, close + 4);
            } else {
                self.consume(parent, Error, 2);
            }
        } else if rest.starts_with("<!doctype") || rest.starts_with("<!DOCTYPE") {
            if let Some(close) = rest.find('>') {
                while !self.elements.is_empty() {
                    self.finish(self.pos, true);
                }
                self.consume(0, DoctypeDecl, close + 1);
            } else {
                self.consume(parent, Error, 2);
            }
        } else if rest.starts_with("<!") {
            self.consume(parent, Error, 2);
        } else if let Some(after) = rest.strip_prefix("</") {
            let name = lexer::context_name(after);
            if name.is_empty() {
                self.consume(parent, IncompleteCloseTag, 2);
            } else {
                self.close_tag(&name);
            }
        } else if let Some(after) = rest.strip_prefix('<') {
            let name = lexer::context_name(after);
            if name.is_empty() {
                self.consume(parent, IncompleteTag, 1);
            } else {
                self.open_tag(name);
            }
        } else {
            self.consume(
                parent,
                Text,
                lexer::prefix(rest, |ch| !matches!(ch, '<' | '&')),
            );
        }
    }

    fn reference(&mut self, parent: usize) {
        let (kind, end) = self.references.read(self.source, self.pos);
        self.consume(parent, kind, end - self.pos);
    }

    fn open_tag(&mut self, name: String) {
        while self
            .elements
            .last()
            .is_some_and(|parent| lexer::close_on_open(&parent.name, &name))
        {
            self.finish(self.pos, false);
        }
        let element = self.start(self.parent(), Element);
        let tag = self.start(element, OpenTag);
        self.consume(tag, StartTag, 1);
        self.tag_name(tag);
        let void = lexer::is_void(&name);
        let allow_self_close =
            self.options.self_closing_tags || self.foreign != 0 || lexer::is_foreign(&name);
        let (self_closed, complete) = loop {
            self.pos += lexer::space_len(self.rest());
            if self.rest().starts_with("/>") {
                self.consume(
                    tag,
                    if allow_self_close {
                        SelfClosingEndTag
                    } else {
                        EndTag
                    },
                    2,
                );
                break (allow_self_close || void, true);
            }
            if self.rest().starts_with('>') {
                self.consume(tag, EndTag, 1);
                break (void, true);
            }
            let len = lexer::attribute_name_len(self.rest());
            if len != 0 {
                self.attribute(tag, len);
            } else if let Some(ch) = self.rest().chars().next().filter(|&ch| ch != '<') {
                self.consume(tag, Error, ch.len_utf8());
            } else {
                let end = self.nodes[*self.nodes[tag].children.last().unwrap()]
                    .range
                    .end;
                self.leaf(tag, Error, end..end);
                break (void, false);
            }
        };
        self.nodes[tag].range.end = self.nodes[*self.nodes[tag].children.last().unwrap()]
            .range
            .end;
        if self_closed {
            self.nodes[tag].kind = SelfClosingTag;
            self.nodes[element].range.end = self.nodes[tag].range.end;
            return;
        }
        let raw = lexer::raw_kind(&name);
        self.foreign += usize::from(lexer::is_foreign(&name));
        *self.active.entry(name.clone()).or_default() += 1;
        self.elements.push(ElementContext {
            id: element,
            name,
            open_complete: complete,
        });
        if let Some(raw) = raw.filter(|_| complete) {
            let name = match raw {
                ScriptText => "script",
                StyleText => "style",
                _ => "textarea",
            };
            let len = lexer::raw_end(self.rest(), name);
            self.consume(element, raw, len);
            if self.pos != self.source.len() {
                self.close_tag(name);
            }
        }
    }

    fn tag_name(&mut self, parent: usize) {
        let len = lexer::name_len(self.rest());
        if len == 0 {
            self.leaf(parent, Error, self.pos..self.pos);
        } else {
            self.consume(parent, TagName, len);
        }
    }

    fn attribute(&mut self, parent: usize, len: usize) {
        let attr = self.start(parent, Attribute);
        self.consume(attr, AttributeName, len);
        let space = lexer::space_len(self.rest());
        if self.rest()[space..].starts_with('=') {
            self.pos += space;
            self.consume(attr, Is, 1);
            self.pos += lexer::space_len(self.rest());
            match self.rest().chars().next() {
                Some(quote @ ('\'' | '"')) => self.quoted(attr, quote),
                _ => {
                    let len = lexer::unquoted_len(self.rest());
                    let kind = if len == 0 {
                        Error
                    } else {
                        UnquotedAttributeValue
                    };
                    self.consume(attr, kind, len);
                }
            }
        }
        self.nodes[attr].range.end = self.nodes[*self.nodes[attr].children.last().unwrap()]
            .range
            .end;
    }

    fn quoted(&mut self, parent: usize, quote: char) {
        let value = self.start(parent, AttributeValue);
        self.pos += 1;
        loop {
            self.pos += lexer::prefix(self.rest(), |ch| ch != quote && ch != '&');
            if self.rest().starts_with('&') {
                self.reference(value);
            } else {
                if self.rest().starts_with(quote) {
                    self.pos += 1;
                } else {
                    self.leaf(value, Error, self.pos..self.pos);
                }
                break;
            }
        }
        self.nodes[value].range.end = self.pos;
    }

    fn close_tag(&mut self, name: &str) {
        while self
            .elements
            .last()
            .is_some_and(|parent| parent.name != name && lexer::implicitly_closed(&parent.name))
        {
            self.finish(self.pos, false);
        }
        if self.options.match_closing_tags && self.active.contains_key(name) {
            while self
                .elements
                .last()
                .is_some_and(|parent| parent.name != name)
            {
                self.finish(
                    self.pos,
                    !lexer::implicitly_closed(&self.elements.last().unwrap().name),
                );
            }
        }
        let matched = self
            .elements
            .last()
            .is_some_and(|parent| parent.name == name);
        let kind = if matched || !self.options.match_closing_tags {
            CloseTag
        } else {
            MismatchedCloseTag
        };
        let tag = self.start(self.parent(), kind);
        self.consume(tag, StartCloseTag, 2);
        self.tag_name(tag);
        self.pos += lexer::space_len(self.rest());
        let len = if self.rest().starts_with("/>") {
            2
        } else {
            usize::from(self.rest().starts_with('>'))
        };
        if len == 0 {
            self.leaf(tag, Error, self.pos..self.pos);
        } else {
            self.consume(tag, EndTag, len);
        }
        self.nodes[tag].range.end = self.pos;
        if matched {
            self.finish(self.pos, false);
        }
    }
}
