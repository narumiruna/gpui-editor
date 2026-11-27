use crate::lint::{Diagnostic, Severity};

use super::lexer::{Lexer, Token};
use super::{
    JsonNodeKind::{self, *},
    Node,
};

pub(super) fn parse(source: &str) -> (Vec<Node>, Option<Diagnostic>) {
    let parser = run(source, false);
    (parser.nodes, parser.diagnostic)
}

pub(super) fn diagnostic(source: &str) -> Option<Diagnostic> {
    run(source, true).diagnostic
}

fn run(source: &str, stop_at_error: bool) -> Parser {
    let mut parser = Parser {
        nodes: vec![Node {
            kind: JsonText,
            range: 0..source.len(),
            parent: None,
            children: Vec::new(),
        }],
        stack: vec![Frame {
            id: 0,
            phase: First,
        }],
        braces: 0,
        brackets: 0,
        diagnostic: None,
    };
    let mut tokens = Lexer::new(source).peekable();
    while let Some(token) = tokens.next() {
        let next = tokens.peek().map(|token| token.kind);
        loop {
            let consumed = parser.advance(&token, next);
            if stop_at_error && parser.diagnostic.is_some() {
                return parser;
            }
            if consumed {
                break;
            }
        }
    }
    parser.finish_document(source.len());
    parser
}

struct Parser {
    nodes: Vec<Node>,
    // Open nodes only. Store them in a flat arena so deep inputs do not
    // recurse during parsing, traversal, or destruction.
    stack: Vec<Frame>,
    braces: usize,
    brackets: usize,
    diagnostic: Option<Diagnostic>,
}

#[derive(Clone, Copy)]
struct Frame {
    id: usize,
    phase: Phase,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    First,
    Item,
    Separator,
    Colon,
    Value,
    Done,
}

use Phase::{Done, First, Item, Separator, Value};

impl Parser {
    // Return false to try the same token after inserting syntax or ending a node.
    fn advance(&mut self, token: &Token, next: Option<JsonNodeKind>) -> bool {
        let frame = self.frame();
        match self.nodes[frame.id].kind {
            JsonText => self.root(token, next, frame.phase),
            Array | Object => self.container(token, next, frame.phase),
            Property => self.property(token, next, frame.phase),
            _ => unreachable!("only compound nodes can be open"),
        }
    }

    fn root(&mut self, token: &Token, next: Option<JsonNodeKind>, phase: Phase) -> bool {
        let at = token.range.start;
        if phase == Done {
            self.report(at, "Unexpected content after JSON value");
            if token.kind != Error {
                self.missing_if_needed(at);
            }
        }
        if is_value(token.kind) {
            self.phase(Done);
            self.value(token);
        } else if matches!(token.kind, CloseBrace | CloseBracket) && next.is_none() {
            self.invalid_value(token);
            self.phase(Done);
            self.without_opener(token);
        } else {
            self.invalid_value(token);
            self.skip(token);
        }
        true
    }

    fn container(&mut self, token: &Token, next: Option<JsonNodeKind>, phase: Phase) -> bool {
        let at = token.range.start;
        let array = self.current().kind == Array;
        let closer = if array { CloseBracket } else { CloseBrace };
        let kind = token.kind;
        if kind == closer || kind == Comma && phase != First {
            if phase == Item {
                self.report(
                    at,
                    if array {
                        "Expected JSON array element"
                    } else {
                        "Expected JSON property name"
                    },
                );
                self.missing_if_needed(at);
            }
            self.token(token, kind);
            if kind == closer {
                self.finish(token.range.end);
            } else {
                self.phase(Item);
            }
        } else if self.can_close(kind) {
            self.report(at, closing_message(array));
            self.missing_if_needed(at);
            self.finish(at);
            return false;
        } else if phase == Separator {
            self.report(
                at,
                if array {
                    "Expected ',' or ']' after JSON array element"
                } else {
                    "Expected ',' or '}' after JSON property value"
                },
            );
            // Rejoin the list at an unambiguous opening delimiter or a key
            // followed by a colon. Keep other recognized tokens under errors.
            if (array && matches!(kind, OpenBrace | OpenBracket))
                || (!array && kind == String && next == Some(Colon))
            {
                self.missing(at);
                self.phase(Item);
                return false;
            }
            self.skip(token);
        } else if array {
            if is_value(kind) {
                self.phase(Separator);
                self.value(token);
            } else if kind == CloseBrace && phase == Item {
                self.invalid_value(token);
                self.phase(Separator);
                self.without_opener(token);
            } else {
                self.invalid_value(token);
                self.skip(token);
            }
        } else if kind == String || kind == Colon && next.is_some_and(is_value) {
            self.phase(Separator);
            self.open(Property, at, Phase::Colon);
            if kind == String {
                self.token(token, PropertyName);
            } else {
                self.report(at, "Expected double-quoted JSON property name");
                let key = self.add(PropertyName, at, at);
                self.push_child(key, Error, at, at);
                return false;
            }
        } else {
            self.report(at, "Expected double-quoted JSON property name");
            self.skip(token);
        }
        true
    }

