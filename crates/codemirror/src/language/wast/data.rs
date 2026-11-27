use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, EditorState,
    commands::CommentTokens,
    language::{LanguageData, lezer::SyntaxTree},
    language_data::CloseBracketData,
};

use super::{
    BLOCK_COMMENT, LINE_COMMENT, Node, SUBLANGUAGES, WastDocument, WastNode, WastNodeType,
    WastNodeTypes, parser,
};

pub use crate::language::lezer::LanguageFacet as WastDataFacet;

impl WastDataFacet {
    pub(crate) fn stock() -> &'static Self {
        static STOCK: LazyLock<WastDataFacet> =
            LazyLock::new(|| WastDataFacet::new(Some(stock_data())));
        &STOCK
    }
}

/// How a matched sublanguage contributes its data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WastSublanguageMode {
    /// Put the sublanguage entries before the root entries.
    Extend,
    /// Return only the sublanguage entries, even if the facet is empty.
    Replace,
}

type SublanguageTest = dyn Fn(WastNode<'_>, &EditorState) -> bool + Send + Sync;

/// One positional data rule for a WAST root. The predicate receives the
/// resolved WAST node and the current state. Markdown uses CodeMirror's
/// mounted query offset and exposes node ranges in host bytes. This changes
/// data, not syntax, presence, edit ownership, or mounts.
#[derive(Clone)]
pub struct WastSublanguage {
    mode: WastSublanguageMode,
    facet: WastDataFacet,
    test: Arc<SublanguageTest>,
}

impl WastSublanguage {
    pub fn new(
        mode: WastSublanguageMode,
        facet: WastDataFacet,
        test: impl Fn(WastNode<'_>, &EditorState) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            mode,
            facet,
            test: Arc::new(test),
        }
    }
}

impl WastDocument {
    pub(crate) fn configured_data_at(
        &self,
        state: &EditorState,
        position: usize,
        side: Assoc,
    ) -> Vec<Arc<LanguageData>> {
        let root = self.root();
        root.node_type()
            .configured_data_at(state, || Some(root.resolve_at(position, Some(side))))
    }

    pub(crate) fn configured_data_in_markdown(
        &self,
        state: &EditorState,
        map: super::MarkdownSourceMap<'_>,
        position: impl FnOnce() -> usize,
        side: Assoc,
    ) -> Vec<Arc<LanguageData>> {
        let root = self.root().in_markdown(map);
        root.node_type()
            .configured_data_at(state, || Some(root.resolve_at(position(), Some(side))))
    }
}

impl WastNodeType {
    pub(crate) fn configured_data_at<'a>(
        &self,
        state: &EditorState,
        resolve: impl FnOnce() -> Option<WastNode<'a>>,
    ) -> Vec<Arc<LanguageData>> {
        let Some(facet) = self.data_facet() else {
            return Vec::new();
        };
        let base = facet.read(state);
        let Some(rules) = self.prop(&SUBLANGUAGES).filter(|rules| !rules.is_empty()) else {
            return base;
        };
        let Some(node) = resolve() else {
            return base;
        };
        let Some(rule) = rules.iter().find(|rule| (rule.test)(node, state)) else {
            return base;
        };
        let mut selected = rule.facet.read(state);
        if rule.mode == WastSublanguageMode::Extend {
            selected.extend(base);
        }
        selected
    }
}

/// Indexed nodes for callbacks that need parent navigation on a viewport tree.
/// Build only when a positional rule needs the published tree.
pub(crate) struct WastDataNodes {
    nodes: Vec<Node>,
    node_types: WastNodeTypes,
}

impl WastDataNodes {
    pub(crate) fn new(tree: &SyntaxTree) -> Self {
        Self {
            nodes: parser::convert(tree, core::convert::identity),
            node_types: tree
                .root()
                .tree_metadata::<WastNodeTypes>()
                .expect("viewport WAST node types")
                .clone(),
        }
    }

    pub(crate) fn resolve_at(&self, position: usize, side: Assoc) -> WastNode<'_> {
        WastNode {
            nodes: &self.nodes,
            node_types: &self.node_types,
            id: 0,
            coordinates: super::Coordinates::Linear(0),
        }
        .resolve_at(position, Some(side))
    }
}

fn stock_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: Some(BLOCK_COMMENT),
            },
        )
        .with(
            "closeBrackets",
            CloseBracketData {
                brackets: Some(&["(", "\""]),
                ..Default::default()
            },
        )
}
