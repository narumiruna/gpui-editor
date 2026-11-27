//! Native completion sources from `@codemirror/lang-javascript` 6.2.5.
//! See `docs/licenses/codemirror-lang-javascript.txt`.

use std::{iter, sync::Arc};

use crate::autocomplete::session::{CompletionItem, SourceResult};

use super::{JavascriptDocument, JavascriptNode};

mod scope;
mod snippets;

pub use scope::{
    JavascriptScope, JavascriptScopeProperty, JavascriptScopeValue, scope_completion_source,
};
pub use snippets::{
    JavascriptSnippet, SNIPPETS, TYPESCRIPT_SNIPPETS, javascript_completion_snippet,
};

/// Return the fixed snippet and keyword list installed by the upstream JS or TS support.
/// Register [`local_completion_source`] separately for local names.
///
/// At acceptance, use [`javascript_completion_snippet`] to identify snippet items
/// and apply their templates. A plain completion transaction inserts the label.
pub fn javascript_completion_source(
    doc: &JavascriptDocument,
    position: usize,
    explicit: bool,
    typescript: bool,
) -> Option<SourceResult> {
    let inner = resolve_left(doc, position)?;
    if ancestors(inner).any(|node| suppresses_completion(node.kind().name())) {
        return None;
    }
    // The list source matches an ASCII word suffix within 250 UTF-16 units.
    // Each matched ASCII byte takes one unit and no line ending can match.
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
        SourceResult::new(position - length, snippets::options(typescript))
            .with_valid_for(|value, _, _| value.bytes().all(ascii_word)),
    )
}

/// Return locally defined names from the syntax tree, in inner-to-outer scope
/// order. This is a syntax search, not name resolution: it includes declarations
/// after the caret and preserves duplicate labels. Incomplete syntax can give
/// different results from Lezer recovery.
pub fn local_completion_source(
    doc: &JavascriptDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    let inner = resolve_left(doc, position)?;
    if suppresses_completion(inner.kind().name()) {
        return None;
    }
    let from = if word_at(doc, inner).is_some() {
        inner.range().start
    } else if explicit {
        position
    } else {
        return None;
    };
    let mut options = Vec::new();
    for node in ancestors(inner).filter(|node| is_scope(node.kind().name())) {
        collect_scope(doc, node, &mut options);
    }
    Some(identifier_result(from, options))
}

/// The object path and the full syntax name at the query position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionPath<'a> {
    pub path: Vec<&'a str>,
    pub name: &'a str,
}

/// Find a name or a directly named member chain such as `a.b.c`.
/// Computed or indirect object paths return `None`. An explicit query outside
/// a word can return an empty path and name. Native recovery can give different
/// results for malformed syntax.
pub fn completion_path(
    doc: &JavascriptDocument,
    position: usize,
    explicit: bool,
) -> Option<CompletionPath<'_>> {
    let inner = resolve_left(doc, position)?;
    let kind = inner.kind().name();
    match kind {
        "PropertyName" | "." | "?." => {
            let parent = inner.parent()?;
            let name = if kind == "PropertyName" {
                read(doc, inner)?
            } else {
                ""
            };
            return if parent.kind().name() == "MemberExpression" {
                member_path(doc, parent, name)
            } else if doc.is_recovered() {
                recovered_member_path(doc, parent, inner, name)
            } else if kind == "PropertyName" {
                member_path(doc, parent, name)
            } else {
                None
            };
        }
        _ if suppresses_completion(kind) => return None,
        _ => {}
    }
    if let Some(name) = word_at(doc, inner) {
        Some(CompletionPath {
            path: Vec::new(),
            name,
        })
    } else if kind == "MemberExpression" {
        member_path(doc, inner, "")
    } else {
        explicit.then_some(CompletionPath {
            path: Vec::new(),
            name: "",
        })
    }
}

fn member_path<'a>(
    doc: &'a JavascriptDocument,
    mut member: JavascriptNode<'_>,
    name: &'a str,
) -> Option<CompletionPath<'a>> {
    let mut path = Vec::new();
    loop {
        let object = member.children().next()?;
        match object.kind().name() {
            "VariableName" => {
                path.push(read(doc, object)?);
                path.reverse();
                return Some(CompletionPath { path, name });
            }
            "MemberExpression" => {
                let property = object.children().next_back()?;
                if property.kind().name() != "PropertyName" {
                    return None;
                }
                path.push(read(doc, property)?);
                member = object;
            }
            _ => return None,
        }
    }
}