    fn property(&mut self, token: &Token, next: Option<JsonNodeKind>, phase: Phase) -> bool {
        let at = token.range.start;
        let kind = token.kind;
        match phase {
            Phase::Colon if kind == Colon => {
                self.token(token, Colon);
                self.phase(Value);
            }
            Phase::Colon | Value if kind == Comma || self.can_close(kind) => {
                if phase == Phase::Colon {
                    self.report(at, "Expected ':' after JSON property name");
                } else {
                    self.invalid_value(token);
                }
                self.missing_if_needed(at);
                self.finish(at);
                return false;
            }
            Phase::Colon => {
                self.report(at, "Expected ':' after JSON property name");
                if is_value(kind) && next != Some(Colon) {
                    self.missing(at);
                    self.phase(Value);
                    return false;
                }
                self.skip(token);
            }
            Value if is_value(kind) => {
                self.phase(Done);
                self.value(token);
            }
            Value => {
                self.invalid_value(token);
                self.skip(token);
            }
            _ => unreachable!("completed properties are closed immediately"),
        }
        true
    }

    fn value(&mut self, token: &Token) {
        if let Some(issue) = token.issue {
            self.report(issue.at, issue.recovery_message());
        }
        match token.kind {
            OpenBrace => self.open(Object, token.range.start, First),
            OpenBracket => self.open(Array, token.range.start, First),
            _ => {}
        }
        self.token(token, token.kind);
        if self.current().kind == Property {
            self.finish(token.range.end);
        }
    }

    fn without_opener(&mut self, token: &Token) {
        let at = token.range.start;
        let kind = if token.kind == CloseBrace {
            Object
        } else {
            Array
        };
        self.open(kind, at, First);
        self.missing(at);
        self.token(token, token.kind);
        self.finish(token.range.end);
    }

    fn finish_document(&mut self, at: usize) {
        while self.stack.len() > 1 {
            let message = match self.current().kind {
                Property if self.frame().phase == Phase::Colon => {
                    "Expected ':' after JSON property name"
                }
                Array if self.frame().phase == Separator => closing_message(true),
                Object if self.frame().phase == Separator => closing_message(false),
                Object => "Expected JSON property name or '}'",
                _ => "Unexpected end of JSON input",
            };
            self.report(at, message);
            self.missing_if_needed(at);
            self.finish(at);
        }
        if self.current().children.is_empty() {
            self.report(at, "Expected a JSON value");
            self.missing(at);
        }
    }

    fn open(&mut self, kind: JsonNodeKind, at: usize, phase: Phase) {
        self.braces += usize::from(kind == Object);
        self.brackets += usize::from(kind == Array);
        let id = self.add(kind, at, at);
        self.stack.push(Frame { id, phase });
    }

    fn finish(&mut self, end: usize) {
        let id = self.stack.pop().expect("an open node").id;
        let node = &mut self.nodes[id];
        node.range.end = end;
        self.braces -= usize::from(node.kind == Object);
        self.brackets -= usize::from(node.kind == Array);
        if self.current().kind == Property && self.frame().phase == Done {
            let id = self.stack.pop().expect("an open property").id;
            self.nodes[id].range.end = end;
        }
    }

    fn can_close(&self, kind: JsonNodeKind) -> bool {
        matches!(kind, CloseBrace) && self.braces > 0
            || matches!(kind, CloseBracket) && self.brackets > 0
    }

    fn current(&self) -> &Node {
        &self.nodes[self.frame().id]
    }

    fn frame(&self) -> Frame {
        *self.stack.last().expect("document root is open")
    }

    fn phase(&mut self, phase: Phase) {
        self.stack.last_mut().expect("document root is open").phase = phase;
    }

    fn missing_if_needed(&mut self, at: usize) {
        if !self
            .current()
            .children
            .last()
            .is_some_and(|&id| self.nodes[id].kind == Error)
        {
            self.missing(at);
        }
    }

    fn missing(&mut self, at: usize) {
        self.add(Error, at, at);
    }

    fn skip(&mut self, token: &Token) {
        let at = token.range.start;
        let end = token.range.end;
        if token.kind == Error
            && let Some(&id) = self.current().children.last()
            && self.nodes[id].kind == Error
            && self.nodes[id].range.end == at
            && !self.nodes[id].range.is_empty()
        {
            self.nodes[id].range.end = end;
            return;
        }
        let error = self.add(Error, at, end);
        // String is a named node only after the grammar accepts the string
        // token as a value or a property name.
        if !matches!(token.kind, Error | String) {
            self.push_child(error, token.kind, at, end);
        }
    }

    fn token(&mut self, token: &Token, kind: JsonNodeKind) {
        self.add(kind, token.range.start, token.range.end);
    }

    fn add(&mut self, kind: JsonNodeKind, start: usize, end: usize) -> usize {
        self.push_child(self.frame().id, kind, start, end)
    }

    fn push_child(&mut self, parent: usize, kind: JsonNodeKind, start: usize, end: usize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            range: start..end,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    fn invalid_value(&mut self, token: &Token) {
        let (at, message) = token
            .issue
            .map_or((token.range.start, "Expected a JSON value"), |issue| {
                (issue.at, issue.recovery_message())
            });
        self.report(at, message);
    }

    fn report(&mut self, at: usize, message: &'static str) {
        if self.diagnostic.is_none() {
            self.diagnostic = Some(Diagnostic::new(at..at, Severity::Error, message));
        }
    }
}

fn is_value(kind: JsonNodeKind) -> bool {
    matches!(
        kind,
        True | False | Null | Number | String | OpenBrace | OpenBracket
    )
}

fn closing_message(array: bool) -> &'static str {
    if array {
        "Expected ']' to close JSON array"
    } else {
        "Expected '}' to close JSON object"
    }
}
