use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{EditorState, language_data::NativeSyntax};

use super::{
    NodeProp, NodeTypes,
    service_node::{Coordinates, LrSyntaxNode, Syntax},
};
use crate::language::{
    cpp::{CppDocument, stock_node_types as stock_cpp_node_types},
    fold::fold_on_line,
    line_projection::splits_crlf,
    markdown::MarkdownSourceMap,
    rust::stock_node_types as stock_rust_node_types,
};

/// An LR fold rule. It reads the current editor state and returns a range in
/// that state's original UTF-8 bytes.
pub type LrFoldCallback =
    Arc<dyn for<'a> Fn(LrFoldNode<'a>, &EditorState) -> Option<Range<usize>> + Send + Sync>;

/// A fold rule on a verified C/C++, JSON, Lezer, or Rust grammar row. An explicit
/// `None` disables the stock rule. The WAST API uses its own `wast::FOLD` key.
pub static FOLD: LazyLock<NodeProp<Option<LrFoldCallback>>> = LazyLock::new(NodeProp::new);

/// An LR syntax node passed to a fold rule.
pub type LrFoldNode<'a> = LrSyntaxNode<'a>;

impl<'a> LrSyntaxNode<'a> {
    /// Fold between the first and last children. If the last child is an
    /// error, extend the fold to the end of this node.
    pub fn fold_inside(self) -> Option<Range<usize>> {
        let mut children = self.children();
        let first = children.next()?;
        let last = children.next_back()?;
        let from = first.range().end;
        let to = last.range().start;
        (from < to).then(|| {
            from..if last.is_error() {
                self.range().end
            } else {
                to
            }
        })
    }

    fn configured_fold(self, state: &EditorState) -> Option<Range<usize>> {
        if let Some(row) = self.node_type() {
            return row.prop(&FOLD)?.as_ref()?(self, state);
        }
        // Local fallback nodes keep their native rule without inferring a row.
        match self.syntax {
            Syntax::Lezer(node) => Some(self.source_fold(node.fold_range()?)),
            Syntax::Rust(_) => self.rust_fold_range(),
            Syntax::Cpp(_) | Syntax::Json(_) => None,
        }
    }

    fn source_fold(self, range: Range<usize>) -> Range<usize> {
        self.coordinates.source(range.start)..self.coordinates.source(range.end)
    }

    pub(in crate::language) fn rust_fold_range(self) -> Option<Range<usize>> {
        let Syntax::Rust(node) = self.syntax else {
            return None;
        };
        Some(self.source_fold(node.fold_range()?))
    }

    pub(in crate::language) fn cpp_fold_range(self) -> Option<Range<usize>> {
        let Syntax::Cpp(node) = self.syntax else {
            return None;
        };
        node.fold_range()
    }

    fn fold_in_mount(
        self,
        state: &EditorState,
        map: MarkdownSourceMap<'a>,
        line: Range<usize>,
    ) -> Option<Range<usize>> {
        if !map.bounds().contains(&line.end) {
            return None;
        }
        self.in_mount(map)
            .fold_on_line(state, line, |at| map.contains_endpoint(at))
    }

    fn fold_on_line(
        self,
        state: &EditorState,
        line: Range<usize>,
        contains: impl Fn(usize) -> bool,
    ) -> Option<Range<usize>> {
        let text = state.text();
        let boundary = |at| text.is_char_boundary(at) && !splits_crlf(text.as_str(), at);
        if line.start > line.end || !boundary(line.start) || !boundary(line.end) {
            return None;
        }
        let node = self.resolve_at(line.end, true);
        fold_on_line(
            std::iter::successors(Some(node), |node| node.parent()),
            line,
            Self::range,
            |node| {
                let range = node.configured_fold(state)?;
                [range.start, range.end]
                    .into_iter()
                    .all(|at| contains(at) && boundary(at))
                    .then_some(range)
            },
        )
    }
}

impl CppDocument {
    pub(in crate::language) fn foldable_in_state(
        &self,
        state: &EditorState,
        line: Range<usize>,
    ) -> Option<Range<usize>> {
        if !self
            .node_types()
            .has_different_folds(stock_cpp_node_types())
        {
            return self.foldable(line.start, line.end);
        }
        if self.text() != state.text() {
            return None;
        }
        LrSyntaxNode::cpp(self.root()).fold_on_line(state, line, |_| true)
    }
}

impl NodeTypes {
    pub(in crate::language) fn has_different_folds(&self, stock: &Self) -> bool {
        !self.same_set(stock)
            && self.iter().zip(stock.iter()).any(|(row, stock)| {
                match (
                    row.prop(&FOLD).and_then(Option::as_ref),
                    stock.prop(&FOLD).and_then(Option::as_ref),
                ) {
                    (Some(left), Some(right)) => !Arc::ptr_eq(left, right),
                    (None, None) => false,
                    _ => true,
                }
            })
    }
}

impl NativeSyntax {
    pub(in crate::language) fn has_configured_folds(&self) -> bool {
        match self {
            Self::Wast(_) => true,
            Self::Json(doc) => doc.has_configured_folds(),
            Self::LezerGrammar(doc) => doc.has_configured_folds(),
            Self::Rust(doc) => doc
                .node_types()
                .has_different_folds(stock_rust_node_types()),
            _ => false,
        }
    }

    /// Query a configured JSON, Lezer, Rust, or WAST fold in a contiguous body
    /// that exactly matches this syntax snapshot. Use original parent byte offsets.
    /// Use [`crate::language::fold::foldable`] to run editor-wide services first.
    pub fn foldable_in_scope(
        &self,
        state: &EditorState,
        body: Range<usize>,
        line: Range<usize>,
    ) -> Option<Range<usize>> {
        if let Self::Wast(doc) = self {
            return doc.foldable_in_scope(state, body, line.start, line.end);
        }
        let configured = self.has_configured_folds();
        let mut root = match self {
            Self::Rust(doc) => LrSyntaxNode::rust(doc.root()),
            _ => self.lr_root()?,
        };
        if line.start < body.start
            || line.end > body.end
            || line.start > line.end
            || state.text().slice_owned(body.clone())? != *self.text()
        {
            return None;
        }
        let origin = body.start;
        if !configured {
            let local = self.foldable(line.start - origin, line.end - origin)?;
            return Some(origin + local.start..origin + local.end);
        }
        root.coordinates = Coordinates::Linear(origin);
        root.fold_on_line(state, line, |at| (origin..=body.end).contains(&at))
    }

    pub(crate) fn foldable_in_markdown(
        &self,
        state: &EditorState,
        map: MarkdownSourceMap<'_>,
        line: Range<usize>,
    ) -> Option<Range<usize>> {
        if let Self::Wast(doc) = self {
            return doc.foldable_in_markdown(state, map, line.start, line.end);
        }
        let root = match self {
            Self::Rust(doc) => LrSyntaxNode::rust(doc.root()),
            _ => self.lr_root()?,
        };
        root.fold_in_mount(state, map, line)
    }
}

pub(in crate::language) fn stock_fold() -> LrFoldCallback {
    static INSIDE: LazyLock<LrFoldCallback> =
        LazyLock::new(|| Arc::new(|node, _| node.fold_inside()));
    INSIDE.clone()
}
