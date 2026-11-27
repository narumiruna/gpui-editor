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
    if let Some(source) = recovered_source {
        recover_self_close_at_eof(&mut nodes, source);
    }
    nodes
}

// At EOF, Lezer can delete two adjacent punctuation bytes after `=` and
// retain `/>`. The bounded parser instead closes the OpenTag at `=` (or
// after its whitespace) and parses the suffix as content. Correct only the
// final top-level element, when this is the first error in the tree.
fn recover_self_close_at_eof(nodes: &mut Vec<Node>, source: &str) {
    let Some([element, tag, attribute, error]) = self_close_recovery(nodes, source) else {
        return;
    };
    let end = source.len();
    let delimiter = end - 2;
    nodes.truncate(error + 1);
    let end_tag = nodes.len();
    nodes[element].children.truncate(1);
    let opening = &mut nodes[tag];
    opening.kind = SelfClosingTag;
    opening.range.end = end;
    opening.children.pop(); // Missing tag end.
    opening.children.push(end_tag);
    nodes[attribute].range.end = delimiter;
    nodes[error].range.end = delimiter;
    nodes.push(Node {
        kind: SelfCloseEndTag,
        range: delimiter..end,
        parent: Some(tag),
        children: Vec::new(),
    });
}

fn self_close_recovery(nodes: &[Node], source: &str) -> Option<[usize; 4]> {
    let end = source.len();
    let &element = nodes[0].children.last()?;
    let complete = &nodes[element];
    let &[tag, .., eof_error] = complete.children.as_slice() else {
        return None;
    };
    let opening = &nodes[tag];
    let at = opening.range.end;
    let [first, second, b'/', b'>'] = source.as_bytes().get(at..)? else {
        return None;
    };
    let brackets = b"()[]{}";
    if !brackets.contains(first) || !(brackets.contains(second) || *second == b'<') {
        return None;
    }
    let leaf = |id: usize, kind, range| {
        let node = &nodes[id];
        node.kind == kind && node.range == range && node.children.is_empty()
    };
    if complete.kind != Element
        || complete.range != (opening.range.start..end)
        || opening.kind != OpenTag
        || !leaf(eof_error, Error, end..end)
    {
        return None;
    }
    let matches_content = match complete.children.as_slice() {
        // In content, the second punctuation byte and the slash form `</`.
        [_, text, close, tail, _] if *second == b'<' => {
            leaf(*text, Text, at..at + 1)
                && leaf(*close, StartCloseTag, at + 1..at + 3)
                && leaf(*tail, Text, at + 3..end)
        }
        [_, text, _] if *second != b'<' => leaf(*text, Text, at..end),
        _ => false,
    };
    let &[start_tag, tag_name, .., attribute, tag_error] = opening.children.as_slice() else {
        return None;
    };
    let &[name, is, error] = nodes[attribute].children.as_slice() else {
        return None;
    };
    (matches_content
        && nodes[start_tag].kind == StartTag
        && nodes[tag_name].kind == TagName
        && nodes[attribute].kind == Attribute
        && nodes[name].kind == AttributeName
        && nodes[is].kind == Is
        && leaf(error, Error, at..at)
        && leaf(tag_error, Error, at..at)
        && nodes[..error].iter().all(|node| node.kind != Error))
    .then_some([element, tag, attribute, error])
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
