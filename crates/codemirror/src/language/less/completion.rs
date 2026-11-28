use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;

use crate::{
    autocomplete::session::{CompletionItem, SourceResult},
    language::css::{CssPropertySnapshot, completion::FixedOptions},
};

use super::{
    LessDocument, LessNode,
    LessNodeKind::{self, *},
    Side,
};

static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A-(?:-[A-Za-z0-9_-]*)?\z").unwrap());

fn variable(kind: LessNodeKind) -> bool {
    matches!(kind, VariableName | AtKeyword)
}

/// Return unfiltered Less completions at a UTF-8 byte position. CSS properties
/// use the pinned stock list. As upstream, `@` names are offered as variables;
/// the reused validity rule only accepts dash prefixes.
pub fn less_completion_source(
    doc: &LessDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    less_completion_source_with_properties(doc, position, explicit, CssPropertySnapshot::stock())
}

/// Run the Less source with these property options in place of the stock names.
pub fn less_completion_source_with_properties(
    doc: &LessDocument,
    position: usize,
    explicit: bool,
    properties: &CssPropertySnapshot,
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
        return Some(properties.result(from));
    }
    if kind == ValueName {
        return Some(FixedOptions::Values.result(from));
    }
    if kind == PseudoClassName {
        return Some(FixedOptions::Pseudos.result(from));
    }
    if variable(kind) {
        return Some(variables(doc, from));
    }
    // The pinned Less grammar calls the function name ValueName, not Callee.
    // The base CSS source's special case for an empty `var()` does not apply.
    if kind == TagName {
        let in_block = std::iter::successors(node.parent(), |node| node.parent())
            .any(|node| node.kind() == Block);
        return Some(if in_block {
            properties.result(from)
        } else {
            FixedOptions::Tags.result(from)
        });
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
        PseudoClassSelector if follows_colon => Some(FixedOptions::Pseudos.result(position)),
        Declaration if follows_colon => Some(FixedOptions::Values.result(position)),
        ArgList => Some(FixedOptions::Values.result(position)),
        Block => Some(properties.result(position)),
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
