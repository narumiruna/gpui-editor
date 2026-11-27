//! CSS-specific disambiguation and recovery. Inspect each rule and declaration
//! boundary once. Reuse the pinned grammar for complete headers and values;
//! build a local tree for fragments that do not parse in isolation.

use std::ops::Range;

use super::{
    CssNodeKind,
    CssNodeKind::*,
    Node,
    lexer::{self, Kind, Token},
    lr, tokens,
};

const MAX_DEPTH: usize = 128;
const DECLARATION_PROPERTY_NAME: u16 = 99;

pub(super) fn parse(source: &str) -> Vec<Node> {
    with_root(source, StyleSheet)
}

pub(super) fn parse_inline(source: &str) -> Vec<Node> {
    with_root(source, Styles)
}

fn with_root(source: &str, kind: CssNodeKind) -> Vec<Node> {
    let mut parser = Recovery {
        source,
        tokens: lexer::lex(source),
        nodes: Vec::new(),
    };
    let root = parser.push(None, kind, 0..source.len());
    parser.items(root, 0..parser.tokens.len(), source.len(), 0);
    parser.nodes
}

struct Recovery<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    nodes: Vec<Node>,
}

impl Recovery<'_> {
    fn push(&mut self, parent: Option<usize>, kind: CssNodeKind, range: Range<usize>) -> usize {
        let id = self.nodes.len();
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        self.nodes.push(Node {
            kind,
            term: None,
            range,
            children: Vec::new(),
            parent,
        });
        id
    }

    fn token(&mut self, parent: usize, kind: CssNodeKind, i: usize) -> usize {
        self.push(Some(parent), kind, self.tokens[i].range.clone())
    }

    fn error(&mut self, parent: usize, pos: usize) {
        self.push(Some(parent), Error, pos..pos);
    }

    fn position(&self, i: usize) -> usize {
        self.tokens[i].range.start
    }

    fn last_child_end(&self, parent: usize) -> usize {
        let last = self.nodes[parent].children.last().expect("child node");
        self.nodes[*last].range.end
    }

    fn text(&self, i: usize) -> &str {
        &self.source[self.tokens[i].range.clone()]
    }

    fn items(&mut self, parent: usize, tokens: Range<usize>, limit: usize, depth: usize) {
        let mut i = tokens.start;
        if depth >= MAX_DEPTH {
            if i < tokens.end {
                self.push(Some(parent), Error, self.position(i)..limit);
            }
            return;
        }
        while i < tokens.end {
            match self.tokens[i].kind {
                Kind::Comment => {
                    self.token(parent, Comment, i);
                    i += 1;
                    continue;
                }
                Kind::Symbol(b';') => {
                    self.token(parent, Semicolon, i);
                    i += 1;
                    continue;
                }
                Kind::Symbol(b'}') => {
                    self.token(parent, Error, i);
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let keyframe = self.nodes[parent].kind == KeyframeList;
            let at_rule = self.tokens[i].kind == Kind::AtKeyword;
            let declaration_name = matches!(self.nodes[parent].kind, Block | Styles)
                && self.tokens[i].kind == Kind::Name;
            let next = declaration_name
                .then(|| (i + 1..tokens.end).find(|&j| self.tokens[j].kind != Kind::Comment))
                .flatten();
            let colon = next.filter(|&j| self.tokens[j].kind == Kind::Symbol(b':'));
            let custom = declaration_name && self.text(i).starts_with("--");
            let stop = self.boundary(i, tokens.end, custom && colon.is_some());
            let open = stop < tokens.end && self.tokens[stop].kind == Kind::Symbol(b'{');
            let semicolon = stop < tokens.end && self.tokens[stop].kind == Kind::Symbol(b';');
            let boundary = if stop < tokens.end {
                self.position(stop)
            } else {
                limit
            };
            if let Some(colon) = colon.filter(|_| !open) {
                self.declaration(parent, i, colon, stop, semicolon, boundary);
            } else if custom
                && next.unwrap_or(tokens.end) == stop
                && (semicolon || stop == tokens.end)
            {
                // With no value or selector tokens, recover only the missing colon.
                let decl = self.push(Some(parent), Declaration, self.position(i)..boundary);
                self.token(decl, VariableName, i);
                for j in i + 1..stop {
                    self.token(decl, Comment, j);
                }
                self.error(decl, boundary);
            } else if i < stop {
                let end = self.tokens[stop - 1].range.end;
                let has_end = at_rule && semicolon;
                let rule = if keyframe {
                    self.keyframe_header(parent, i..stop);
                    parent
                } else {
                    self.header(parent, i..stop, end, has_end)
                };
                if open {
                    let kind = if self.nodes[rule].kind == KeyframesStatement {
                        KeyframeList
                    } else {
                        Block
                    };
                    let (block, next) = self.block(rule, kind, stop, tokens.end, limit, depth + 1);
                    if rule != parent {
                        self.nodes[rule].range.end = self.nodes[block].range.end;
                    }
                    i = next;
                    continue;
                }
                if !has_end {
                    self.error(rule, end);
                }
            } else if open {
                // A stray opening brace still starts a recoverable block.
                let (_, next) = self.block(parent, Block, stop, tokens.end, limit, depth + 1);
                i = next;
                continue;
            }
            if semicolon {
                if !at_rule {
                    self.token(parent, Semicolon, stop);
                }
                i = stop + 1;
            } else {
                i = stop;
            }
        }
    }

    fn boundary(&self, mut i: usize, end: usize, custom: bool) -> usize {
        while i < end {
            let token = &self.tokens[i];
            match token.kind {
                Kind::Symbol(b'{') if !custom => break,
                Kind::Symbol(b';' | b'}') => break,
                Kind::Symbol(b'(' | b'[' | b'{') => {
                    i = token.close.filter(|&close| close < end).unwrap_or(i);
                }
                _ => {}
            }
            i += 1;
        }
        i
    }

    fn block(
        &mut self,
        parent: usize,
        kind: CssNodeKind,
        i: usize,
        end: usize,
        limit: usize,
        depth: usize,
    ) -> (usize, usize) {
        let close = self.tokens[i].close.filter(|&close| close < end);
        let content_end = close.map_or(limit, |j| self.position(j));
        let block = self.push(
            Some(parent),
            kind,
            self.position(i)..close.map_or(limit, |j| self.tokens[j].range.end),
        );
        self.token(block, OpenBrace, i);
        self.items(block, i + 1..close.unwrap_or(end), content_end, depth);
        if let Some(close) = close {
            self.token(block, CloseBrace, close);
            (block, close + 1)
        } else {
            self.error(block, limit);
            (block, end)
        }
    }

    fn header(
        &mut self,
        parent: usize,
        indices: Range<usize>,
        end: usize,
        semicolon: bool,
    ) -> usize {
        let start = self.position(indices.start);
        let stop = indices.end;
        let actual_end = if semicolon {
            self.tokens[stop].range.end
        } else {
            end
        };
        let mut fragment = self.source[start..actual_end].to_owned();
        if !semicolon {
            fragment.push_str("{}");
        }
        if let Ok(nodes) = lr::parse(&fragment)
            && let [id] = nodes[0].children.as_slice()
            && (semicolon
                || nodes[*id].children.last().is_some_and(|&id| {
                    matches!(nodes[id].kind, Block | KeyframeList)
                        && nodes[id].range.start == end - start
                }))
        {
            let node = &nodes[*id];
            let result = self.push(Some(parent), node.kind, start..actual_end);
            self.nodes[result].term = node.term;
            let n = node.children.len() - usize::from(!semicolon);
            self.copy(&nodes, &node.children[..n], result, start, 0);
            return result;
        }
        let i = indices.start;
        let (kind, keyword) = at_rule(self.text(i));
        let node = self.push(Some(parent), kind, start..actual_end);
        if let Some(keyword) = keyword {
            self.token(node, keyword, i);
            if kind == KeyframesStatement || kind == NamespaceStatement {
                let name = if kind == KeyframesStatement {
                    KeyframeName
                } else {
                    NamespaceName
                };
                if i + 1 < indices.end {
                    self.token(node, name, i + 1);
                    self.values(node, i + 2..indices.end, end, 0);
                }
            } else {
                self.values(node, i + 1..indices.end, end, 0);
            }
        } else {
            self.selectors(node, indices);
        }
        if semicolon {
            self.token(node, Semicolon, stop);
        }
        node
    }

    fn keyframe_header(&mut self, parent: usize, indices: Range<usize>) {
        let mut i = indices.start;
        while i < indices.end {
            match self.tokens[i].kind {
                Kind::Symbol(b',') => {
                    self.token(parent, Comma, i);
                }
                Kind::Comment => {
                    self.token(parent, Comment, i);
                }
                Kind::Name => {
                    let selector = self.token(parent, KeyframeSelector, i);
                    self.token(selector, KeyframeRangeName, i);
                    if i + 1 < indices.end && matches!(self.tokens[i + 1].kind, Kind::Number { .. })
                    {
                        i += 1;
                        self.number(selector, i);
                        self.nodes[selector].range.end = self.tokens[i].range.end;
                    }
                }
                Kind::Number { .. } => {
                    let selector = self.token(parent, KeyframeSelector, i);
                    self.number(selector, i);
                }
                _ => {
                    self.token(parent, Error, i);
                }
            }
            i += 1;
        }
    }

    fn declaration(
        &mut self,
        parent: usize,
        start: usize,
        colon: usize,
        end: usize,
        semicolon: bool,
        boundary: usize,
    ) {
        // The property/selector GLR fork happens before the colon. Parse the
        // value in the Styles entry with an unambiguous custom-property prefix,
        // discard that synthetic prefix, and use the real property and colon.
        const PREFIX: &str = "--p:";
        let after_colon = self.tokens[colon].range.end;
        let text_end = self.tokens[end - 1].range.end;
        let fragment_end = if semicolon {
            self.tokens[end].range.end
        } else {
            text_end
        };
        let fragment = format!("{PREFIX}{}", &self.source[after_colon..fragment_end]);
        let property = if self.text(start).starts_with("--") {
            VariableName
        } else {
            PropertyName
        };
        let parsed = lr::parse_styles(&fragment).ok();
        let parsed = parsed.as_deref().and_then(|nodes| {
            let id = *nodes[0].children.first()?;
            let decl = &nodes[id];
            (decl.kind == Declaration
                && decl.children.len() >= 2
                && nodes[0].children[1..]
                    .iter()
                    .all(|&id| matches!(nodes[id].kind, Comment | Semicolon)))
            .then_some((nodes, decl))
        });
        let to = parsed.map_or(text_end, |(_, decl)| {
            after_colon + (decl.range.end - PREFIX.len())
        });
        let declaration = self.push(Some(parent), Declaration, self.position(start)..to);
        let property_node = self.token(declaration, property, start);
        if property == PropertyName {
            self.nodes[property_node].term = Some(DECLARATION_PROPERTY_NAME);
        }
        for i in start + 1..colon {
            self.token(declaration, Comment, i);
        }
        self.token(declaration, Colon, colon);
        if let Some((nodes, decl)) = parsed {
            self.copy(
                nodes,
                &decl.children[2..],
                declaration,
                after_colon,
                PREFIX.len(),
            );
            for &id in &nodes[0].children[1..] {
                if nodes[id].kind == Comment {
                    self.copy(nodes, &[id], parent, after_colon, PREFIX.len());
                }
            }
        } else {
            self.values(declaration, colon + 1..end, boundary, 0);
            self.nodes[declaration].range.end = self.last_child_end(declaration);
        }
    }

    fn copy(&mut self, nodes: &[Node], children: &[usize], parent: usize, to: usize, from: usize) {
        let mut stack: Vec<_> = children.iter().rev().map(|&id| (id, parent)).collect();
        while let Some((id, parent)) = stack.pop() {
            let node = &nodes[id];
            let id = self.push(
                Some(parent),
                node.kind,
                (node.range.start - from + to)..(node.range.end - from + to),
            );
            self.nodes[id].term = node.term;
            stack.extend(node.children.iter().rev().map(|&id_child| (id_child, id)));
        }
    }

    fn selectors(&mut self, parent: usize, indices: Range<usize>) {
        let mut i = indices.start;
        while i < indices.end {
            match self.tokens[i].kind {
                Kind::Name => {
                    let selector = self.token(parent, TagSelector, i);
                    self.token(selector, TagName, i);
                }
                Kind::Hash => {
                    let selector = self.token(parent, IdSelector, i);
                    self.token(selector, IdName, i);
                }
                Kind::Symbol(symbol @ (b'.' | b':')) => {
                    let (selector, name, punct) = if symbol == b'.' {
                        (ClassSelector, ClassName, Dot)
                    } else {
                        (PseudoClassSelector, PseudoClassName, Colon)
                    };
                    let node = self.token(parent, selector, i);
                    self.token(node, punct, i);
                    if i + 1 < indices.end && self.tokens[i + 1].kind == Kind::Name {
                        i += 1;
                        self.token(node, name, i);
                        self.nodes[node].range.end = self.tokens[i].range.end;
                    } else {
                        self.error(node, self.tokens[i].range.end);
                    }
                }
                Kind::Symbol(b'[') => {
                    let end = self.tokens[i].close.filter(|&end| end < indices.end);
                    let stop = end.unwrap_or(indices.end - 1);
                    let node = self.push(
                        Some(parent),
                        AttributeSelector,
                        self.position(i)..self.tokens[stop].range.end,
                    );
                    self.token(node, OpenBracket, i);
                    i += 1;
                    if i <= stop && self.tokens[i].kind == Kind::Name {
                        self.token(node, AttributeName, i);
                        i += 1;
                    }
                    let interior = stop + usize::from(end.is_none());
                    if i < interior {
                        let limit = self.tokens[interior - 1].range.end;
                        self.values(node, i..interior, limit, 0);
                    }
                    if let Some(end) = end {
                        self.token(node, CloseBracket, end);
                    } else {
                        self.error(node, self.tokens[stop].range.end);
                    }
                    i = stop;
                }
                kind => {
                    let name = match kind {
                        Kind::Comment => Comment,
                        Kind::Symbol(b'*') => UniversalSelector,
                        Kind::Symbol(b'&') => NestingSelector,
                        Kind::Symbol(b'>') => ChildOp,
                        Kind::Symbol(b'+' | b'~') => SiblingOp,
                        Kind::Symbol(b',') => Comma,
                        Kind::Symbol(b'|') => Pipe,
                        _ => Error,
                    };
                    self.token(parent, name, i);
                }
            }
            i += 1;
        }
    }

    fn number(&mut self, parent: usize, i: usize) {
        let Kind::Number { number_end } = self.tokens[i].kind else {
            unreachable!()
        };
        let number = self.token(parent, NumberLiteral, i);
        let end = self.tokens[i].range.end;
        if number_end < end {
            self.push(Some(number), Unit, number_end..end);
        }
    }

    fn values(&mut self, parent: usize, indices: Range<usize>, limit: usize, depth: usize) {
        let mut i = indices.start;
        if depth >= MAX_DEPTH && i < indices.end {
            self.push(
                Some(parent),
                Error,
                self.position(i)..self.tokens[indices.end - 1].range.end,
            );
            return;
        }
        while i < indices.end {
            let token = &self.tokens[i];
            let kind = match token.kind {
                Kind::Number { .. } => {
                    self.number(parent, i);
                    i += 1;
                    continue;
                }
                Kind::Name => {
                    if i + 1 < indices.end
                        && self.tokens[i + 1].kind == Kind::Symbol(b'(')
                        && token.range.end == self.position(i + 1)
                    {
                        let raw = lexer::raw_function(self.text(i));
                        let nodes = if raw {
                            (CallLiteral, CallTag)
                        } else {
                            (CallExpression, Callee)
                        };
                        let node = self.token(parent, nodes.0, i);
                        self.token(node, nodes.1, i);
                        let container = self.group(
                            node,
                            i + 1,
                            indices.end,
                            if raw { None } else { Some(ArgList) },
                            limit,
                            depth,
                        );
                        self.nodes[node].range.end = self.last_child_end(node);
                        i = container + 1;
                        continue;
                    }
                    if self.text(i).starts_with("--") {
                        VariableName
                    } else {
                        ValueName
                    }
                }
                Kind::String => StringLiteral,
                Kind::UrlContent => ParenthesizedContent,
                Kind::Hash if tokens::is_color(self.text(i)) => ColorLiteral,
                Kind::Comment => Comment,
                Kind::AtKeyword => AtKeyword,
                Kind::Symbol(symbol @ (b'(' | b'[' | b'{')) => {
                    let node = match symbol {
                        b'(' => ParenthesizedValue,
                        b'[' => BracketedValue,
                        _ => BracedValue,
                    };
                    i = self.group(parent, i, indices.end, Some(node), limit, depth) + 1;
                    continue;
                }
                Kind::Symbol(b'!')
                    if i + 1 < indices.end
                        && self.text(i + 1).eq_ignore_ascii_case("important") =>
                {
                    self.push(
                        Some(parent),
                        Important,
                        token.range.start..self.tokens[i + 1].range.end,
                    );
                    i += 2;
                    continue;
                }
                Kind::Symbol(b',') => Comma,
                Kind::Symbol(b'.') => Dot,
                Kind::Symbol(b':') => Colon,
                Kind::Symbol(b';') => Semicolon,
                Kind::Symbol(b'+' | b'-' | b'*' | b'/') => BinOp,
                _ => Error,
            };
            self.token(parent, kind, i);
            i += 1;
        }
    }

    fn group(
        &mut self,
        parent: usize,
        i: usize,
        end: usize,
        kind: Option<CssNodeKind>,
        limit: usize,
        depth: usize,
    ) -> usize {
        let close = self.tokens[i].close.filter(|&close| close < end);
        let stop = close.map_or(end, |close| close + 1);
        let range = self.position(i)..self.tokens[stop - 1].range.end;
        let (open_kind, close_kind) = match self.tokens[i].kind {
            Kind::Symbol(b'[') => (OpenBracket, CloseBracket),
            Kind::Symbol(b'{') => (OpenBrace, CloseBrace),
            _ => (OpenParen, CloseParen),
        };
        let node = kind.map_or(parent, |kind| self.push(Some(parent), kind, range));
        self.token(node, open_kind, i);
        let content_end = close.map_or(limit, |close| self.position(close));
        let mut start = i + 1;
        if let Some(close) = close
            && kind == Some(ArgList)
            && depth + 1 < MAX_DEPTH
            && self.text(i - 1) == "var"
            && let Some(comma) = self.first_var_dash(node, start, close)
        {
            start = comma;
        }
        self.values(node, start..close.unwrap_or(end), content_end, depth + 1);
        if let Some(close) = close {
            self.token(node, close_kind, close);
        } else {
            // Keep whitespace before the recovery boundary in an unfinished
            // argument list, without taking the next declaration's delimiter.
            let end = if kind == Some(ArgList) {
                limit
            } else {
                self.last_child_end(node)
            };
            self.nodes[node].range.end = end;
            self.error(node, end);
        }
        stop - 1
    }

    fn first_var_dash(&mut self, parent: usize, start: usize, close: usize) -> Option<usize> {
        let dash = (start..close).find(|&i| self.tokens[i].kind != Kind::Comment)?;
        if self.tokens[dash].kind != Kind::Symbol(b'-') {
            return None;
        }
        let comma = (dash + 1..close).find(|&i| self.tokens[i].kind != Kind::Comment)?;
        if self.tokens[comma].kind != Kind::Symbol(b',') {
            return None;
        }
        // Lezer's incomplete binary expression ends at the comma, with
        // comments after the dash inside it. The comma stays in the ArgList.
        for i in start..dash {
            self.token(parent, Comment, i);
        }
        let from = self.position(dash);
        let to = self.position(comma);
        let expression = self.push(Some(parent), BinaryExpression, from..to);
        self.error(expression, from);
        self.token(expression, BinOp, dash);
        for i in dash + 1..comma {
            self.token(expression, Comment, i);
        }
        self.error(expression, to);
        Some(comma)
    }
}

fn at_rule(text: &str) -> (CssNodeKind, Option<CssNodeKind>) {
    let (statement, keyword) = match text {
        "@import" => (ImportStatement, Import),
        "@media" => (MediaStatement, Media),
        "@charset" => (CharsetStatement, Charset),
        "@namespace" => (NamespaceStatement, Namespace),
        "@keyframes" => (KeyframesStatement, Keyframes),
        "@supports" => (SupportsStatement, Supports),
        "@scope" => (ScopeStatement, Scope),
        "@font-feature-values" => (FontFeatureStatement, FontFeatureValues),
        _ if text.starts_with('@') => (AtRule, AtKeyword),
        _ => return (RuleSet, None),
    };
    (statement, Some(keyword))
}
