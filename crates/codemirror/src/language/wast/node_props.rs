use std::{
    any::Any,
    collections::HashMap,
    ops::Range,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::{
    WastDataFacet, WastNode, WastNodeKind, WastSublanguage,
    indent::{WastIndentStrategy, app_indent},
    style_tags::{StyleRules, stock_rules},
    tables,
};
use crate::{
    EditorState,
    language::{
        HighlightContext,
        lezer::{SyntaxNode, SyntaxTree},
    },
};

type Value = Arc<dyn Any + Send + Sync>;
type Props = HashMap<usize, Value>;
type Combine<T> = dyn Fn(&T, &T) -> T + Send + Sync;
type ApplySource = dyn Fn(&WastNodeType, &mut Option<Props>) + Send + Sync;

/// A typed key for WAST node-type properties. Each definition has its own
/// identity, even for keys with the same value type. Clones keep that identity.
pub struct NodeProp<T> {
    id: usize,
    combine: Option<Arc<Combine<T>>>,
}

impl<T: Send + Sync + 'static> NodeProp<T> {
    /// Define a key for which the last contribution wins.
    pub fn new() -> Self {
        Self::define(None)
    }

    /// Define a key that combines an existing value and the next contribution,
    /// in that order. The first contribution is stored without combining.
    pub fn with_combine(combine: impl Fn(&T, &T) -> T + Send + Sync + 'static) -> Self {
        Self::define(Some(Arc::new(combine)))
    }

    fn define(combine: Option<Arc<Combine<T>>>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("node property keys exhausted");
        Self { id, combine }
    }

    /// Create a source that reads the row as it was before the current batch.
    /// `None` makes no contribution. For an optional value type, `Some(None)`
    /// stores a value. `Some(false)` and `Some(0)` also store values.
    pub fn add(
        &self,
        value: impl Fn(&WastNodeType) -> Option<T> + Send + Sync + 'static,
    ) -> WastPropSource {
        let key = self.clone();
        WastPropSource(Box::new(move |row, pending| {
            let Some(mut value) = value(row) else { return };
            let props = pending.get_or_insert_with(|| row.props.clone());
            if let Some(combine) = &key.combine
                && let Some(old) = get(props, &key)
            {
                value = combine(old, &value);
            }
            props.insert(key.id, Arc::new(value));
        }))
    }
}

impl<T: Send + Sync + 'static> Default for NodeProp<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for NodeProp<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            combine: self.combine.clone(),
        }
    }
}

fn get<'a, T: 'static>(props: &'a Props, key: &NodeProp<T>) -> Option<&'a T> {
    props.get(&key.id)?.downcast_ref()
}

/// One property source in a [`WastLanguage::with_node_props`] batch.
///
/// [`WastLanguage::with_node_props`]: crate::language_data::WastLanguage::with_node_props
pub struct WastPropSource(Box<ApplySource>);

/// Group names used by [`WastNodeType::is`]. Stock expression types have the
/// group `Expression`. A contribution replaces the entire list.
pub static GROUP: LazyLock<NodeProp<Vec<String>>> = LazyLock::new(NodeProp::new);

/// Names that can close this node type. Stock `(` names `)`. An empty list
/// marks a delimiter with no named partner; an absent list allows text options
/// to mark a one-character node name.
pub static CLOSED_BY: LazyLock<NodeProp<Vec<String>>> = LazyLock::new(NodeProp::new);

/// Names that can open this node type. Stock `)` names `(`. An empty list
/// marks a delimiter with no named partner, as for [`CLOSED_BY`].
pub static OPENED_BY: LazyLock<NodeProp<Vec<String>>> = LazyLock::new(NodeProp::new);

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

/// The base direction of a syntax isolate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WastIsolation {
    /// Use the first strong character in the text.
    Auto,
    Ltr,
    Rtl,
}

