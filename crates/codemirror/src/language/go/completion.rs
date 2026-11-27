//! Native completion sources from `@codemirror/lang-go` 6.0.1.
//! See `docs/licenses/codemirror-lang-go.txt`.

use std::{
    iter,
    sync::{Arc, LazyLock},
};

use crate::autocomplete::{
    Snippet,
    session::{CompletionItem, SourceResult},
};

use super::{GoDocument, GoNode, GoNodeKind, GoNodeKind::*};

/// A fixed keyword completion with a native snippet template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GoSnippet {
    pub label: &'static str,
    pub detail: Option<&'static str>,
    pub template: &'static str,
}

impl GoSnippet {
    /// Parse the template for insertion with the native snippet engine.
    pub fn snippet(self) -> Snippet {
        Snippet::parse(self.template)
    }
}

const fn snippet(
    label: &'static str,
    detail: Option<&'static str>,
    template: &'static str,
) -> GoSnippet {
    GoSnippet {
        label,
        detail,
        template,
    }
}

/// The upstream Go snippet catalog, in upstream order.
pub static SNIPPETS: &[GoSnippet] = &[
    snippet(
        "func",
        Some("declaration"),
        "func ${name}(${params}) ${type} {\n\t${}\n}",
    ),
    snippet(
        "func",
        Some("method declaration"),
        "func (${receiver}) ${name}(${params}) ${type} {\n\t${}\n}",
    ),
    snippet("var", Some("declaration"), "var ${name} = ${value}"),
    snippet("type", Some("declaration"), "type ${name} ${type}"),
    snippet("const", Some("declaration"), "const ${name} = ${value}"),
    snippet("type", Some("alias declaration"), "type ${name} = ${type}"),
    snippet(
        "for",
        Some("loop"),
        "for ${init}; ${test}; ${update} {\n\t${}\n}",
    ),
    snippet(
        "for",
        Some("range"),
        "for ${i} := range ${value} {\n\t${}\n}",
    ),
    snippet("select", Some("statement"), "select {\n\t${}\n}"),
    snippet("case", None, "case ${}:\n${}"),
    snippet("switch", Some("statement"), "switch ${} {\n\t${}\n}"),
    snippet(
        "switch",
        Some("type statement"),
        "switch ${}.(${type}) {\n\t${}\n}",
    ),
    snippet("if", Some("block"), "if ${} {\n\t${}\n}"),
    snippet(
        "if",
        Some("/ else block"),
        "if ${} {\n\t${}\n} else {\n\t${}\n}",
    ),
    snippet(
        "import",
        Some("declaration"),
        "import ${name} \"${module}\"\n${}",
    ),
];

const KEYWORDS: &str = "interface struct chan map package go return break continue goto fallthrough \
    else defer range true false nil";

static FIXED_OPTIONS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| {
    SNIPPETS
        .iter()
        .map(|snippet| {
            let mut option = item(snippet.label, "keyword");
            option.detail = snippet.detail.map(str::to_owned);
            option
        })
        .chain(
            KEYWORDS
                .split_ascii_whitespace()
                .map(|label| item(label, "keyword")),
        )
        .collect()
});

/// Find the template for an item from the fixed source. Plain keywords return `None`.
/// For snippet items, use [`GoSnippet::snippet`] instead of inserting the label.
pub fn go_completion_snippet(item: &CompletionItem) -> Option<&'static GoSnippet> {
    if item.kind.as_deref() != Some("keyword") {
        return None;
    }
    SNIPPETS
        .iter()
        .find(|snippet| snippet.label == item.label && snippet.detail == item.detail.as_deref())
}

/// Return the fixed snippets and keywords installed by `go()`.
/// Register [`go_local_completion`] separately for local names.
pub fn go_snippet_completion(
    doc: &GoDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    let inner = resolve_left(doc, position)?;
    if recovered_selector(doc, inner.kind(), position)
        || ancestors(inner).any(|node| suppresses_completion(node.kind()))
    {
        return None;
    }
    // The upstream list matches an ASCII word suffix within 250 UTF-16 units.
    // Each matching byte takes one unit, and no line ending matches.
    let length = doc.text().as_str()[..position]
        .bytes()
        .rev()
        .take(250)
        .take_while(|&byte| ascii_word(byte))
        .count();
    (explicit || length != 0).then(|| {
        SourceResult::new(position - length, Arc::clone(&FIXED_OPTIONS))
            .with_valid_for(|value, _, _| value.bytes().all(ascii_word))
    })
}

