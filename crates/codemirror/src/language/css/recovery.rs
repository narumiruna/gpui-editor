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
const MAX_ATTRIBUTE_HEADER_BYTES: usize = 4096;
const DECLARATION_PROPERTY_NAME: u16 = 99;

pub(super) fn parse(source: &str) -> Vec<Node> {
    with_root(source, lr::Mode::Sheet)
}

pub(super) fn parse_inline(source: &str) -> Vec<Node> {
    with_root(source, lr::Mode::Styles)
}

fn with_root(source: &str, mode: lr::Mode) -> Vec<Node> {
    let mut parser = Recovery {
        source,
        tokens: lexer::lex(source),
        nodes: Vec::new(),
        missing_semicolon: false,
        unfinished_group: false,
        stray_closers: Vec::new(),
    };
    let kind = match mode {
        lr::Mode::Sheet => StyleSheet,
        lr::Mode::Styles => Styles,
    };
    let root = parser.push(None, kind, 0..source.len());
    parser.items(root, 0..parser.tokens.len(), source.len(), 0);
    // The grammar can prefer a selector or a missing semicolon. Let its
    // recovery choose, but keep local boundaries for unfinished value groups.
    let recover_separator = parser.missing_semicolon
        && !parser.unfinished_group
        && !parser
            .tokens
            .iter()
            .any(|token| matches!(token.kind, Kind::Symbol(b'(' | b'[')) && token.close.is_none());
    if (recover_separator || parser.only_stray_closers(mode))
        && let Ok(nodes) = lr::recover(source, mode)
    {
        return nodes;
    }
    parser.nodes
}

struct Recovery<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    nodes: Vec<Node>,
    missing_semicolon: bool,
    unfinished_group: bool,
    stray_closers: Vec<usize>,
}

