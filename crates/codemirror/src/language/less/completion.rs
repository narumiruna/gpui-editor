use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;

use crate::{
    Text,
    autocomplete::session::{CompletionItem, SourceResult},
    language::css::{CssDocument, css_completion_source},
};

use super::{
    LessDocument, LessNode,
    LessNodeKind::{self, *},
    Side,
};

// Reuse CSS's option lists and validity rule. These fixed probes do not parse
// Less input as CSS. The Less tree alone selects a completion context.
static PROPERTIES: LazyLock<SourceResult> = LazyLock::new(|| css_options("a { color: red }", 8));
static VALUES: LazyLock<SourceResult> = LazyLock::new(|| css_options("a { color: red }", 13));
static PSEUDOS: LazyLock<SourceResult> = LazyLock::new(|| css_options("a:hover {}", 5));
static TAGS: LazyLock<SourceResult> = LazyLock::new(|| css_options("body {}", 2));
static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A-(?:-[A-Za-z0-9_-]*)?\z").unwrap());

fn css_options(source: &str, at: usize) -> SourceResult {
    css_completion_source(&CssDocument::parse(Text::new(source)), at, false)
        .expect("fixed CSS completion context")
}

fn fixed(from: usize, template: &SourceResult) -> SourceResult {
    let mut result = template.clone();
    result.from = from;
    result
}

fn variable(kind: LessNodeKind) -> bool {
    matches!(kind, VariableName | AtKeyword)
}

/// Return unfiltered Less completions at a UTF-8 byte position. CSS properties
/// use the native CSS source's fixed browser snapshot. As upstream, `@` names
/// are offered as variables; the reused validity rule only accepts dash prefixes.
pub fn less_completion_source(
    doc: &LessDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    doc.text().slice(position..position)?;
    let node = doc.path(position, Side::Left).last()?;
    let range = node.range();
    let from = range.start;
    let kind = node.kind();
    let dash = kind == Error && doc.text().slice(range.clone()) == Some("-");
    if kind == PropertyName
        || (dash || kind == TagName) && doc.path(range.end, Side::Inner).last()?.kind() == Block
    {
        return Some(fixed(from, &PROPERTIES));
    }
    if kind == ValueName {
        return Some(fixed(from, &VALUES));
    }
    if kind == PseudoClassName {
        return Some(fixed(from, &PSEUDOS));
    }
    if variable(kind) {
        return Some(variables(doc, from));
    }
    // The pinned Less grammar calls the function name ValueName, not Callee.
    // The base CSS source's special case for an empty `var()` does not apply.
    if kind == TagName {
        let in_block = std::iter::successors(node.parent(), |node| node.parent())
            .any(|node| node.kind() == Block);
        return Some(fixed(from, if in_block { &PROPERTIES } else { &TAGS }));
    }
    if !explicit {
        return None;
    }
    let above = doc.path(position, Side::Inner).last()?;
    let follows_colon = above
        .children()
        .rfind(|child| child.range().start < position)
        .is_some_and(|node| node.kind() == Colon);
    match above.kind() {
        PseudoClassSelector if follows_colon => Some(fixed(position, &PSEUDOS)),
        Declaration if follows_colon => Some(fixed(position, &VALUES)),
        ArgList => Some(fixed(position, &VALUES)),
        Block => Some(fixed(position, &PROPERTIES)),
        _ => None,
    }
}

fn variables(doc: &LessDocument, from: usize) -> SourceResult {
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    for declaration in doc.nodes().filter(|node| node.kind() == Declaration) {
        let mut children = declaration.children().peekable();
        while let Some(node) = children.next() {
            if variable(node.kind())
                && children
                    .peek()
                    .is_some_and(|node: &LessNode<'_>| node.kind() == Colon)
            {
                let label = &doc.text().as_str()[node.range()];
                if seen.insert(label) {
                    let mut option = CompletionItem::new(label);
                    option.kind = Some("variable".to_owned());
                    options.push(option);
                }
            }
        }
    }
    SourceResult::new(from, options).with_valid_for(|prefix, _, _| VARIABLE.is_match(prefix))
}
