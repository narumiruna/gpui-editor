use std::ops::Range;

use super::{CommandResult, caret, select, selection_by};
use crate::{
    Assoc, EditorSelection, EditorState, SelectionRange,
    language::{
        brackets::{BracketConfig, BracketDirection, BracketMatch, match_bracket},
        lezer::{SyntaxNode, SyntaxTree},
    },
};

/// Move each cursor to the other bracket of a plain `()`, `[]`, or `{}` pair.
/// A bracket of a different type can be the other end of a pair.
pub fn cursor_matching_bracket(state: &EditorState) -> CommandResult {
    to_matching_bracket(state, false)
}

/// Move each selection head to the other bracket of a plain pair.
/// This does not use syntax classes or grammar-specific bracket handles.
pub fn select_matching_bracket(state: &EditorState) -> CommandResult {
    to_matching_bracket(state, true)
}

fn to_matching_bracket(state: &EditorState, extend: bool) -> CommandResult {
    use BracketDirection::{Backward, Forward};

    let text = state.text();
    let scan =
        |pos, direction| match_bracket(text, pos, direction, BracketConfig::default(), |_, _| true);
    let mut found = false;
    let selection = selection_by(state, |range| {
        let head = range.head();
        // The first bracket stops the search, even if it has no partner.
        let matching = scan(head, Backward)
            .or_else(|| scan(head, Forward))
            .or_else(|| scan(head.checked_sub(1)?, Forward))
            .or_else(|| scan(head.checked_add(1)?, Backward));
        let Some(BracketMatch {
            start,
            end: Some(end),
            ..
        }) = matching
        else {
            return range;
        };
        found = true;
        let head = if start.start == head {
            end.end
        } else {
            end.start
        };
        if extend {
            SelectionRange::new(range.anchor(), head)
        } else {
            caret(head)
        }
    })?;
    Ok(found.then(|| select(selection, true)))
}

/// Expand each range to the smallest larger non-root node on its syntax path.
/// Supply a tree for the current document. New ranges are undirectional.
pub fn select_parent_syntax(state: &EditorState, tree: &SyntaxTree) -> CommandResult {
    let root = tree.root();
    if root.range() != (0..state.text().len()) {
        return Ok(None);
    }
    let selection = selection_by(state, |range| {
        let span = range.range();
        let resolve = |side| {
            resolve_parent(
                root,
                ParentQuery {
                    selection: &span,
                    at: span.start,
                    side,
                    include_root: false,
                },
                SyntaxNode::range,
                SyntaxNode::children,
                Some,
            )
        };
        choose_parent(&span, resolve)
            .filter(|span| state.text().is_valid_range(span.clone()))
            .map_or(range, |span| {
                EditorSelection::undirectional_range(span.start, span.end)
            })
    })?;
    Ok((&selection != state.selection()).then(|| select(selection, true)))
}

pub(crate) struct ParentQuery<'a> {
    pub selection: &'a Range<usize>,
    pub at: usize,
    pub side: Assoc,
    pub include_root: bool,
}

#[derive(Default)]
pub(crate) struct ParentResolution {
    pub leaf: Option<Range<usize>>,
    pub larger: Option<Range<usize>>,
}

pub(crate) fn choose_parent(
    span: &Range<usize>,
    resolve: impl Fn(Assoc) -> ParentResolution,
) -> Option<Range<usize>> {
    let right = resolve(Assoc::After);
    if span.is_empty() {
        let left = resolve(Assoc::Before);
        if left.leaf.as_ref().is_some_and(|left| {
            right
                .leaf
                .as_ref()
                .is_none_or(|right| left.start >= right.start && left.end <= right.end)
        }) {
            return left.larger;
        }
    }
    right.larger
}

// The leaf decides the side of a caret. The candidate must also cover the
// entire selection. Mounted trees can map their nodes back to source here.
pub(crate) fn resolve_parent<N: Copy, I: IntoIterator<Item = N>>(
    mut node: N,
    query: ParentQuery<'_>,
    range: impl Fn(N) -> Range<usize>,
    children: impl Fn(N) -> I,
    source: impl Fn(Range<usize>) -> Option<Range<usize>>,
) -> ParentResolution {
    let mut result = ParentResolution::default();
    let mut eligible = query.include_root;
    loop {
        if let Some(span) = source(range(node)) {
            if eligible
                && span.start <= query.selection.start
                && span.end >= query.selection.end
                && span != *query.selection
            {
                result.larger = Some(span.clone());
            }
            result.leaf = Some(span);
        }
        let Some(child) = children(node).into_iter().find(|&child| {
            let span = range(child);
            match query.side {
                Assoc::Before => span.start < query.at && query.at <= span.end,
                Assoc::After => span.contains(&query.at),
            }
        }) else {
            break;
        };
        eligible = true;
        node = child;
    }
    result
}