/// Assign an isolate direction to a syntax type. An explicit `None` suppresses
/// the stock isolate. By default, strings and comments use automatic direction.
pub static ISOLATE: LazyLock<NodeProp<Option<WastIsolation>>> = LazyLock::new(NodeProp::new);

/// Propose a fold in original UTF-8 byte offsets for a configured node in the
/// current editor state. Node ranges refer to the same document as the state.
/// Return `None` to skip this node.
pub type WastFoldCallback =
    Arc<dyn Fn(WastNode<'_>, &EditorState) -> Option<Range<usize>> + Send + Sync>;

/// State-aware fold strategy. App and BlockComment have stock callbacks.
/// A source that returns `Some(None)` disables this row's rule, not its
/// ancestors' rules.
pub static FOLD: LazyLock<NodeProp<Option<WastFoldCallback>>> = LazyLock::new(NodeProp::new);

/// The facet for a WAST top node. `Module` has the stock facet by default.
/// An explicit `None` supplies no data and skips all sublanguage predicates.
pub static LANGUAGE_DATA: LazyLock<NodeProp<Option<WastDataFacet>>> = LazyLock::new(NodeProp::new);

/// Ordered sublanguage rules for the WAST top node. Only the first matching
/// rule applies. The stock `Module` has no rules.
pub static SUBLANGUAGES: LazyLock<NodeProp<Vec<WastSublanguage>>> = LazyLock::new(NodeProp::new);

pub(super) static STYLE_RULES: LazyLock<NodeProp<StyleRules>> =
    LazyLock::new(|| NodeProp::with_combine(StyleRules::combine));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WastNodeTypeId(usize);

/// An immutable WAST node-type row, shared by all occurrences of its term.
#[derive(Debug)]
pub struct WastNodeType {
    id: WastNodeTypeId,
    term: u16,
    name: &'static str,
    props: Props,
}

impl WastNodeType {
    fn new(term: u16, name: &'static str, props: Props) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("WAST node type IDs exhausted");
        Self {
            id: WastNodeTypeId(id),
            term,
            name,
            props,
        }
    }

    pub(crate) fn id(&self) -> WastNodeTypeId {
        self.id
    }

    /// The grammar term ID, not a node occurrence index.
    pub fn term(&self) -> u16 {
        self.term
    }

    /// The grammar name, or an empty string for an anonymous repeat row.
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn is_top(&self) -> bool {
        self.term == tables::TOP_RULE[1]
    }

    pub fn is_error(&self) -> bool {
        self.term == 0
    }

    pub fn is_skipped(&self) -> bool {
        // These are the skipped term IDs in the published WAST grammar.
        matches!(self.term, 0..=2)
    }

    pub fn is_anonymous(&self) -> bool {
        self.name.is_empty()
    }

    /// Match the grammar name or one of the row's configured group names.
    pub fn is(&self, name_or_group: &str) -> bool {
        self.name == name_or_group
            || self
                .prop(&GROUP)
                .is_some_and(|groups| groups.iter().any(|name| name == name_or_group))
    }

    /// Read a value by key identity. `None` means the key is absent.
    pub fn prop<T: 'static>(&self, key: &NodeProp<T>) -> Option<&T> {
        get(&self.props, key)
    }

    pub(crate) fn data_facet(&self) -> Option<&WastDataFacet> {
        self.prop(&LANGUAGE_DATA).and_then(Option::as_ref)
    }
}

impl<'a> SyntaxNode<'a> {
    /// The row installed by direct WAST parsing, if present on this tree.
    pub fn wast_node_type(self) -> Option<&'a WastNodeType> {
        self.tree_metadata::<WastNodeTypes>()?.get(self.term())
    }

    /// Read a WAST node-type property by key identity.
    pub fn prop<T: 'static>(self, key: &NodeProp<T>) -> Option<&'a T> {
        self.wast_node_type()?.prop(key)
    }
}

