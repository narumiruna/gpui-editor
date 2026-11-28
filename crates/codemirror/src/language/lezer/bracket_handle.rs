use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, Text,
    language::{
        brackets::{
            BracketDirection, BracketMatch, BracketMates, named_delimiter, named_sibling,
            node_delimiter,
        },
        line_projection::splits_crlf,
        markdown::{MarkdownCodeParser, MarkdownMount},
    },
    language_data::NativeSyntax,
};

use super::{
    LrSyntaxNode, NodeProp,
    service_node::{Coordinates, Syntax},
};

/// Select the part of a marked delimiter to probe and highlight. Returning
/// `None` skips it as a marked start and omits its highlight as a mate.
pub type LrBracketHandle =
    Arc<dyn for<'a> Fn(LrSyntaxNode<'a>) -> Option<LrSyntaxNode<'a>> + Send + Sync>;

/// A handle selector for a C/C++, JSON, Lezer grammar, or Rust row. If there is no
/// selector, the entire delimiter is the handle. A selected node must be
/// nonempty and come from the same tree. C/C++ handles use original document
/// offsets. WAST has a separate property.
pub static BRACKET_MATCHING_HANDLE: LazyLock<NodeProp<Option<LrBracketHandle>>> =
    LazyLock::new(NodeProp::new);

pub(in crate::language) fn match_lr_marked(
    text: &Text,
    node: LrSyntaxNode<'_>,
    pos: usize,
    direction: BracketDirection,
    pairs: &str,
) -> Option<BracketMatch> {
    for token in std::iter::successors(Some(node), |node| node.parent()) {
        let range = token.range();
        if range.is_empty() {
            continue;
        }
        let Some(mates) = names(token, direction, pairs) else {
            continue;
        };
        let Some(start) = handle(text, token).filter(|h| match direction {
            BracketDirection::Forward => h.contains(&pos),
            BracketDirection::Backward => h.start < pos && pos <= h.end,
        }) else {
            continue;
        };
        let found = named_sibling(
            &range,
            direction,
            mates,
            token.parent().into_iter().flat_map(LrSyntaxNode::children),
            |sibling| (sibling.range(), sibling.name()),
            |sibling, dir| names(sibling, dir, pairs).is_some(),
        );
        return Some(BracketMatch {
            start,
            end: found.and_then(|(mate, _)| handle(text, mate)),
            matched: found.is_some_and(|(_, matched)| matched),
        });
    }
    None
}

impl MarkdownMount {
    pub(crate) fn lr_bracket_node(&self, local: usize, side: Assoc) -> Option<LrSyntaxNode<'_>> {
        let node = match self.syntax() {
            NativeSyntax::Json(doc) => LrSyntaxNode::json(doc.node_at(local, Some(side))?),
            NativeSyntax::LezerGrammar(doc) => {
                LrSyntaxNode::lezer(doc.resolve_at(local, Some(side)))
            }
            NativeSyntax::Rust(doc)
                if matches!(self.code_parser(), Some(MarkdownCodeParser::Rust(_))) =>
            {
                LrSyntaxNode::rust(doc.bracket_node(local, side)?)
            }
            _ => return None,
        };
        Some(node.in_mount(self.source_map()))
    }
}

impl LrSyntaxNode<'_> {
    pub(in crate::language) fn same_mounted_bracket_type(self, other: Self) -> bool {
        // Do not pair plain Rust delimiters across independent fences.
        (!matches!(self.syntax, Syntax::Rust(_)) || self.same_tree(other))
            && self.same_syntax_type(other)
    }
}

fn names<'a>(
    node: LrSyntaxNode<'a>,
    dir: BracketDirection,
    pairs: &'a str,
) -> Option<BracketMates<'a>> {
    if let Some(row) = node.node_type() {
        return node_delimiter(row, dir, pairs);
    }
    // Local Lezer and Rust recovery can leave a delimiter without a grammar term.
    named_delimiter(node.name(), dir, "()[]{}")
        .or_else(|| named_delimiter(node.name(), dir, pairs))
        .map(Into::into)
}

