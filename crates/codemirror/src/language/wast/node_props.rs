use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use super::{
    WastDataFacet, WastNode, WastNodeKind, WastSublanguage,
    indent::{WastIndentStrategy, app_indent},
    style_tags::stock_rules,
    tables,
};
pub use crate::language::lezer::{
    CLOSED_BY, GROUP, ISOLATE, Isolation as WastIsolation, LANGUAGE_DATA, NodeProp,
    NodePropSource as WastPropSource, NodeType as WastNodeType, OPENED_BY,
};
use crate::{
    EditorState,
    language::{
        HighlightContext,
        lezer::{NodeTypes, STYLE_RULES, SyntaxNode, SyntaxTree},
    },
    language_data::NativeLanguage,
};

/// Select the node to highlight for this delimiter. Returning `None` makes
/// this delimiter inactive as a start and omits its range as an endpoint.
pub type WastBracketHandle =
    Arc<dyn for<'a> Fn(WastNode<'a>) -> Option<WastNode<'a>> + Send + Sync>;

/// Optional selector for the highlighted part of a delimiter. With no
/// callback, the whole node is the handle. A selected node must be nonempty
/// and come from the same document.
pub static BRACKET_MATCHING_HANDLE: LazyLock<NodeProp<Option<WastBracketHandle>>> =
    LazyLock::new(NodeProp::new);

/// Indentation strategy for a WAST node type. By default, `App` indents its
/// contents one unit past its reference line and aligns its closing `)` with it.
pub static INDENT: LazyLock<NodeProp<WastIndentStrategy>> = LazyLock::new(NodeProp::new);

/// Propose a fold in original UTF-8 byte offsets for a configured node in the
/// current editor state. Node ranges refer to the same document as the state.
/// Return `None` to skip this node.
pub type WastFoldCallback =
    Arc<dyn Fn(WastNode<'_>, &EditorState) -> Option<Range<usize>> + Send + Sync>;

/// State-aware fold strategy. App and BlockComment have stock callbacks.
/// A source that returns `Some(None)` disables this row's rule, not its
/// ancestors' rules.
pub static FOLD: LazyLock<NodeProp<Option<WastFoldCallback>>> = LazyLock::new(NodeProp::new);

/// Ordered sublanguage rules for the WAST top node. Only the first matching
/// rule applies. The stock `Module` has no rules.
pub static SUBLANGUAGES: LazyLock<NodeProp<Vec<WastSublanguage>>> = LazyLock::new(NodeProp::new);

impl<'a> SyntaxNode<'a> {
    /// The row installed by direct WAST parsing, if present on this tree.
    pub fn wast_node_type(self) -> Option<&'a WastNodeType> {
        self.tree_metadata::<WastNodeTypes>()?.get(self.term())
    }
}

impl SyntaxTree {
    /// Read the highlight context from this tree's actual WAST root. Return
    /// `None` for other syntax trees. This root query does not validate a span
    /// or establish editor coverage. For viewport spans, use
    /// [`EditorState::wast_viewport_highlight_context`].
    pub fn wast_highlight_context(&self) -> Option<HighlightContext> {
        HighlightContext::from_root(NativeLanguage::Wast, self.root().wast_node_type()?)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WastNodeTypes(NodeTypes);

impl WastNodeTypes {
    pub(crate) fn stock() -> Self {
        static STOCK: LazyLock<WastNodeTypes> = LazyLock::new(|| {
            let group = Arc::new(vec![String::from("Expression")]);
            let app_indent = Arc::new(Arc::new(app_indent) as WastIndentStrategy);
            let auto_isolate = Arc::new(Some(WastIsolation::Auto));
            let app: WastFoldCallback = Arc::new(|node, _| node.fold_inside());
            let comment: WastFoldCallback = Arc::new(|node, _| Some(node.fold_block_comment()));
            WastNodeTypes(NodeTypes::new(
                tables::NODE_NAMES,
                tables::REPEAT_NODE_COUNT,
                tables::TOP_RULE[1],
                &[0, 1, 2],
                |term, name, props| {
                    match name {
                        "(" => props.insert(&CLOSED_BY, vec![String::from(")")]),
                        ")" => props.insert(&OPENED_BY, vec![String::from("(")]),
                        _ => (),
                    }
                    if term == tables::TOP_RULE[1] {
                        props.insert(&LANGUAGE_DATA, Some(WastDataFacet::stock().clone()));
                    }
                    if matches!(term, 6..=11) {
                        props.insert_shared(&GROUP, Arc::clone(&group));
                    }
                    if term == 6 {
                        props.insert_shared(&INDENT, Arc::clone(&app_indent));
                    }
                    if matches!(term, 1 | 2 | 11) {
                        props.insert_shared(&ISOLATE, Arc::clone(&auto_isolate));
                    }
                    let fold = match name {
                        "App" => Some(&app),
                        "BlockComment" => Some(&comment),
                        _ => None,
                    };
                    if let Some(fold) = fold {
                        props.insert(&FOLD, Some(fold.clone()));
                    }
                    if let Some(styles) = stock_rules(name) {
                        props.insert(&STYLE_RULES, styles);
                    }
                },
            ))
        });
        STOCK.clone()
    }

    pub(super) fn has_configured_styles(&self) -> bool {
        self.0.has_different_styles(&Self::stock().0)
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &WastNodeType> {
        self.0.iter()
    }

    pub(crate) fn get(&self, term: u16) -> Option<&WastNodeType> {
        self.0.get(term)
    }

    pub(crate) fn root(&self) -> &WastNodeType {
        self.get(tables::TOP_RULE[1]).expect("WAST has a root term")
    }

    pub(super) fn for_kind(&self, kind: WastNodeKind) -> &WastNodeType {
        use WastNodeKind::*;
        let term = match kind {
            Error => 0,
            LineComment => 1,
            BlockComment => 2,
            Module => 3,
            CloseParen => 4,
            OpenParen => 5,
            App => 6,
            Identifier => 7,
            Type => 8,
            Keyword => 9,
            Number => 10,
            String => 11,
        };
        self.get(term).expect("WAST kind has a grammar term")
    }

    pub(crate) fn extend(&self, sources: &[WastPropSource]) -> Self {
        Self(self.0.extend(sources))
    }
}
