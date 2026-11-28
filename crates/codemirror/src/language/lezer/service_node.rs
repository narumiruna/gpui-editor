use std::ops::Range;

use crate::{
    language::{
        cpp::CppNode, json::JsonNode, lezer_grammar::LezerNode, markdown::MarkdownSourceMap,
        rust::RustNode,
    },
    language_data::NativeSyntax,
};

use super::NodeType;

#[derive(Clone, Copy)]
pub(super) enum Syntax<'a> {
    Cpp(CppNode<'a>),
    Json(JsonNode<'a>),
    Lezer(LezerNode<'a>),
    Rust(RustNode<'a>),
}

#[derive(Clone, Copy)]
pub(super) enum Coordinates<'a> {
    Linear(usize),
    Mount(MarkdownSourceMap<'a>),
}

impl Coordinates<'_> {
    pub(super) fn source(self, local: usize) -> usize {
        match self {
            Self::Linear(origin) => origin + local,
            Self::Mount(map) => map.source_offset(local),
        }
    }
}

/// A borrowed LR syntax node for an editor service. Ranges use original
/// UTF-8 bytes, including when the code body has Markdown source gaps.
#[derive(Clone, Copy)]
pub struct LrSyntaxNode<'a> {
    pub(super) syntax: Syntax<'a>,
    pub(super) coordinates: Coordinates<'a>,
}

impl<'a> LrSyntaxNode<'a> {
    pub(in crate::language) fn cpp(node: CppNode<'a>) -> Self {
        Self {
            syntax: Syntax::Cpp(node),
            coordinates: Coordinates::Linear(0),
        }
    }

    pub(in crate::language) fn json(node: JsonNode<'a>) -> Self {
        Self {
            syntax: Syntax::Json(node),
            coordinates: Coordinates::Linear(0),
        }
    }

    pub(in crate::language) fn lezer(node: LezerNode<'a>) -> Self {
        Self {
            syntax: Syntax::Lezer(node),
            coordinates: Coordinates::Linear(0),
        }
    }

    pub(in crate::language) fn rust(node: RustNode<'a>) -> Self {
        Self {
            syntax: Syntax::Rust(node),
            coordinates: Coordinates::Linear(0),
        }
    }

    pub(super) fn in_mount(self, map: MarkdownSourceMap<'a>) -> Self {
        let coordinates = if map.is_contiguous() {
            Coordinates::Linear(map.bounds().start)
        } else {
            Coordinates::Mount(map)
        };
        Self {
            coordinates,
            ..self
        }
    }

    pub fn name(self) -> &'static str {
        match self.syntax {
            Syntax::Cpp(node) => node.node_type().name(),
            Syntax::Json(node) => node.kind().name(),
            Syntax::Lezer(node) => node.kind().name(),
            Syntax::Rust(node) => node.kind().name(),
        }
    }

    /// Local recovery nodes without a verified grammar term have no row.
    pub fn node_type(self) -> Option<&'a NodeType> {
        match self.syntax {
            Syntax::Cpp(node) => Some(node.node_type()),
            Syntax::Json(node) => Some(node.node_type()),
            Syntax::Lezer(node) => node.node_type(),
            Syntax::Rust(node) => node.node_type(),
        }
    }

    pub fn range(self) -> Range<usize> {
        let local = match self.syntax {
            Syntax::Cpp(node) => node.range(),
            Syntax::Json(node) => node.range(),
            Syntax::Lezer(node) => node.range(),
            Syntax::Rust(node) => node.range(),
        };
        self.coordinates.source(local.start)..self.coordinates.source(local.end)
    }

    pub fn parent(self) -> Option<Self> {
        let syntax = match self.syntax {
            Syntax::Cpp(node) => Syntax::Cpp(node.parent()?),
            Syntax::Json(node) => Syntax::Json(node.parent()?),
            Syntax::Lezer(node) => Syntax::Lezer(node.parent()?),
            Syntax::Rust(node) => Syntax::Rust(node.parent()?),
        };
        Some(Self { syntax, ..self })
    }

    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        (0..self.child_count()).map(move |index| self.child(index).expect("verified child index"))
    }

    pub(in crate::language) fn same_syntax_type(self, other: Self) -> bool {
        match (self.node_type(), other.node_type()) {
            (Some(a), Some(b)) => std::ptr::eq(a, b),
            (None, None) => self.same_tree(other) && self.name() == other.name(),
            _ => false,
        }
    }

    pub(super) fn same_tree(self, other: Self) -> bool {
        match (self.syntax, other.syntax) {
            (Syntax::Cpp(this), Syntax::Cpp(other)) => this.same_tree(other),
            (Syntax::Json(this), Syntax::Json(other)) => this.same_tree(other),
            (Syntax::Lezer(this), Syntax::Lezer(other)) => this.same_tree(other),
            (Syntax::Rust(this), Syntax::Rust(other)) => this.same_tree(other),
            _ => false,
        }
    }

    pub(super) fn is_error(self) -> bool {
        self.name() == "⚠"
    }

    fn child_count(self) -> usize {
        match self.syntax {
            Syntax::Cpp(node) => node.children().len(),
            Syntax::Json(node) => node.children().len(),
            Syntax::Lezer(node) => node.children().len(),
            Syntax::Rust(node) => node.children().len(),
        }
    }

    fn child(self, index: usize) -> Option<Self> {
        let syntax = match self.syntax {
            Syntax::Cpp(node) => Syntax::Cpp(node.children().nth(index)?),
            Syntax::Json(node) => Syntax::Json(node.children().nth(index)?),
            Syntax::Lezer(node) => Syntax::Lezer(node.children().nth(index)?),
            Syntax::Rust(node) => Syntax::Rust(node.children().nth(index)?),
        };
        Some(Self { syntax, ..self })
    }

    pub(super) fn resolve_at(mut self, position: usize, include_start: bool) -> Self {
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
            let Some(child) = self.child(first).filter(|node| {
                let start = node.range().start;
                start < position || (include_start && start == position)
            }) else {
                return self;
            };
            self = child;
        }
    }

    pub(super) fn unfinished_before(mut self, position: usize) -> Self {
        let mut before = 0;
        let mut end = self.child_count();
        while before < end {
            let mid = before + (end - before) / 2;
            if self.child(mid).unwrap().range().start < position {
                before = mid + 1;
            } else {
                end = mid;
            }
        }
        let mut scan = before.checked_sub(1).and_then(|i| self.child(i));
        while let Some(node) = scan {
            let mut children = node.children();
            let Some(last) = children
                .next_back()
                .filter(|last| last.range().end == node.range().end)
            else {
                break;
            };
            if last.is_error() && last.range().is_empty() {
                self = node;
                scan = children.next_back();
            } else {
                scan = Some(last);
            }
        }
        self
    }
}

impl NativeSyntax {
    pub(super) fn lr_root(&self) -> Option<LrSyntaxNode<'_>> {
        match self {
            Self::Json(doc) => Some(LrSyntaxNode::json(doc.root())),
            Self::LezerGrammar(doc) => Some(LrSyntaxNode::lezer(doc.root())),
            _ => None,
        }
    }
}
