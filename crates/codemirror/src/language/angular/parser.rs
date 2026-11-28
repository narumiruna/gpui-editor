use std::{ops::Range, sync::LazyLock};

use super::{AngularRegionKind, Node, tables};
use crate::language::lezer::{
    ExternalOptions, ExternalTokenizer, ParseError, Parser, SyntaxTree, TokenInput, Tokenizer,
};

fn parser(top_rule: [u16; 2]) -> Parser {
    let text = Tokenizer::External(ExternalTokenizer::new(
        ExternalOptions::default(),
        |input, _| {
            let start = input.position();
            let mut next = input.peek(0)?;
            while let Some(ch) = next {
                if ch == u16::from(b'{') && input.peek(1)? == Some(u16::from(b'{')) {
                    break;
                }
                next = input.advance(1)?;
                if ch == u16::from(b'\n') {
                    break;
                }
            }
            Ok((input.position() > start).then(|| input.token(1)))
        },
    ));
    tables::parser(
        top_rule,
        &[
            text,
            attribute(b'\'', 33, false),
            attribute(b'"', 34, false),
            attribute(b'\'', 35, true),
            attribute(b'"', 36, true),
            0.into(),
            1.into(),
        ],
    )
}

fn attribute(quote: u8, term: u16, script: bool) -> Tokenizer {
    Tokenizer::External(ExternalTokenizer::new(
        ExternalOptions::default(),
        move |input, _| {
            let start = input.position();
            let mut next = input.peek(0)?;
            while let Some(ch) = next {
                if ch == u16::from(quote) || !script && stops_attribute(input, ch)? {
                    break;
                }
                next = input.advance(1)?;
            }
            Ok((input.position() > start).then(|| input.token(term)))
        },
    ))
}

fn stops_attribute(input: &TokenInput<'_>, ch: u16) -> Result<bool, ParseError> {
    Ok(ch == u16::from(b'&') || ch == u16::from(b'{') && input.peek(1)? == Some(u16::from(b'{')))
}

fn strict(parser: &Parser, input: &str) -> Result<SyntaxTree, ParseError> {
    parser.parse(input).or_else(|error| match error {
        ParseError::Ambiguous { .. } => parser.parse_glr(input),
        other => Err(other),
    })
}

pub(super) fn parse(
    source: &str,
    range: Range<usize>,
    kind: AngularRegionKind,
) -> (Vec<Node>, bool) {
    static CONTENT: LazyLock<Parser> = LazyLock::new(|| parser(tables::CONTENT));
    static ATTRIBUTE: LazyLock<Parser> = LazyLock::new(|| parser(tables::ATTRIBUTE));
    let parser = match kind {
        AngularRegionKind::Content => &*CONTENT,
        AngularRegionKind::Attribute => &*ATTRIBUTE,
    };
    let input = &source[range.clone()];
    let error = match strict(parser, input) {
        Ok(tree) => return (collect(&tree, range), false),
        Err(error) => error,
    };
    if input.len() <= 65_536
        && let ParseError::UnexpectedToken { offset, .. } = error
    {
        let next = input.as_bytes().get(offset);
        if kind == AngularRegionKind::Attribute
            && input.starts_with(['[', '('])
            && (next == Some(&b'=') || next == Some(&b']') && input.starts_with("[("))
            && let Ok(tree) = parser.parse_recovering(input)
            && has_missing_binding_closer(&tree, input, offset)
        {
            return (collect(&tree, range), true);
        }
        let suffixes: &[_] = match kind {
            AngularRegionKind::Content => &["}}"],
            AngularRegionKind::Attribute => &["\"", "'", "}}\"", "}}'"],
        };
        for suffix in suffixes {
            let closed = format!("{input}{suffix}");
            if let Ok(tree) = strict(parser, &closed) {
                return (collect(&tree, range), true);
            }
        }
    }
    let name = match kind {
        AngularRegionKind::Content => "Content",
        AngularRegionKind::Attribute => "Attribute",
    };
    (
        vec![
            Node {
                name,
                range: range.clone(),
                children: vec![1],
                parent: None,
            },
            Node {
                name: "⚠",
                range,
                children: Vec::new(),
                parent: Some(0),
            },
        ],
        true,
    )
}

// Keep only the repair for one missing name closer before a quoted value.
fn has_missing_binding_closer(tree: &SyntaxTree, input: &str, offset: usize) -> bool {
    let mut children = tree.root().children();
    let (Some(name), Some(equals), Some(value)) =
        (children.next(), children.next(), children.next())
    else {
        return false;
    };
    if !matches!(
        (name.name(), value.name()),
        ("BoundAttributeName", "ExpressionAttributeValue")
            | ("EventName", "StatementAttributeValue")
    ) || equals.name() != "Is"
        || children.next().is_some()
    {
        return false;
    }
    // If `[(...` reaches `=`, the inner close paren must already exist.
    let previous = if input.starts_with("[(") && input.as_bytes().get(offset) == Some(&b'=') {
        ")"
    } else {
        "Identifier"
    };
    let missing = name
        .children()
        .zip(name.children().skip(1))
        .any(|(prev, node)| {
            node.is_error()
                && node.range() == (offset..offset)
                && prev.name() == previous
                && input
                    .get(prev.range().end..offset)
                    .is_some_and(|gap| super::trim_space(gap).is_empty())
        });
    if !missing {
        return false;
    }
    let mut pending = vec![tree.root()];
    let mut count = 0;
    while let Some(node) = pending.pop() {
        count += usize::from(node.is_error());
        pending.extend(node.children());
    }
    count == 1
}

fn collect(tree: &SyntaxTree, source: Range<usize>) -> Vec<Node> {
    static NAMES: LazyLock<Vec<&'static str>> =
        LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());
    let len = source.len();
    let mut nodes: Vec<Node> = Vec::new();
    let mut pending = vec![(tree.root(), None::<usize>)];
    while let Some((node, parent)) = pending.pop() {
        let id = nodes.len();
        let local = node.range();
        let synthetic = local.start >= len && local.end > len;
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            name: if synthetic {
                "⚠"
            } else {
                NAMES[usize::from(node.term())]
            },
            range: source.start + local.start.min(len)..source.start + local.end.min(len),
            children: Vec::new(),
            parent,
        });
        if !synthetic {
            pending.extend(node.children().rev().map(|child| (child, Some(id))));
        }
    }
    nodes
}
