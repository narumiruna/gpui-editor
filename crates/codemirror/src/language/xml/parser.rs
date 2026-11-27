use std::collections::HashMap;

use super::{
    Node,
    XmlNodeKind::{self, *},
};

pub(super) fn parse(source: &str) -> Vec<Node> {
    let mut parser = Parser {
        source,
        pos: 0,
        last_gt: source.rfind('>'),
        nodes: Vec::new(),
        open: Vec::new(),
        active_names: HashMap::new(),
    };
    parser.add(None, Document, 0, source.len());
    while parser.pos < source.len() {
        parser.entity();
    }
    while !parser.open.is_empty() {
        parser.finish_open(Error);
    }
    if source.is_empty() {
        parser.missing(0);
    }
    parser.nodes
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
    last_gt: Option<usize>,
    nodes: Vec<Node>,
    open: Vec<(usize, &'a str)>,
    active_names: HashMap<&'a str, usize>,
}

impl<'a> Parser<'a> {
    fn add(&mut self, parent: Option<usize>, kind: XmlNodeKind, start: usize, end: usize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            range: start..end,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        id
    }

    fn begin(&mut self, parent: usize, kind: XmlNodeKind) -> usize {
        self.add(Some(parent), kind, self.pos, self.pos)
    }

    fn finish(&mut self, id: usize) {
        self.nodes[id].range.end = self.pos;
    }

    fn token(&mut self, parent: usize, kind: XmlNodeKind, len: usize) {
        self.add(Some(parent), kind, self.pos, self.pos + len);
        self.pos += len;
    }

    fn missing(&mut self, parent: usize) {
        self.token(parent, Error, 0);
    }

    fn rest(&self) -> &'a str {
        &self.source[self.pos..]
    }

    fn skip_space(&mut self) {
        self.pos += self.rest().len() - trim_space(self.rest()).len();
    }

    fn entity(&mut self) {
        let parent = self.open.last().map_or(0, |&(id, _)| id);
        let rest = self.rest();
        match rest.as_bytes()[0] {
            b'&' => self.reference(parent),
            b'<' if rest.starts_with("</") => self.close_tag(parent),
            b'<' if rest.starts_with("<!--") => self.delimited(parent, Comment, 4, "-->"),
            b'<' if rest.starts_with("<?") => self.delimited(parent, ProcessingInst, 2, "?>"),
            b'<' if rest.starts_with("<![CDATA[") || rest.starts_with("<![cdata[") => {
                self.delimited(parent, Cdata, 9, "]]>");
            }
            b'<' if rest.starts_with("<!") => {
                // Do not rescan an unterminated suffix for each declaration.
                let end = self.last_gt.is_some_and(|last| self.pos < last)
                    && (rest.starts_with("<!DOCTYPE") || rest.starts_with("<!doctype"));
                let end = end.then(|| rest.find('>').unwrap());
                if let Some(end) = end {
                    // This production is only allowed at the document level.
                    while !self.open.is_empty() {
                        self.finish_open(Error);
                    }
                    self.token(0, DoctypeDecl, end + 1);
                } else {
                    self.token(parent, Error, 1);
                }
            }
            b'<' => self.open_tag(parent),
            _ => {
                let len = rest.find(['<', '&']).unwrap_or(rest.len());
                self.token(parent, Text, len);
            }
        }
    }

    fn delimited(&mut self, parent: usize, kind: XmlNodeKind, prefix: usize, end: &str) {
        let rest = self.rest();
        if let Some(offset) = rest[prefix..].find(end) {
            self.token(parent, kind, prefix + offset + end.len());
        } else {
            let node = self.begin(parent, kind);
            self.pos = self.source.len();
            self.missing(node);
            self.finish(node);
        }
    }

    fn open_tag(&mut self, parent: usize) {
        let element = self.begin(parent, Element);
        let tag = self.begin(element, OpenTag);
        let name = context_name(&self.rest()[1..]);
        self.token(tag, StartTag, 1);
        self.skip_space();
        let mut len = name_len(self.rest());
        if len == 0
            && let Some(skip) = stray_before_self_close(self.rest())
        {
            self.token(tag, Error, skip);
            self.skip_space();
            len = name_len(self.rest());
        }
        if len > 0 {
            self.token(tag, TagName, len);
            self.skip_space();
            while name_len(self.rest()) > 0 {
                self.attribute(tag);
                self.skip_space();
            }
            let rest = self.rest();
            if !rest.starts_with(['>', '<'])
                && let Some(ch) = rest.chars().next()
                && trim_space(&rest[ch.len_utf8()..]).starts_with("/>")
            {
                self.token(tag, Error, ch.len_utf8());
                self.skip_space();
            }
        } else {
            self.missing(tag);
        }
        let rest = self.rest();
        let self_closed = rest.starts_with("/>");
        // A bad suffix on a bare top-level start tag can recover as an
        // unfinished self-closing tag instead of swallowing trailing text.
        let recover_self_close = len > 0
            && parent == 0
            && self.nodes[tag].children.len() == 2
            && !rest.is_empty()
            && !rest.starts_with(['>', '&'])
            && rest.find(['<', '&', '=', '\'', '"']).is_none();
        if self_closed {
            self.token(tag, SelfCloseEndTag, 2);
        } else if len > 0 && rest.starts_with('>') {
            self.token(tag, EndTag, 1);
        } else if len > 0 {
            self.missing(tag);
        }
        self.finish(tag);
        if self_closed || recover_self_close {
            self.nodes[tag].kind = SelfClosingTag;
            self.finish(element);
        } else {
            self.open.push((element, name));
            *self.active_names.entry(name).or_default() += 1;
        }
    }

    fn attribute(&mut self, tag: usize) {
        let attr = self.begin(tag, Attribute);
        self.token(attr, AttributeName, name_len(self.rest()));
        self.skip_space();
        if self.rest().starts_with('=') {
            self.token(attr, Is, 1);
            self.skip_space();
            if let Some(quote @ ('\'' | '"')) = self.rest().chars().next() {
                self.attribute_value(attr, quote);
                self.finish(attr);
                return;
            }
        }
        self.missing(attr);
        self.finish(attr);
    }

    fn attribute_value(&mut self, attr: usize, quote: char) {
        let value = self.begin(attr, AttributeValue);
        self.pos += 1;
        loop {
            let rest = self.rest();
            match rest.find([quote, '&']) {
                Some(offset) if rest.as_bytes()[offset] == b'&' => {
                    self.pos += offset;
                    self.reference(value);
                }
                Some(offset) => {
                    self.pos += offset + 1;
                    break;
                }
                None => {
                    self.pos = self.source.len();
                    self.missing(value);
                    break;
                }
            }
        }
        self.finish(value);
    }

    fn reference(&mut self, parent: usize) {
        let rest = &self.rest()[1..];
        let (kind, len) = if let Some(number) = rest.strip_prefix('#') {
            let (prefix, digits) = match number.strip_prefix('x') {
                Some(hex) => (2, hex.bytes().take_while(u8::is_ascii_hexdigit).count()),
                None => (1, number.bytes().take_while(u8::is_ascii_digit).count()),
            };
            (
                CharacterReference,
                if digits == 0 { 0 } else { prefix + digits },
            )
        } else {
            (EntityReference, name_len(rest))
        };
        if len > 0 && rest.as_bytes().get(len) == Some(&b';') {
            self.token(parent, kind, len + 2);
        } else {
            self.token(parent, Error, 1);
        }
    }

    fn close_tag(&mut self, parent: usize) {
        let name = context_name(&self.rest()[2..]);
        if name.is_empty() {
            self.token(parent, StartCloseTag, 2);
            return;
        }
        let (parent, kind) = if self.active_names.contains_key(name) {
            while let Some(&(_, current)) = self.open.last() {
                if current == name {
                    break;
                }
                self.finish_open(MissingCloseTag);
            }
            (self.open.last().unwrap().0, CloseTag)
        } else {
            (parent, MismatchedCloseTag)
        };
        let tag = self.begin(parent, kind);
        self.token(tag, StartCloseTag, 2);
        self.skip_space();
        let len = name_len(self.rest());
        if len > 0 {
            self.token(tag, TagName, len);
            self.skip_space();
        }
        if len > 0 && self.rest().starts_with('>') {
            self.token(tag, EndTag, 1);
        } else {
            self.missing(tag);
        }
        self.finish(tag);
        if kind == CloseTag {
            self.pop_open();
        }
    }

    fn finish_open(&mut self, marker: XmlNodeKind) {
        self.token(self.open.last().unwrap().0, marker, 0);
        self.pop_open();
    }

    fn pop_open(&mut self) {
        let (element, name) = self.open.pop().unwrap();
        let count = self.active_names.get_mut(name).unwrap();
        *count -= 1;
        if *count == 0 {
            self.active_names.remove(name);
        }
        self.finish(element);
    }
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches([' ', '\t', '\r', '\n'])
}

// The upstream tag-matching tokenizer uses a looser class than the grammar
// and stops at ASCII digits. Keep it separate from the visible TagName.
fn context_name(text: &str) -> &str {
    let text = trim_space(text);
    let end = text.find(|ch: char| {
        !(ch.is_ascii_alphabetic() || matches!(ch, '-' | '.' | ':' | '_') || ch >= '\u{a1}')
    });
    &text[..end.unwrap_or(text.len())]
}

fn name_start(ch: char) -> bool {
    matches!(ch, ':' | 'A'..='Z' | '_' | 'a'..='z' |
        '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' |
        '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' |
        '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' |
        '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}

fn name_char(ch: char) -> bool {
    name_start(ch)
        || matches!(ch, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

fn name_len(text: &str) -> usize {
    if text.starts_with(name_start) {
        text.find(|ch| !name_char(ch)).unwrap_or(text.len())
    } else {
        0
    }
}

fn stray_before_self_close(text: &str) -> Option<usize> {
    let skip = text.chars().next()?.len_utf8();
    let rest = trim_space(&text[skip..]);
    let name = name_len(rest);
    (trim_space(&rest[name..]).starts_with("/>") && !text.starts_with("/>")).then_some(skip)
}
