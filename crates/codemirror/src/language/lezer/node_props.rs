use std::{
    any::Any,
    collections::HashMap,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

type Value = Arc<dyn Any + Send + Sync>;
type Combine<T> = dyn Fn(&T, &T) -> T + Send + Sync;
type ApplySource = dyn Fn(&NodeType, &mut Option<Properties>) + Send + Sync;

/// A typed key for LR node-type properties. Keys with the same value type
/// have distinct identities. Clones keep the identity of their key.
pub struct NodeProp<T> {
    id: usize,
    combine: Option<Arc<Combine<T>>>,
}

impl<T: Send + Sync + 'static> NodeProp<T> {
    /// Define a key for which the last contribution wins.
    pub fn new() -> Self {
        Self::define(None)
    }

    /// Combine an existing value and the next value in that order. Store the
    /// first value without combining.
    pub fn with_combine(combine: impl Fn(&T, &T) -> T + Send + Sync + 'static) -> Self {
        Self::define(Some(Arc::new(combine)))
    }

    fn define(combine: Option<Arc<Combine<T>>>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = next_id(&NEXT, "node property keys exhausted");
        Self { id, combine }
    }

    /// Create a source that reads each row as it was before the batch.
    /// `None` makes no contribution. When the value type is optional,
    /// `Some(None)` stores a value. `Some(false)` and `Some(0)` also store values.
    pub fn add(
        &self,
        value: impl Fn(&NodeType) -> Option<T> + Send + Sync + 'static,
    ) -> NodePropSource {
        let key = self.clone();
        NodePropSource(Box::new(move |row, pending| {
            let Some(mut value) = value(row) else { return };
            let props = pending.get_or_insert_with(|| row.props.clone());
            if let Some(combine) = &key.combine
                && let Some(old) = props.get(&key)
            {
                value = combine(old, &value);
            }
            props.insert(&key, value);
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

/// One source in a node-type property batch.
pub struct NodePropSource(Box<ApplySource>);

/// The names of the groups to which a node type belongs.
pub static GROUP: LazyLock<NodeProp<Vec<String>>> = LazyLock::new(NodeProp::new);

/// Names that can close this node type. An empty list marks a delimiter
/// without a named partner. An absent list allows plain text matching.
pub static CLOSED_BY: LazyLock<NodeProp<Vec<String>>> = LazyLock::new(NodeProp::new);

/// Names that can open this node type. See [`CLOSED_BY`].
pub static OPENED_BY: LazyLock<NodeProp<Vec<String>>> = LazyLock::new(NodeProp::new);

/// The base direction of a syntax isolate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Isolation {
    /// Use the first strong character in the text.
    Auto,
    Ltr,
    Rtl,
}

/// The syntax isolate direction. An explicit `None` disables the isolate.
pub static ISOLATE: LazyLock<NodeProp<Option<Isolation>>> = LazyLock::new(NodeProp::new);

#[derive(Clone, Debug, Default)]
pub(crate) struct Properties(HashMap<usize, Value>);

impl Properties {
    fn get<T: 'static>(&self, key: &NodeProp<T>) -> Option<&T> {
        self.0.get(&key.id)?.downcast_ref()
    }

    pub(crate) fn insert<T: Send + Sync + 'static>(&mut self, key: &NodeProp<T>, value: T) {
        self.insert_shared(key, Arc::new(value));
    }

    pub(crate) fn insert_shared<T: Send + Sync + 'static>(
        &mut self,
        key: &NodeProp<T>,
        value: Arc<T>,
    ) {
        self.0.insert(key.id, value);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeTypeId(usize);

/// An immutable LR node-type row shared by all occurrences of its grammar term.
#[derive(Debug)]
pub struct NodeType {
    id: NodeTypeId,
    term: u16,
    name: &'static str,
    top: bool,
    skipped: bool,
    props: Properties,
}

impl NodeType {
    fn new(term: u16, name: &'static str, top: bool, skipped: bool, props: Properties) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self {
            id: NodeTypeId(next_id(&NEXT, "node type IDs exhausted")),
            term,
            name,
            top,
            skipped,
            props,
        }
    }

    pub(crate) fn id(&self) -> NodeTypeId {
        self.id
    }

    /// The grammar-local term ID, not a node occurrence index.
    pub fn term(&self) -> u16 {
        self.term
    }

    /// The grammar name, or an empty string for an anonymous repeat row.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether this row describes the root of the grammar.
    pub fn is_top(&self) -> bool {
        self.top
    }

    /// Whether this row describes the reserved Lezer error term.
    pub fn is_error(&self) -> bool {
        self.term == 0
    }

    /// Whether the grammar marks this row as skipped.
    pub fn is_skipped(&self) -> bool {
        self.skipped
    }

    /// Whether this row has no grammar name.
    pub fn is_anonymous(&self) -> bool {
        self.name.is_empty()
    }

    /// Match the grammar name or one of the configured group names.
    pub fn is(&self, name_or_group: &str) -> bool {
        self.name == name_or_group
            || self
                .prop(&GROUP)
                .is_some_and(|groups| groups.iter().any(|name| name == name_or_group))
    }

    /// Read a value by key identity. `None` means the key is absent.
    pub fn prop<T: 'static>(&self, key: &NodeProp<T>) -> Option<&T> {
        self.props.get(key)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NodeTypes(Arc<[Arc<NodeType>]>);

impl NodeTypes {
    pub(crate) fn new(
        names: &'static str,
        repeat_count: u16,
        top: u16,
        skipped: &[u16],
        seed: impl Fn(u16, &'static str, &mut Properties),
    ) -> Self {
        Self::with_tops(names, repeat_count, &[top], skipped, seed)
    }

    pub(crate) fn with_tops(
        names: &'static str,
        repeat_count: u16,
        tops: &[u16],
        skipped: &[u16],
        seed: impl Fn(u16, &'static str, &mut Properties),
    ) -> Self {
        Self(
            names
                .split(' ')
                .chain(std::iter::repeat_n("", usize::from(repeat_count)))
                .enumerate()
                .map(|(index, name)| {
                    let term = index.try_into().expect("grammar term ID fits in u16");
                    let mut props = Properties::default();
                    seed(term, name, &mut props);
                    Arc::new(NodeType::new(
                        term,
                        name,
                        tops.contains(&term),
                        skipped.contains(&term),
                        props,
                    ))
                })
                .collect(),
        )
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &NodeType> {
        self.0.iter().map(Arc::as_ref)
    }

    pub(crate) fn get(&self, term: u16) -> Option<&NodeType> {
        self.0.get(usize::from(term)).map(Arc::as_ref)
    }

    pub(crate) fn same_set(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn extend(&self, sources: &[NodePropSource]) -> Self {
        Self(
            self.0
                .iter()
                .map(|row| {
                    let mut pending = None;
                    for source in sources {
                        (source.0)(row, &mut pending);
                    }
                    match pending {
                        Some(props) => Arc::new(NodeType::new(
                            row.term,
                            row.name,
                            row.top,
                            row.skipped,
                            props,
                        )),
                        None => Arc::clone(row),
                    }
                })
                .collect(),
        )
    }
}

fn next_id(sequence: &AtomicUsize, exhausted: &str) -> usize {
    sequence
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect(exhausted)
}
