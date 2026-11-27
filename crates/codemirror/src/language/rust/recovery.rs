mod lexer;

use super::{Node, RustNodeKind, RustNodeKind::*};

pub(super) fn parse(source: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    add(&mut nodes, None, SourceFile, 0..source.len());
    let mut stack = Vec::new();
    let mut parent = 0;
    let mut attribute = None;
    let mut previous = None;
    for mut token in lexer::tokens(source) {
        let start = token.range.start;
        let end = token.range.end;
        if matches!(token.kind, Attribute | InnerAttribute) {
            attribute = Some((token.kind, start));
            continue;
        }
        match token.kind {
            OpenBrace | OpenParen | OpenBracket => {
                let is_macro = previous == Some(Bang);
                let (kind, close) = match token.kind {
                    OpenBrace => (if is_macro { BracedTokens } else { Block }, CloseBrace),
                    OpenParen => (
                        if is_macro {
                            ParenthesizedTokens
                        } else {
                            ParenthesizedExpression
                        },
                        CloseParen,
                    ),
                    OpenBracket => (
                        if is_macro {
                            BracketedTokens
                        } else {
                            ArrayExpression
                        },
                        CloseBracket,
                    ),
                    _ => unreachable!(),
                };
                let (kind, start) = attribute.take().unwrap_or((kind, start));
                let group = add(&mut nodes, Some(parent), kind, start..end);
                stack.push((parent, close));
                parent = group;
            }
            CloseBrace | CloseParen | CloseBracket => {
                if stack.last().is_some_and(|&(_, close)| close == token.kind) {
                    add(&mut nodes, Some(parent), token.kind, token.range);
                    nodes[parent].range.end = end;
                    parent = stack.pop().unwrap().0;
                    previous = Some(token.kind);
                    continue;
                }
                // Do not discard an unexpected delimiter or close a different group.
                token.kind = Error;
            }
            Identifier if matches!(previous, Some(FnKeyword | LetKeyword)) => {
                token.kind = BoundIdentifier
            }
            _ => {}
        }
        let id = add(&mut nodes, Some(parent), token.kind, token.range);
        if !token.closed {
            add(&mut nodes, Some(id), Error, end..end);
        }
        if !matches!(token.kind, LineComment | BlockComment) {
            previous = Some(token.kind);
        }
    }
    while let Some((outer, _)) = stack.pop() {
        let end = source.len();
        add(&mut nodes, Some(parent), Error, end..end);
        nodes[parent].range.end = end;
        parent = outer;
    }
    nodes
}

fn add(
    nodes: &mut Vec<Node>,
    parent: Option<usize>,
    kind: RustNodeKind,
    range: std::ops::Range<usize>,
) -> usize {
    let id = nodes.len();
    if let Some(parent) = parent {
        nodes[parent].children.push(id);
    }
    nodes.push(Node {
        kind,
        range,
        parent,
        children: Vec::new(),
        statement: false,
    });
    id
}
