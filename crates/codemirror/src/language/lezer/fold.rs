use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{EditorState, language_data::NativeSyntax};

use super::{NodeProp, NodeType, NodeTypes};
use crate::language::{
    fold::fold_on_line,
    json::{JsonNode, JsonNodeKind},
    lezer_grammar::{LezerNode, LezerNodeKind},
    line_projection::splits_crlf,
    markdown::MarkdownSourceMap,
};

/// A JSON or Lezer grammar fold rule. It reads the current editor state and
/// returns a range in that state's original UTF-8 bytes.
pub type LrFoldCallback =
    Arc<dyn for<'a> Fn(LrFoldNode<'a>, &EditorState) -> Option<Range<usize>> + Send + Sync>;

/// The fold rule on a verified JSON or Lezer grammar row. An explicit `None`
/// disables the stock rule. The WAST API uses its own `wast::FOLD` key.
pub static FOLD: LazyLock<NodeProp<Option<LrFoldCallback>>> = LazyLock::new(NodeProp::new);

#[derive(Clone, Copy)]
enum Syntax<'a> {
    Json(JsonNode<'a>),
    Lezer(LezerNode<'a>),
}

#[derive(Clone, Copy, Debug)]
enum Coordinates<'a> {
    Linear(usize),
    Mount(MarkdownSourceMap<'a>),
}

impl Coordinates<'_> {
    fn source(self, local: usize) -> usize {
        match self {
            Self::Linear(origin) => origin + local,
            Self::Mount(map) => map.source_offset(local),
        }
    }
}

/// A borrowed fold candidate. Node ranges use host byte offsets, including
/// when the parser receives a detached code body or disjoint mount segments.
#[derive(Clone, Copy)]
pub struct LrFoldNode<'a> {
    syntax: Syntax<'a>,
    coordinates: Coordinates<'a>,
}

impl<'a> LrFoldNode<'a> {
    fn json(node: JsonNode<'a>) -> Self {
        Self {
            syntax: Syntax::Json(node),
            coordinates: Coordinates::Linear(0),
        }
    }

    fn lezer(node: LezerNode<'a>) -> Self {
        Self {
            syntax: Syntax::Lezer(node),
            coordinates: Coordinates::Linear(0),
        }
    }

    pub fn name(self) -> &'static str {
        match self.syntax {
            Syntax::Json(node) => node.kind().name(),
            Syntax::Lezer(node) => node.kind().name(),
        }
    }

    pub fn node_type(self) -> Option<&'a NodeType> {
        match self.syntax {
            Syntax::Json(node) => Some(node.node_type()),
            Syntax::Lezer(node) => node.node_type(),
        }
    }

    pub fn range(self) -> Range<usize> {
        let local = match self.syntax {
            Syntax::Json(node) => node.range(),
            Syntax::Lezer(node) => node.range(),
        };
        self.coordinates.source(local.start)..self.coordinates.source(local.end)
    }

    pub fn parent(self) -> Option<Self> {
        let syntax = match self.syntax {
            Syntax::Json(node) => Syntax::Json(node.parent()?),
            Syntax::Lezer(node) => Syntax::Lezer(node.parent()?),
        };
        Some(Self { syntax, ..self })
    }

    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        (0..self.child_count()).map(move |index| self.child(index).expect("verified child index"))
    }

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

    fn is_error(self) -> bool {
        match self.syntax {
            Syntax::Json(node) => node.kind() == JsonNodeKind::Error,
            Syntax::Lezer(node) => node.kind() == LezerNodeKind::Error,
        }
    }

    fn child_count(self) -> usize {
        match self.syntax {
            Syntax::Json(node) => node.children().len(),
            Syntax::Lezer(node) => node.children().len(),
        }
    }

    fn child(self, index: usize) -> Option<Self> {
        let syntax = match self.syntax {
            Syntax::Json(node) => Syntax::Json(node.children().nth(index)?),
            Syntax::Lezer(node) => Syntax::Lezer(node.children().nth(index)?),
        };
        Some(Self { syntax, ..self })
    }

    fn resolve_at(mut self, position: usize) -> Self {
        loop {
            let mut first = 0;
            let mut end = self.child_count();
            while first < end {
                let mid = first + (end - first) / 2;
                if self.child(mid).unwrap().range().end <= position {
                    first = mid + 1;
                } else {
                    end = mid;
                }
            }
            let Some(child) = self
                .child(first)
                .filter(|node| node.range().start <= position)
            else {
                return self;
            };
            self = child;
        }
    }

    fn configured_fold(self, state: &EditorState) -> Option<Range<usize>> {
        if let Some(row) = self.node_type() {
            return row.prop(&FOLD)?.as_ref()?(self, state);
        }
        // Only the local Lezer fallback can lack a verified row.
        let Syntax::Lezer(node) = self.syntax else {
            return None;
        };
        let range = node.fold_range()?;
        Some(self.coordinates.source(range.start)..self.coordinates.source(range.end))
    }

    fn fold_in_mount(
        mut self,
        state: &EditorState,
        map: MarkdownSourceMap<'a>,
        line: Range<usize>,
    ) -> Option<Range<usize>> {
        let body = map.bounds();
        if !body.contains(&line.end) {
            return None;
        }
        self.coordinates = if map.is_contiguous() {
            Coordinates::Linear(body.start)
        } else {
            Coordinates::Mount(map)
        };
        self.fold_on_line(state, line, |at| map.contains_endpoint(at))
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
        let node = self.resolve_at(line.end);
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
            _ => false,
        }
    }

    fn lr_fold_root(&self) -> Option<LrFoldNode<'_>> {
        match self {
            Self::Json(doc) => Some(LrFoldNode::json(doc.root())),
            Self::LezerGrammar(doc) => Some(LrFoldNode::lezer(doc.root())),
            _ => None,
        }
    }

    /// Query a configured JSON, Lezer, or WAST fold in a contiguous body that
    /// exactly matches this syntax snapshot. Use original parent byte offsets.
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
        let mut root = self.lr_fold_root()?;
        if line.start < body.start
            || line.end > body.end
            || line.start > line.end
            || state.text().slice_owned(body.clone())? != *self.text()
        {
            return None;
        }
        let origin = body.start;
        if !self.has_configured_folds() {
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
        self.lr_fold_root()?.fold_in_mount(state, map, line)
    }
}

pub(in crate::language) fn stock_fold() -> LrFoldCallback {
    static INSIDE: LazyLock<LrFoldCallback> =
        LazyLock::new(|| Arc::new(|node, _| node.fold_inside()));
    INSIDE.clone()
}
