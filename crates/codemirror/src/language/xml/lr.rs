use std::sync::LazyLock;

use super::{
    super::lezer::{Parser, SyntaxNode, SyntaxTree, TreeFragment},
    Node,
    XmlNodeKind::{self, *},
    parser, tables,
};

static PARSER: LazyLock<Parser> = LazyLock::new(tables::parser);

pub(super) fn parse(
    source: &str,
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, Option<SyntaxTree>) {
    let fragment = prefix.map(|(tree, end)| TreeFragment {
        tree,
        old_range: 0..end,
        new_range: 0..end,
    });
    if let Ok(tree) = PARSER.parse_incremental(source, fragment) {
        (nodes(&tree, None), Some(tree))
    } else if let Ok(tree) = PARSER.parse_recovering(source) {
        (nodes(&tree, Some(source)), None)
    } else {
        (parser::parse(source), None)
    }
}

fn nodes(tree: &SyntaxTree, recovered_source: Option<&str>) -> Vec<Node> {
    let root = tree.root();
    let eof = root.range().end;
    let mut pending = vec![(Some(root), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = pending.pop() {
        let id = nodes.len();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        let (kind, range) =
            syntax.map_or((Error, eof..eof), |node| (kind(node.term()), node.range()));
        nodes.push(Node {
            kind,
            range,
            parent,
            children: Vec::new(),
        });
        if let Some(syntax) = syntax {
            if kind == AttributeValue
                && recovered_source.is_some_and(|source| missing_quote_at_eof(syntax, source))
            {
                pending.push((None, Some(id)));
            }
            pending.extend(syntax.children().rev().map(|child| (Some(child), Some(id))));
        }
    }
    nodes
}

// Bounded recovery can merge the missing quote at EOF with a preceding
// error on a bad reference because the text between them is hidden.
fn missing_quote_at_eof(node: SyntaxNode<'_>, source: &str) -> bool {
    let range = node.range();
    if range.end != source.len() {
        return false;
    }
    let [quote @ (b'\'' | b'"'), .., last] = &source.as_bytes()[range] else {
        return false;
    };
    quote != last
        && node
            .children()
            .next_back()
            .is_some_and(|child| child.is_error() && child.range().end < source.len())
}

fn kind(term: u16) -> XmlNodeKind {
    match term {
        0 => Error,
        1 => StartTag,
        2 | 4 | 5 => StartCloseTag,
        3 => MissingCloseTag,
        6 => Document,
        7 => Text,
        8 => EntityReference,
        9 => CharacterReference,
        10 => Cdata,
        11 => Element,
        12 => EndTag,
        13 => OpenTag,
        14 => TagName,
        15 => Attribute,
        16 => AttributeName,
        17 => Is,
        18 => AttributeValue,
        19 => CloseTag,
        20 => SelfCloseEndTag,
        21 => SelfClosingTag,
        22 => Comment,
        23 => ProcessingInst,
        24 => MismatchedCloseTag,
        25 => DoctypeDecl,
        _ => unreachable!("unnamed XML term {term}"),
    }
}