/// Return names from enclosing syntax scopes, in inner-to-outer order.
/// This is not Go name resolution: it includes declarations after the caret,
/// keeps duplicates, and only gathers the declaration forms used upstream.
/// Incomplete syntax can give different results from Lezer recovery.
pub fn go_local_completion(
    doc: &GoDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    let inner = resolve_left(doc, position)?;
    if suppresses_completion(inner.kind()) || recovered_selector(doc, inner.kind(), position) {
        return None;
    }
    let source = doc.text().as_str();
    let range = inner.range();
    let from = if inner.kind() == VariableName || {
        let text = &source[range.clone()];
        // Upstream only applies the fallback word test to short nodes.
        // Count at most 20 units even when the resolved node is large.
        text.encode_utf16().take(20).count() < 20 && identifier(text)
    } {
        range.start
    } else if explicit {
        position
    } else {
        return None;
    };
    let mut options = Vec::new();
    for scope in ancestors(inner).filter(|node| is_scope(node.kind())) {
        collect_scope(source, scope, &mut options);
    }
    Some(SourceResult::new(from, options).with_valid_for(|value, _, _| identifier(value)))
}

// If recovery loses the function containing an unfinished selector, keep the
// dot a completion boundary. Do not treat a numeric literal as a selector.
fn recovered_selector(doc: &GoDocument, kind: GoNodeKind, position: usize) -> bool {
    if kind != Error {
        return false;
    }
    let Some(prefix) = doc.text().as_str()[..position].strip_suffix('.') else {
        return false;
    };
    let before = prefix.trim_end_matches(|ch: char| ch == '_' || ch.is_alphanumeric());
    prefix[before.len()..].starts_with(|ch: char| ch == '_' || ch.is_alphabetic())
}

fn resolve_left(doc: &GoDocument, position: usize) -> Option<GoNode<'_>> {
    if !doc.text().as_str().is_char_boundary(position) {
        return None;
    }
    let mut node = doc.root();
    loop {
        let children = &node.data().children;
        let index = children.partition_point(|&id| node.nodes[id].range.end < position);
        match children.get(index) {
            Some(&id) if node.nodes[id].range.start < position => node = GoNode { id, ..node },
            _ => return Some(node),
        }
    }
}

fn ancestors(node: GoNode<'_>) -> impl Iterator<Item = GoNode<'_>> {
    iter::successors(Some(node), |node| node.parent())
}

fn suppresses_completion(kind: GoNodeKind) -> bool {
    matches!(
        kind,
        String | LineComment | BlockComment | DefName | LabelName | FieldName | Dot
    )
}

fn is_scope(kind: GoNodeKind) -> bool {
    matches!(
        kind,
        SourceFile
            | Block
            | FunctionDecl
            | MethodDecl
            | FunctionLiteral
            | ForStatement
            | SwitchStatement
            | TypeSwitchStatement
            | IfStatement
    )
}

fn collect_scope(source: &str, scope: GoNode<'_>, options: &mut Vec<CompletionItem>) {
    let mut pending: Vec<_> = scope.children().rev().collect();
    while let Some(node) = pending.pop() {
        let (kind, spec) = match node.kind() {
            FunctionDecl => ("function", None),
            VarDecl => ("var", Some(VarSpec)),
            ConstDecl => ("constant", Some(ConstSpec)),
            TypeDecl => ("type", Some(TypeSpec)),
            ImportDecl => ("constant", Some(ImportSpec)),
            Parameter => ("var", None),
            kind => {
                if !is_scope(kind) {
                    pending.extend(node.children().rev());
                }
                continue;
            }
        };
        collect_defs(source, node, kind, spec, options);
    }
}

fn collect_defs(
    source: &str,
    node: GoNode<'_>,
    kind: &str,
    spec: Option<GoNodeKind>,
    options: &mut Vec<CompletionItem>,
) {
    for child in node.children() {
        if child.kind() == DefName {
            options.push(item(&source[child.range()], kind));
        } else if child.kind() == SpecList || Some(child.kind()) == spec {
            collect_defs(source, child, kind, spec, options);
        }
    }
}

fn item(label: &str, kind: &str) -> CompletionItem {
    let mut option = CompletionItem::new(label);
    option.kind = Some(kind.to_owned());
    option
}

fn ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let word = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$') || ch >= '\u{a1}';
    chars.next().is_some_and(word) && chars.all(word)
}
