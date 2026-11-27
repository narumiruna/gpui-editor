use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, EditorState, Extension, Facet,
    commands::CommentTokens,
    extension::FacetId,
    language::{LanguageData, lezer::SyntaxTree},
    language_data::CloseBracketData,
};

use super::{
    BLOCK_COMMENT, LINE_COMMENT, Node, SUBLANGUAGES, WastDocument, WastNode, WastNodeType,
    WastNodeTypes, parser,
};

type DataFacet = Facet<Arc<LanguageData>, Vec<Arc<LanguageData>>>;

/// A facet for language data on a WAST root or sublanguage. Each definition
/// has its own identity; clones refer to the same installed entries.
#[derive(Clone)]
pub struct WastDataFacet {
    facet: Arc<DataFacet>,
    base: Option<Arc<LanguageData>>,
}

impl WastDataFacet {
    /// Define a facet with optional default data, ordered after installed entries.
    pub fn new(base: Option<LanguageData>) -> Self {
        let base = base.map(Arc::new);
        let default = base.clone();
        let facet = Arc::new(Facet::new(move |entries: &[&Arc<LanguageData>]| {
            entries
                .iter()
                .copied()
                .chain(default.iter())
                .cloned()
                .collect()
        }));
        Self { facet, base }
    }

    pub(crate) fn base(&self) -> Option<&Arc<LanguageData>> {
        self.base.as_ref()
    }

    /// Install a data entry at the extension's precedence.
    pub fn of(&self, data: LanguageData) -> Extension {
        self.facet.of(Arc::new(data))
    }

    pub(crate) fn id(&self) -> FacetId {
        self.facet.id()
    }

    pub(crate) fn stock() -> &'static Self {
        static STOCK: LazyLock<WastDataFacet> =
            LazyLock::new(|| WastDataFacet::new(Some(stock_data())));
        &STOCK
    }

    pub(crate) fn read(&self, state: &EditorState) -> Vec<Arc<LanguageData>> {
        state
            .context()
            .facet(&self.facet)
            .expect("typed WAST data")
            .as_ref()
            .clone()
    }
}

impl PartialEq for WastDataFacet {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.facet, &other.facet)
    }
}

impl Eq for WastDataFacet {}

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
/// innermost node resolved at the query offset and side, and the current state.
/// This changes data, not syntax, presence, edit ownership, or mounts.
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
            origin: 0,
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