impl Recovery<'_> {
    fn only_stray_closers(&self, mode: lr::Mode) -> bool {
        if self.stray_closers.is_empty() || self.unfinished_group {
            return false;
        }
        // Use grammar recovery only if the rest of the input parses strictly.
        // Spaces keep the tokens on either side of each closer separate.
        let mut source = self.source.to_owned();
        for &i in &self.stray_closers {
            source.replace_range(self.tokens[i].range.clone(), " ");
        }
        lr::accepts_strict(&source, mode)
    }

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

    fn last_noncomment_kind(&self, parent: usize) -> Option<CssNodeKind> {
        self.nodes[parent]
            .children
            .iter()
            .rev()
            .map(|&id| self.nodes[id].kind)
            .find(|&kind| kind != Comment)
    }

    // Return true if a skipped block item is the last emitted child.
    fn items(&mut self, parent: usize, tokens: Range<usize>, limit: usize, depth: usize) -> bool {
        let mut i = tokens.start;
        if depth >= MAX_DEPTH {
            if i < tokens.end {
                self.push(Some(parent), Error, self.position(i)..limit);
            }
            return false;
        }
        let mut saw_declaration = false;
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
                Kind::Symbol(symbol @ (b')' | b']'))
                    if i == tokens.start && self.nodes[parent].kind == Styles =>
                {
                    // Skip one leading closer before a valid declaration list.
                    // Leave malformed suffixes to the existing recovery path.
                    let end = self.tokens[i].range.end;
                    if let Ok(nodes) = lr::parse_styles(&self.source[end..limit])
                        && nodes[0]
                            .children
                            .iter()
                            .map(|&id| nodes[id].kind)
                            .find(|&kind| kind != Comment)
                            == Some(Declaration)
                    {
                        let error = self.token(parent, Error, i);
                        let closer = if symbol == b')' {
                            CloseParen
                        } else {
                            CloseBracket
                        };
                        self.token(error, closer, i);
                        self.copy(&nodes, &nodes[0].children, parent, end, 0);
                        return false;
                    }
                }
                Kind::Symbol(b')' | b']')
                    if saw_declaration && self.last_noncomment_kind(parent) == Some(Semicolon) =>
                {
                    for j in i..tokens.end {
                        match self.tokens[j].kind {
                            Kind::Symbol(b')' | b']') => self.stray_closers.push(j),
                            Kind::Comment => continue,
                            _ => break,
                        }
                    }
                }
                _ => {}
            }
            if self.nodes[parent].kind == Block
                && let Kind::Symbol(symbol @ (b'?' | b')' | b']')) = self.tokens[i].kind
                && self.before_block_end_or_declaration(i + 1..tokens.end)
            {
                let error = self.token(parent, Error, i);
                let closer = match symbol {
                    b')' => Some(CloseParen),
                    b']' => Some(CloseBracket),
                    _ => None,
                };
                if let Some(closer) = closer {
                    self.token(error, closer, i);
                }
                i += 1;
                if i == tokens.end {
                    return true;
                }
                continue;
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
            let stop = self.boundary(i, tokens.end, colon.is_some(), custom);
            let open = stop < tokens.end && self.tokens[stop].kind == Kind::Symbol(b'{');
            let semicolon = stop < tokens.end && self.tokens[stop].kind == Kind::Symbol(b';');
            let boundary = if stop < tokens.end {
                self.position(stop)
            } else {
                limit
            };
            if let Some(colon) = colon.filter(|_| !open) {
                self.declaration(parent, i, colon, stop, semicolon, boundary);
                saw_declaration = true;
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
                    self.header(parent, i..stop, end, has_end, open)
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
        false
    }

    fn before_block_end_or_declaration(&self, tokens: Range<usize>) -> bool {
        let mut significant = self.tokens[tokens]
            .iter()
            .filter(|token| token.kind != Kind::Comment);
        match significant.next().map(|token| token.kind) {
            None => true,
            Some(Kind::Name) => significant
                .next()
                .is_some_and(|token| token.kind == Kind::Symbol(b':')),
            _ => false,
        }
    }

    fn boundary(&mut self, mut i: usize, end: usize, declaration: bool, custom: bool) -> usize {
        while i < end {
            let token = &self.tokens[i];
            match token.kind {
                Kind::Symbol(b';' | b'}') => break,
                Kind::Symbol(symbol @ (b'(' | b'[' | b'{')) => {
                    let close = token.close.filter(|&close| close < end);
                    self.unfinished_group |= declaration && close.is_none();
                    if symbol == b'{' && !(declaration && custom) {
                        break;
                    }
                    i = close.unwrap_or(i);
                }
                _ => {}
            }
            i += 1;
        }
        i
    }

    fn missing_separator(&self, start: usize, colon: usize, end: usize) -> bool {
        let Some(first) = (colon + 1..end).find(|&i| self.tokens[i].kind != Kind::Comment) else {
            return false;
        };
        let mut i = first;
        let mut previous = first;
        while i < end {
            match self.tokens[i].kind {
                Kind::Comment => {
                    i += 1;
                    continue;
                }
                Kind::Symbol(b'(' | b'[' | b'{') => {
                    let Some(close) = self.tokens[i].close.filter(|&close| close < end) else {
                        return false;
                    };
                    i = close;
                }
                Kind::Symbol(b':') => {
                    // Require a name after a complete, nonempty value. A colon
                    // inside a group, string, or comment is not a separator.
                    return previous > first
                        && self.tokens[previous].kind == Kind::Name
                        && lr::parse_styles(
                            &self.source[self.position(start)..self.position(previous)],
                        )
                        .is_ok_and(|nodes| {
                            nodes[0]
                                .children
                                .first()
                                .is_some_and(|&id| nodes[id].kind == Declaration)
                        });
                }
                _ => {}
            }
            previous = i;
            i += 1;
        }
        false
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
        let ended_with_skipped_item =
            self.items(block, i + 1..close.unwrap_or(end), content_end, depth);
        if let Some(close) = close {
            self.token(block, CloseBrace, close);
            (block, close + 1)
        } else {
            if !ended_with_skipped_item {
                self.error(block, limit);
            }
            (block, end)
        }
    }

    fn header(
        &mut self,
        parent: usize,
        indices: Range<usize>,
        end: usize,
        semicolon: bool,
        has_block: bool,
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
        let parsed = lr::parse(&fragment).ok().or_else(|| {
            // Recover only the header of a real stylesheet rule. The local
            // block parser still handles the body, including malformed items.
            let header_len = end - start;
            if !has_block
                || self.nodes[0].kind != StyleSheet
                || self.tokens[indices.start].kind == Kind::AtKeyword
                || header_len > MAX_ATTRIBUTE_HEADER_BYTES
            {
                return None;
            }
            let (left, right) = self.empty_attribute_pair(indices.clone())?;
            let open = self.position(left) - start;
            let close = self.position(right) - start;
            let nodes = lr::recover(&fragment, lr::Mode::Sheet).ok()?;
            missing_attribute_name(&nodes, open, close, header_len).then_some(nodes)
        });
        if let Some(nodes) = parsed
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

    fn empty_attribute_pair(&self, indices: Range<usize>) -> Option<(usize, usize)> {
        let mut pair = None;
        let mut i = indices.start;
        while i < indices.end {
            match self.tokens[i].kind {
                Kind::Symbol(delim @ (b'[' | b'(')) => {
                    let close = self.tokens[i].close.filter(|&end| end < indices.end)?;
                    // Skip the interiors of other matched groups. Brackets in
                    // a value or a pseudo-class argument are not candidates.
                    if delim == b'['
                        && self.tokens[i + 1..close]
                            .iter()
                            .all(|token| token.kind == Kind::Comment)
                        && pair.replace((i, close)).is_some()
                    {
                        return None;
                    }
                    i = close;
                }
                Kind::Symbol(b')' | b']') => return None,
                _ => {}
            }
            i += 1;
        }
        pair
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
            if !self.missing_semicolon && !self.unfinished_group {
                self.missing_semicolon = self.missing_separator(start, colon, end);
            }
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
        // A semicolon can end a declaration even if masking leaves no value.
        // Keep closer-only values without a semicolon on the local path.
        let recover_closer = self.nodes[parent].kind == Declaration
            && (self
                .tokens
                .get(indices.end)
                .is_some_and(|token| token.kind == Kind::Symbol(b';'))
                || self.tokens[indices.clone()]
                    .iter()
                    .any(|token| !matches!(token.kind, Kind::Comment | Kind::Symbol(b')' | b']'))));
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
                Kind::Symbol(b')' | b']') if recover_closer => {
                    self.stray_closers.push(i);
                    Error
                }
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

fn missing_attribute_name(nodes: &[Node], open: usize, close: usize, header_len: usize) -> bool {
    let [rule] = nodes[0].children.as_slice() else {
        return false;
    };
    let rule = &nodes[*rule];
    let Some(block) = rule.children.last().map(|&id| &nodes[id]) else {
        return false;
    };
    let [left, right] = block.children.as_slice() else {
        return false;
    };
    if rule.kind != RuleSet
        || rule.range != (0..header_len + 2)
        || block.kind != Block
        || block.range != (header_len..header_len + 2)
        || nodes[*left].kind != OpenBrace
        || nodes[*left].range != (header_len..header_len + 1)
        || nodes[*right].kind != CloseBrace
        || nodes[*right].range != (header_len + 1..header_len + 2)
    {
        return false;
    }

    let mut errors = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == Error);
    let Some((id, error)) = errors.next() else {
        return false;
    };
    let Some(name) = error.parent.map(|id| &nodes[id]) else {
        return false;
    };
    let Some(attr) = name.parent.map(|id| &nodes[id]) else {
        return false;
    };
    errors.next().is_none()
        && error.range == (close..close)
        && error.children.is_empty()
        && name.kind == AttributeName
        && name.range == (close..close)
        && name.children == [id]
        && attr.kind == AttributeSelector
        && attr.range.end == close + 1
        && attr
            .children
            .iter()
            .any(|&id| nodes[id].kind == OpenBracket && nodes[id].range == (open..open + 1))
        && attr.children.last().is_some_and(|&id| {
            nodes[id].kind == CloseBracket && nodes[id].range == (close..close + 1)
        })
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