fn handle(text: &Text, token: LrSyntaxNode<'_>) -> Option<Range<usize>> {
    let node = match token
        .node_type()
        .and_then(|row| row.prop(&BRACKET_MATCHING_HANDLE))
    {
        Some(Some(select)) => select(token)?,
        _ => token,
    };
    if !node.same_tree(token)
        || (matches!(token.syntax, Syntax::Cpp(_))
            && !matches!(node.coordinates, Coordinates::Linear(0)))
    {
        return None;
    }
    let range = node.range();
    (!range.is_empty()
        && text.is_valid_range(range.clone())
        && !splits_crlf(text.as_str(), range.start)
        && !splits_crlf(text.as_str(), range.end))
    .then_some(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EditorState,
        language::{
            brackets::BracketConfig,
            cpp::CppDocument,
            json::JsonDocument,
            lezer::{CLOSED_BY, OPENED_BY},
        },
        language_data::{NativeSyntax, attached_language_snapshot, cpp_language, json_language},
    };

    #[test]
    fn cpp_rejects_foreign_and_remapped_handles_with_valid_byte_ranges() {
        const SOURCE: &str = "int f() { g((12), (34)); }";
        static FOREIGN: LazyLock<CppDocument> =
            LazyLock::new(|| CppDocument::parse(Text::new(SOURCE)));
        let first = SOURCE.find("12").unwrap();
        let second = SOURCE.find("34").unwrap();
        let foreign: LrBracketHandle = Arc::new(move |node| {
            let handle = node.children().nth(1)?;
            if handle.range().start != second {
                return Some(handle);
            }
            FOREIGN
                .nodes()
                .find(|token| token.range() == (second..second + 2))
                .map(LrSyntaxNode::cpp)
        });
        let remapped: LrBracketHandle = Arc::new(move |node| {
            let mut handle = node.children().nth(1)?;
            if handle.range().start == second {
                handle.coordinates = Coordinates::Linear(1);
            }
            Some(handle)
        });
        for select in [foreign, remapped] {
            let language = cpp_language().with_node_props(&[
                CLOSED_BY.add(|row| (row.term() == 102).then(|| vec![row.name().into()])),
                OPENED_BY.add(|row| (row.term() == 102).then(|| vec![row.name().into()])),
                BRACKET_MATCHING_HANDLE
                    .add(move |row| (row.term() == 102).then(|| Some(select.clone()))),
            ]);
            let state = EditorState::with_extensions(SOURCE, language.extension()).unwrap();
            let snapshot = attached_language_snapshot(&state).unwrap();
            let NativeSyntax::Cpp(doc) = snapshot.syntax() else {
                unreachable!()
            };
            let config = BracketConfig {
                brackets: "<>",
                ..Default::default()
            };
            let number = doc
                .nodes()
                .find(|node| node.range() == (first..first + 2))
                .unwrap();
            assert_eq!(
                match_lr_marked(
                    doc.text(),
                    LrSyntaxNode::cpp(number),
                    first + 1,
                    BracketDirection::Forward,
                    "<>"
                ),
                Some(BracketMatch {
                    start: first..first + 2,
                    end: None,
                    matched: true,
                })
            );
            assert_eq!(doc.bracket_near_with_config(second + 1, config), None);
        }
    }

    #[test]
    fn rejects_a_handle_from_a_foreign_tree_even_with_the_same_offsets() {
        const SOURCE: &str = "[ 12, 34 ]";
        static FOREIGN: LazyLock<JsonDocument> =
            LazyLock::new(|| JsonDocument::parse(Text::new(SOURCE)));
        let foreign: LrBracketHandle =
            Arc::new(|_| Some(LrSyntaxNode::json(FOREIGN.node_at(3, None)?)));
        let configured = json_language().with_node_props(&[
            CLOSED_BY.add(|row| row.is("Number").then(|| vec!["Number".into()])),
            BRACKET_MATCHING_HANDLE.add(move |row| row.is("Number").then(|| Some(foreign.clone()))),
        ]);
        let parsed = configured
            .parse_for_editor(Text::new(SOURCE), Default::default())
            .unwrap();
        let NativeSyntax::Json(doc) = parsed.syntax() else {
            unreachable!()
        };
        assert_eq!(
            doc.match_bracket_at(3, BracketDirection::Forward, BracketConfig::default()),
            None
        );
    }
}
