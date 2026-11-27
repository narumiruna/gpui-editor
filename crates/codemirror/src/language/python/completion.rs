//! Native completion services from `@codemirror/lang-python` 6.2.1.
//! See `docs/licenses/codemirror-lang-python.txt`.

use std::{iter, sync::Arc};

use crate::autocomplete::session::{CompletionItem, SourceResult};

use super::{PythonDocument, PythonNode};

mod globals;

pub use globals::{PythonSnippet, SNIPPETS, python_completion_snippet};

/// Return the built-ins and snippets installed by Python language support.
/// Register [`local_completion_source`] separately for local names. When a
/// completion is accepted, use [`python_completion_snippet`] to find its
/// native snippet action, if any.
///
/// Positions are UTF-8 byte offsets. Completion in incomplete code depends
/// on the available native syntax tree, which can differ from Lezer recovery.
pub fn global_completion_source(
    doc: &PythonDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    let inner = resolve_left(doc, position)?;
    if ancestors(inner).any(suppressed) {
        return None;
    }
    // Upstream matches an ASCII word suffix within 250 UTF-16 units of the
    // cursor. Each matched ASCII byte is one unit; no line break can match.
    let length = doc.text().as_str()[..position]
        .bytes()
        .rev()
        .take(250)
        .take_while(|&byte| ascii_word(byte))
        .count();
    if length == 0 && !explicit {
        return None;
    }
    Some(
        SourceResult::new(position - length, Arc::clone(&globals::OPTIONS))
            .with_valid_for(|text, _, _| text.bytes().all(ascii_word)),
    )
}

/// Return names from the native tree, from the innermost scope outwards and
/// in syntax order within each scope. This is a syntax search: it includes
/// declarations after the cursor and retains duplicates.
///
/// Positions are UTF-8 byte offsets. Completion in incomplete code depends
/// on the available native syntax tree, which can differ from Lezer recovery.
pub fn local_completion_source(
    doc: &PythonDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    let inner = resolve_left(doc, position)?;
    if suppressed(inner) {
        return None;
    }
    let range = inner.range();
    let text = doc.text().slice(range.clone())?;
    let from = if inner.name() == "VariableName"
        || text.encode_utf16().take(20).count() < 20 && identifier(text)
    {
        range.start
    } else if explicit {
        position
    } else {
        return None;
    };
    let mut options = Vec::new();
    for scope in ancestors(inner).filter(|node| scope_node(node.name())) {
        collect_scope(doc, scope, &mut options);
    }
    Some(SourceResult::new(from, options).with_valid_for(|text, _, _| identifier(text)))
}

fn resolve_left(doc: &PythonDocument, position: usize) -> Option<PythonNode<'_>> {
    if !doc.text().as_str().is_char_boundary(position) {
        return None;
    }
    let mut node = doc.root();
    loop {
        let children = &node.data().children;
        let index = children.partition_point(|&id| doc.nodes[id].range.end < position);
        if let Some(&id) = children.get(index)
            && doc.nodes[id].range.start < position
        {
            node = PythonNode { doc, id };
        } else {
            return Some(node);
        }
    }
}

fn ancestors(node: PythonNode<'_>) -> impl Iterator<Item = PythonNode<'_>> {
    iter::successors(Some(node), |node| node.parent())
}

fn suppressed(node: PythonNode<'_>) -> bool {
    matches!(
        node.name(),
        "String" | "FormatString" | "Comment" | "PropertyName"
    )
}

fn scope_node(name: &str) -> bool {
    matches!(
        name,
        "Script"
            | "Body"
            | "FunctionDefinition"
            | "ClassDefinition"
            | "LambdaExpression"
            | "ForStatement"
            | "MatchClause"
    )
}

fn ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn identifier(text: &str) -> bool {
    // The pinned expression is /^[\w\xa1-\uffff][\w\d\xa1-\uffff]*$/.
    // Astral characters match too: both of their UTF-16 units are in range.
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch >= '\u{a1}' || ch.is_ascii() && ascii_word(ch as u8))
}

fn collect_scope(doc: &PythonDocument, root: PythonNode<'_>, options: &mut Vec<CompletionItem>) {
    let mut define = |node: PythonNode<'_>, kind| {
        if let Some(name) = doc.text().slice(node.range()) {
            options.push(item(name, kind));
        }
    };
    let mut pending = vec![(root, true)];
    while let Some((node, outer)) = pending.pop() {
        if !gather(node, outer, &mut define) && (outer || !scope_node(node.name())) {
            pending.extend(node.children().rev().map(|child| (child, false)));
        }
    }
}

// Return true to skip the node's descendants.
fn gather(
    node: PythonNode<'_>,
    outer: bool,
    define: &mut impl FnMut(PythonNode<'_>, &'static str),
) -> bool {
    let definition = match node.name() {
        "FunctionDefinition" => Some("function"),
        "ClassDefinition" => Some("class"),
        "CapturePattern" | "AsPattern" => Some("variable"),
        _ => None,
    };
    if let Some(kind) = definition {
        if !outer && let Some(id) = node.children().find(|child| child.name() == "VariableName") {
            define(id, kind);
        }
        return !outer;
    }
    match node.name() {
        "ForStatement" if outer => {
            for child in node.children().take_while(|child| child.name() != "in") {
                if child.name() == "VariableName" {
                    define(child, "variable");
                }
            }
        }
        "ImportStatement" => {
            let mut children = node.children().peekable();
            let kind = if children.peek().is_some_and(|child| child.name() == "from") {
                "variable"
            } else {
                "namespace"
            };
            children.find(|child| child.name() == "import");
            while let Some(child) = children.next() {
                if child.name() == "VariableName"
                    && children.peek().is_none_or(|next| next.name() != "as")
                {
                    define(child, kind);
                }
            }
        }
        "AssignStatement" => {
            for child in node.children() {
                match child.name() {
                    "VariableName" => define(child, "variable"),
                    ":" | "AssignOp" => break,
                    _ => {}
                }
            }
        }
        "ParamList" => {
            let mut previous = "";
            for child in node.children() {
                let name = child.name();
                if name == "VariableName"
                    && !previous.contains('*')
                    && !previous.contains("AssignOp")
                {
                    define(child, "variable");
                }
                previous = name;
            }
        }
        _ => {}
    }
    false
}

fn item(label: &str, kind: &str) -> CompletionItem {
    let mut option = CompletionItem::new(label);
    option.kind = Some(kind.to_owned());
    option
}
