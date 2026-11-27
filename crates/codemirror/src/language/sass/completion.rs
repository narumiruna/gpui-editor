// Completion rules from @codemirror/lang-css 6.3.1 as configured by lang-sass.
// See docs/licenses/codemirror-lang-sass.txt and docs/licenses/codemirror-lang-css.txt.
use std::{collections::HashSet, sync::LazyLock};

use crate::{
    Text,
    autocomplete::session::{CompletionItem, SourceResult},
};

use super::super::css::{CssDocument, define_css_completion_source};
use super::{SassDocument, SassNode};

#[derive(Clone, Copy)]
enum Options {
    Properties,
    Values,
    Pseudos,
    Tags,
    AtRules,
}

impl Options {
    fn result(self, from: usize) -> SourceResult {
        // CSS's public helper takes a CSS document, not a generic syntax tree.
        // Get its fixed option sets from small CSS contexts. All queries of
        // the actual source (including variables) use the Sass tree.
        static RESULTS: LazyLock<[SourceResult; 5]> = LazyLock::new(|| {
            let css = define_css_completion_source(|_| false);
            [
                ("a { color: red; }", 9),
                ("a { color: red; }", 14),
                ("a:hover {}", 7),
                ("div {}", 3),
                ("@unknown;", 8),
            ]
            .map(|(text, at)| {
                css(&CssDocument::parse(Text::new(text)), at, false)
                    .expect("published CSS completion for fixed context")
            })
        });
        let mut result = RESULTS[self as usize].clone();
        result.from = from;
        result
    }
}

/// Return unfiltered options selected by the Sass syntax at a UTF-8 position.
/// Property names come from the native CSS browser snapshot.
/// Malformed input can give different results from upstream Lezer recovery.
pub fn sass_completion_source(
    doc: &SassDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    doc.text().slice(position..position)?;
    let node = doc.path(position, -1).last()?;
    let range = node.range();
    let from = range.start;
    let name = node.name();
    let is_dash = node.is_error() && doc.text().slice(range.clone()) == Some("-");
    if name == "PropertyName"
        || (is_dash || name == "TagName") && doc.path(range.end, 0).last()?.name() == "Block"
    {
        return Some(Options::Properties.result(from));
    }
    if name == "ValueName" {
        return Some(Options::Values.result(from));
    }
    if name == "PseudoClassName" {
        return Some(Options::Pseudos.result(from));
    }
    if is_variable(node) || (explicit || is_dash) && is_var_arg(doc, node) {
        return Some(variables(
            doc,
            if is_variable(node) || is_dash {
                from
            } else {
                position
            },
        ));
    }
    if name == "TagName" {
        let in_block = std::iter::successors(node.parent(), |parent| parent.parent())
            .any(|parent| parent.name() == "Block");
        return Some(
            if in_block {
                Options::Properties
            } else {
                Options::Tags
            }
            .result(from),
        );
    }
    if name == "AtKeyword" {
        return Some(Options::AtRules.result(from));
    }
    if !explicit {
        return None;
    }
    let above = doc.path(position, 0).last()?;
    let after_colon = above
        .children()
        .rfind(|child| child.range().start < position)
        .is_some_and(|child| child.name() == ":");
    Some(
        match above.name() {
            "PseudoClassSelector" if after_colon => Options::Pseudos,
            "Declaration" if after_colon => Options::Values,
            "ArgList" => Options::Values,
            "Block" => Options::Properties,
            _ => return None,
        }
        .result(position),
    )
}

fn is_variable(node: SassNode<'_>) -> bool {
    matches!(node.name(), "VariableName" | "SassVariableName")
}

fn is_var_arg(doc: &SassDocument, mut node: SassNode<'_>) -> bool {
    if node.name() == "(" || node.is_error() {
        node = node.parent().unwrap_or(node);
    }
    node.name() == "ArgList"
        && node
            .parent()
            .and_then(|parent| parent.children().next())
            .is_some_and(|callee| {
                callee.name() == "Callee" && doc.text().slice(callee.range()) == Some("var")
            })
}

fn variables(doc: &SassDocument, from: usize) -> SourceResult {
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    for decl in doc.nodes().filter(|node| node.name() == "Declaration") {
        for (name, next) in decl.children().zip(decl.children().skip(1)) {
            if is_variable(name) && next.name() == ":" {
                let label = &doc.text().as_str()[name.range()];
                if seen.insert(label) {
                    let mut item = CompletionItem::new(label);
                    item.kind = Some("variable".into());
                    options.push(item);
                }
            }
        }
    }
    // The published CSS source applies this rule even to Sass variables.
    // A $ prefix thus requests new options instead of keeping this result.
    SourceResult::new(from, options).with_valid_for(|text, _, _| {
        text == "-"
            || text.strip_prefix("--").is_some_and(|rest| {
                rest.bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
            })
    })
}