impl SyntaxTree {
    /// Read the highlight context from this tree's actual WAST root. Return
    /// `None` for other syntax trees. This root query does not validate a span
    /// or establish editor coverage. For viewport spans, use
    /// [`EditorState::wast_viewport_highlight_context`].
    pub fn wast_highlight_context(&self) -> Option<HighlightContext> {
        HighlightContext::from_wast_root(self.root().wast_node_type()?)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WastNodeTypes(Arc<[Arc<WastNodeType>]>);

impl WastNodeTypes {
    pub(crate) fn stock() -> Self {
        static STOCK: LazyLock<WastNodeTypes> = LazyLock::new(|| {
            let group: Value = Arc::new(vec![String::from("Expression")]);
            let closed_by: Value = Arc::new(vec![String::from(")")]);
            let opened_by: Value = Arc::new(vec![String::from("(")]);
            let app_indent: Value = Arc::new(Arc::new(app_indent) as WastIndentStrategy);
            let auto_isolate: Value = Arc::new(Some(WastIsolation::Auto));
            let app: WastFoldCallback = Arc::new(|node, _| node.fold_inside());
            let comment: WastFoldCallback = Arc::new(|node, _| Some(node.fold_block_comment()));
            WastNodeTypes(
                tables::NODE_NAMES
                    .split(' ')
                    .chain(std::iter::repeat_n(
                        "",
                        usize::from(tables::REPEAT_NODE_COUNT),
                    ))
                    .enumerate()
                    .map(|(term, name)| {
                        let mut props = Props::new();
                        match name {
                            "(" => props.insert(CLOSED_BY.id, Arc::clone(&closed_by)),
                            ")" => props.insert(OPENED_BY.id, Arc::clone(&opened_by)),
                            _ => None,
                        };
                        if term == usize::from(tables::TOP_RULE[1]) {
                            props.insert(
                                LANGUAGE_DATA.id,
                                Arc::new(Some(WastDataFacet::stock().clone())),
                            );
                        }
                        if matches!(term, 6..=11) {
                            props.insert(GROUP.id, Arc::clone(&group));
                        }
                        if term == 6 {
                            props.insert(INDENT.id, Arc::clone(&app_indent));
                        }
                        if matches!(term, 1 | 2 | 11) {
                            props.insert(ISOLATE.id, Arc::clone(&auto_isolate));
                        }
                        let fold = match name {
                            "App" => Some(&app),
                            "BlockComment" => Some(&comment),
                            _ => None,
                        };
                        if let Some(fold) = fold {
                            props.insert(FOLD.id, Arc::new(Some(fold.clone())));
                        }
                        if let Some(styles) = stock_rules(name) {
                            props.insert(STYLE_RULES.id, Arc::new(styles));
                        }
                        Arc::new(WastNodeType::new(
                            term.try_into().expect("WAST term ID"),
                            name,
                            props,
                        ))
                    })
                    .collect(),
            )
        });
        STOCK.clone()
    }

    pub(super) fn has_configured_styles(&self) -> bool {
        let stock = Self::stock();
        !Arc::ptr_eq(&self.0, &stock.0)
            && self.iter().zip(stock.iter()).any(|(row, stock)| {
                row.prop(&STYLE_RULES).map(std::ptr::from_ref)
                    != stock.prop(&STYLE_RULES).map(std::ptr::from_ref)
            })
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &WastNodeType> {
        self.0.iter().map(Arc::as_ref)
    }

    pub(crate) fn get(&self, term: u16) -> Option<&WastNodeType> {
        self.0.get(usize::from(term)).map(Arc::as_ref)
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
        Self(
            self.0
                .iter()
                .map(|row| {
                    let mut pending = None;
                    for source in sources {
                        (source.0)(row, &mut pending);
                    }
                    match pending {
                        Some(props) => Arc::new(WastNodeType::new(row.term, row.name, props)),
                        None => Arc::clone(row),
                    }
                })
                .collect(),
        )
    }
}