// Local recovery can keep the tokens of an unfinished member chain as siblings
// instead of creating MemberExpression nodes. Only use a direct named chain.
fn recovered_member_path<'a>(
    doc: &'a JavascriptDocument,
    parent: JavascriptNode<'_>,
    inner: JavascriptNode<'_>,
    name: &'a str,
) -> Option<CompletionPath<'a>> {
    let start = inner.range().start;
    let mut previous = parent
        .children()
        .rev()
        .skip_while(|node| node.range().start >= start)
        .filter(|node| !matches!(node.kind().name(), "LineComment" | "BlockComment"));
    let dot = |node: JavascriptNode<'_>| matches!(node.kind().name(), "." | "?.");
    if inner.kind().name() == "PropertyName" && !dot(previous.next()?) {
        return None;
    }
    let mut path = Vec::new();
    loop {
        let node = previous.next()?;
        match node.kind().name() {
            "VariableName" => {
                path.push(read(doc, node)?);
                path.reverse();
                return Some(CompletionPath { path, name });
            }
            "PropertyName" if dot(previous.next()?) => path.push(read(doc, node)?),
            _ => return None,
        }
    }
}

fn collect_scope(
    doc: &JavascriptDocument,
    scope: JavascriptNode<'_>,
    out: &mut Vec<CompletionItem>,
) {
    let mut pending: Vec<_> = scope.children().rev().collect();
    while let Some(node) = pending.pop() {
        let kind = node.kind().name();
        let declaration = match kind {
            "FunctionDeclaration" => Some("function"),
            "ClassDeclaration" => Some("class"),
            "EnumDeclaration" => Some("constant"),
            "TypeAliasDeclaration" => Some("type"),
            "NamespaceDeclaration" => Some("namespace"),
            _ => None,
        };
        if let Some(kind) = declaration {
            if let Some(id) = node
                .children()
                .find(|child| child.kind().name() == "VariableDefinition")
                && let Some(name) = read(doc, id)
            {
                out.push(item(name, kind));
            }
        } else if kind != "ClassExpression" && !is_scope(kind) {
            if let Some(kind) = match kind {
                "VariableDefinition"
                    if node
                        .parent()
                        .is_none_or(|p| p.kind().name() != "FunctionDeclaration") =>
                {
                    Some("variable")
                }
                "TypeDefinition" => Some("type"),
                _ => None,
            } && let Some(name) = read(doc, node)
            {
                out.push(item(name, kind));
            }
            pending.extend(node.children().rev());
        }
    }
}

fn is_scope(kind: &str) -> bool {
    matches!(
        kind,
        "Script"
            | "Block"
            | "FunctionExpression"
            | "FunctionDeclaration"
            | "ArrowFunction"
            | "MethodDeclaration"
            | "ForStatement"
    )
}

fn resolve_left(doc: &JavascriptDocument, position: usize) -> Option<JavascriptNode<'_>> {
    doc.text().slice(position..position)?;
    let mut node = doc.root();
    // At a boundary, resolve to the preceding token. Empty recovery nodes
    // cannot contain a position with left bias.
    while let Some(child) = node
        .children()
        .find(|child| child.range().end >= position)
        .filter(|child| child.range().start < position)
    {
        node = child;
    }
    Some(node)
}

fn ancestors(node: JavascriptNode<'_>) -> impl Iterator<Item = JavascriptNode<'_>> {
    iter::successors(Some(node), |node| node.parent())
}

fn suppresses_completion(name: &str) -> bool {
    matches!(
        name,
        "TemplateString"
            | "String"
            | "RegExp"
            | "LineComment"
            | "BlockComment"
            | "VariableDefinition"
            | "TypeDefinition"
            | "Label"
            | "PropertyDefinition"
            | "PropertyName"
            | "PrivatePropertyDefinition"
            | "PrivatePropertyName"
            | "JSXText"
            | "JSXAttributeValue"
            | "JSXOpenTag"
            | "JSXCloseTag"
            | "JSXSelfClosingTag"
            | "."
            | "?."
    )
}

fn word_at<'a>(doc: &'a JavascriptDocument, node: JavascriptNode<'_>) -> Option<&'a str> {
    let text = read(doc, node)?;
    (node.kind().name() == "VariableName"
        || text.encode_utf16().take(20).count() < 20 && identifier(text))
    .then_some(text)
}

fn read<'a>(doc: &'a JavascriptDocument, node: JavascriptNode<'_>) -> Option<&'a str> {
    doc.text().slice(node.range())
}

fn ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn identifier(text: &str) -> bool {
    // Upstream uses [\w$\xa1-\uffff] without the Unicode flag. This also
    // accepts astral characters, whose surrogate units are in that range.
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$') || ch >= '\u{a1}')
}

fn item(label: &str, kind: &str) -> CompletionItem {
    let mut item = CompletionItem::new(label);
    item.kind = Some(kind.to_owned());
    item
}

fn identifier_result(from: usize, options: impl Into<Arc<[CompletionItem]>>) -> SourceResult {
    SourceResult::new(from, options).with_valid_for(|value, _, _| identifier(value))
}
