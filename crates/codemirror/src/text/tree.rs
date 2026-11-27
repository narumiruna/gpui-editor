//! Persistent AVL tree with bounded UTF-8 leaves and explicit line endings.

use std::{ops::Range, sync::Arc};

// Adjacent leaves are merged when their combined length fits this limit.
const CHUNK_BYTES: usize = 1024;

type Link = Option<Arc<Node>>;

#[derive(Clone, Default)]
pub(super) struct Tree(Link);

struct Node {
    len: usize,
    breaks: usize,
    height: u8,
    kind: Kind,
}

enum Kind {
    Leaf(Leaf),
    Branch(Arc<Node>, Arc<Node>),
}

struct Leaf {
    text: Box<str>,
    breaks: Box<[u16]>,
}

/// A leaf and its offsets in this tree, not in the tree that created it.
#[derive(Clone, Copy)]
pub(super) struct Chunk<'a> {
    pub(super) start: usize,
    breaks_before: usize,
    leaf: &'a Leaf,
}

impl<'a> Chunk<'a> {
    pub(super) fn end(self) -> usize {
        self.start + self.leaf.text.len()
    }

    pub(super) fn slice(self, range: Range<usize>) -> Option<&'a str> {
        self.leaf
            .text
            .get(range.start.checked_sub(self.start)?..range.end.checked_sub(self.start)?)
    }

    pub(super) fn byte(self, position: usize) -> u8 {
        self.leaf.text.as_bytes()[position - self.start]
    }

    pub(super) fn is_char_boundary(self, position: usize) -> bool {
        self.leaf.text.is_char_boundary(position - self.start)
    }

    pub(super) fn breaks_before(self, position: usize) -> usize {
        self.breaks_before
            + self
                .leaf
                .breaks
                .partition_point(|&index| usize::from(index) < position - self.start)
    }

    pub(super) fn break_at(self, index: usize) -> Option<usize> {
        let local = index.checked_sub(self.breaks_before)?;
        Some(self.start + usize::from(*self.leaf.breaks.get(local)?))
    }
}

impl Tree {
    pub(super) fn new(source: String) -> Self {
        if source.is_empty() {
            return Self::default();
        }
        if source.len() <= CHUNK_BYTES {
            return Self(Some(Node::leaf(source)));
        }
        Self::from_str(&source)
    }

    fn from_str(mut source: &str) -> Self {
        if source.len() <= CHUNK_BYTES {
            return Self::new(source.to_owned());
        }
        let mut leaves = Vec::with_capacity(source.len().div_ceil(CHUNK_BYTES));
        while !source.is_empty() {
            let (chunk, tail) = source.split_at(source.floor_char_boundary(CHUNK_BYTES));
            leaves.push(Node::leaf(chunk.to_owned()));
            source = tail;
        }
        Self(build(&leaves))
    }

    pub(super) fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |root| root.len)
    }

    pub(super) fn breaks(&self) -> usize {
        self.0.as_ref().map_or(0, |root| root.breaks)
    }

    pub(super) fn contiguous(&self) -> Option<&str> {
        match self.0.as_deref().map(|root| &root.kind) {
            None => Some(""),
            Some(Kind::Leaf(leaf)) => Some(&leaf.text),
            Some(Kind::Branch(..)) => None,
        }
    }

    pub(super) fn chunk_at(&self, position: usize) -> Option<Chunk<'_>> {
        let mut node = self.0.as_deref()?;
        if position >= node.len {
            return None;
        }
        let mut start = 0;
        let mut breaks_before = 0;
        loop {
            match &node.kind {
                Kind::Leaf(leaf) => {
                    return Some(Chunk {
                        start,
                        breaks_before,
                        leaf,
                    });
                }
                Kind::Branch(left, right) => {
                    node = if position - start < left.len {
                        left
                    } else {
                        start += left.len;
                        breaks_before += left.breaks;
                        right
                    };
                }
            }
        }
    }

    /// The byte offset of the zero-based LF index.
    pub(super) fn break_at(&self, mut index: usize) -> Option<usize> {
        let mut node = self.0.as_deref()?;
        if index >= node.breaks {
            return None;
        }
        let mut start = 0;
        loop {
            match &node.kind {
                Kind::Leaf(leaf) => return Some(start + usize::from(leaf.breaks[index])),
                Kind::Branch(left, right) => {
                    node = if index < left.breaks {
                        left
                    } else {
                        index -= left.breaks;
                        start += left.len;
                        right
                    };
                }
            }
        }
    }

    /// Iterate over the physical chunks intersecting a valid byte range.
    pub(super) fn chunks(&self, range: Range<usize>) -> impl Iterator<Item = &str> {
        let mut pending = Vec::new();
        if !range.is_empty()
            && let Some(root) = self.0.as_deref()
        {
            pending.push((root, 0));
        }
        std::iter::from_fn(move || {
            while let Some((node, start)) = pending.pop() {
                let end = start + node.len;
                if range.start >= end || range.end <= start {
                    continue;
                }
                match &node.kind {
                    Kind::Leaf(leaf) => {
                        return Some(
                            &leaf.text
                                [range.start.saturating_sub(start)..range.end.min(end) - start],
                        );
                    }
                    Kind::Branch(left, right) => {
                        pending.push((right, start + left.len));
                        pending.push((left, start));
                    }
                }
            }
            None
        })
    }

    pub(super) fn same_bytes(&self, other: &Self) -> bool {
        let (Some(left), Some(right)) = (self.0.as_deref(), other.0.as_deref()) else {
            return self.0.is_none() && other.0.is_none();
        };
        if std::ptr::eq(left, right) {
            return true;
        }
        if let (Kind::Leaf(a), Kind::Leaf(b)) = (&left.kind, &right.kind) {
            return a.text == b.text;
        }
        let mut a = vec![Part::whole(left)];
        let mut b = vec![Part::whole(right)];
        // Each stack starts at the same logical offset. Skip a shared node
        // only when both documents have reached that same part of it.
        while let (Some(&left), Some(&right)) = (a.last(), b.last()) {
            if std::ptr::eq(left.node, right.node) && left.offset == right.offset {
                a.pop();
                b.pop();
                continue;
            }
            match (&left.node.kind, &right.node.kind) {
                (Kind::Leaf(l), Kind::Leaf(r)) => {
                    let x = &l.text.as_bytes()[left.offset..];
                    let y = &r.text.as_bytes()[right.offset..];
                    let width = x.len().min(y.len());
                    if x[..width] != y[..width] {
                        return false;
                    }
                    Part::consume(&mut a, width);
                    Part::consume(&mut b, width);
                }
                (Kind::Branch(..), Kind::Leaf(_)) => Part::expand(&mut a),
                (Kind::Branch(..), Kind::Branch(..)) if left.node.len >= right.node.len => {
                    Part::expand(&mut a);
                }
                (_, Kind::Branch(..)) => Part::expand(&mut b),
            }
        }
        a.is_empty() && b.is_empty()
    }

    pub(super) fn subtree(&self, range: Range<usize>) -> Self {
        if range.is_empty() {
            return Self::default();
        }
        let len = range.len();
        let (_, tail) = self.clone().split(range.start);
        tail.split(len).0
    }

    pub(super) fn replace(&self, range: Range<usize>, insert: &str) -> Self {
        self.replace_tree(range, Self::from_str(insert))
    }

    pub(super) fn replace_tree(&self, range: Range<usize>, insert: Self) -> Self {
        let (prefix, tail) = self.clone().split(range.start);
        let (_, suffix) = tail.split(range.len());
        prefix.concat(insert).concat(suffix)
    }

    fn split(self, at: usize) -> (Self, Self) {
        if at == 0 {
            return (Self::default(), self);
        }
        if at == self.len() {
            return (self, Self::default());
        }
        let (left, right) = split(self.0.unwrap(), at);
        // A split can make only the new edge leaf small enough to merge with its neighbor.
        (Self(left).pack_edge(false), Self(right).pack_edge(true))
    }

    fn pack_edge(self, front: bool) -> Self {
        let Some((a, b)) = self.edge_pair(front) else {
            return self;
        };
        let Some(node) = merge_leaves(a.leaf, b.leaf) else {
            return self;
        };
        let start = a.start;
        let (before, rest) = split(self.0.unwrap(), start);
        let (_, after) = split(rest.unwrap(), node.len);
        Self(join(join(before, Some(node)), after))
    }

    fn edge_pair(&self, front: bool) -> Option<(Chunk<'_>, Chunk<'_>)> {
        if front {
            let a = self.chunk_at(0)?;
            Some((a, self.chunk_at(a.end())?))
        } else {
            let b = self.chunk_at(self.len().checked_sub(1)?)?;
            Some((self.chunk_at(b.start.checked_sub(1)?)?, b))
        }
    }

    fn concat(self, other: Self) -> Self {
        let Some(left) = self.0 else { return other };
        let Some(right) = other.0 else {
            return Self(Some(left));
        };
        let last = edge_leaf(&left, false);
        let first = edge_leaf(&right, true);
        let Some(middle) = merge_leaves(last, first) else {
            return Self(Some(join_nodes(left, right)));
        };
        let left_end = left.len - last.text.len();
        let right_start = first.text.len();
        let (prefix, _) = split(left, left_end);
        let (_, suffix) = split(right, right_start);
        Self(join(join(prefix, Some(middle)), suffix))
    }
}

#[derive(Clone, Copy)]
struct Part<'a> {
    node: &'a Node,
    offset: usize,
}

impl<'a> Part<'a> {
    fn whole(node: &'a Node) -> Self {
        Self { node, offset: 0 }
    }

    fn consume(pending: &mut Vec<Self>, width: usize) {
        let part = pending.last_mut().expect("current leaf");
        part.offset += width;
        if part.offset == part.node.len {
            pending.pop();
        }
    }

    fn expand(pending: &mut Vec<Self>) {
        let part = pending.pop().expect("current branch");
        debug_assert_eq!(part.offset, 0);
        let (left, right) = part.node.children();
        pending.extend([Self::whole(right), Self::whole(left)]);
    }
}

impl Node {
    fn leaf(text: String) -> Arc<Self> {
        debug_assert!(!text.is_empty() && text.len() <= CHUNK_BYTES);
        let breaks: Box<[_]> = text
            .bytes()
            .enumerate()
            .filter_map(|(i, byte)| (byte == b'\n').then_some(i as u16))
            .collect();
        Arc::new(Self {
            len: text.len(),
            breaks: breaks.len(),
            height: 1,
            kind: Kind::Leaf(Leaf {
                text: text.into_boxed_str(),
                breaks,
            }),
        })
    }

    fn branch(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        debug_assert!(left.height.abs_diff(right.height) <= 1);
        Arc::new(Self {
            len: left.len + right.len,
            breaks: left.breaks + right.breaks,
            height: left.height.max(right.height) + 1,
            kind: Kind::Branch(left, right),
        })
    }

    fn children(&self) -> (&Arc<Self>, &Arc<Self>) {
        let Kind::Branch(left, right) = &self.kind else {
            unreachable!("leaf has no children")
        };
        (left, right)
    }
}

fn build(leaves: &[Arc<Node>]) -> Link {
    match leaves {
        [] => None,
        [leaf] => Some(leaf.clone()),
        _ => {
            let (left, right) = leaves.split_at(leaves.len() / 2);
            Some(Node::branch(build(left).unwrap(), build(right).unwrap()))
        }
    }
}

fn split(node: Arc<Node>, at: usize) -> (Link, Link) {
    if at == 0 {
        return (None, Some(node));
    }
    if at == node.len {
        return (Some(node), None);
    }
    match &node.kind {
        Kind::Leaf(leaf) => {
            let (left, right) = leaf.text.split_at(at);
            (
                Some(Node::leaf(left.to_owned())),
                Some(Node::leaf(right.to_owned())),
            )
        }
        Kind::Branch(left, right) => match at.cmp(&left.len) {
            std::cmp::Ordering::Less => {
                let (prefix, tail) = split(left.clone(), at);
                (prefix, join(tail, Some(right.clone())))
            }
            std::cmp::Ordering::Equal => (Some(left.clone()), Some(right.clone())),
            std::cmp::Ordering::Greater => {
                let (head, suffix) = split(right.clone(), at - left.len);
                (join(Some(left.clone()), head), suffix)
            }
        },
    }
}

fn edge_leaf(mut node: &Node, front: bool) -> &Leaf {
    loop {
        match &node.kind {
            Kind::Leaf(leaf) => return leaf,
            Kind::Branch(left, right) => node = if front { left } else { right },
        }
    }
}

fn merge_leaves(left: &Leaf, right: &Leaf) -> Link {
    (left.text.len() + right.text.len() <= CHUNK_BYTES)
        .then(|| Node::leaf([&*left.text, &*right.text].concat()))
}

fn join(left: Link, right: Link) -> Link {
    match (left, right) {
        (Some(left), Some(right)) => Some(join_nodes(left, right)),
        (None, rest) | (rest, None) => rest,
    }
}

#[cfg(test)]
impl Tree {
    pub(super) fn check_structure(&self) {
        fn check(node: &Node, previous: &mut Option<usize>) {
            match &node.kind {
                Kind::Leaf(leaf) => {
                    assert!((1..=CHUNK_BYTES).contains(&node.len));
                    if let Some(len) = previous.replace(node.len) {
                        assert!(len + node.len > CHUNK_BYTES);
                    }
                    assert_eq!(node.len, leaf.text.len());
                    assert_eq!(node.height, 1);
                    assert_eq!(node.breaks, leaf.breaks.len());
                    assert!(
                        leaf.breaks.iter().map(|&i| usize::from(i)).eq(leaf
                            .text
                            .bytes()
                            .enumerate()
                            .filter_map(|(i, b)| (b == b'\n').then_some(i)))
                    );
                }
                Kind::Branch(left, right) => {
                    assert!(left.height.abs_diff(right.height) <= 1);
                    assert_eq!(node.height, left.height.max(right.height) + 1);
                    assert_eq!(node.len, left.len + right.len);
                    assert_eq!(node.breaks, left.breaks + right.breaks);
                    check(left, previous);
                    check(right, previous);
                }
            }
        }
        if let Some(root) = self.0.as_deref() {
            check(root, &mut None);
        }
    }

    pub(super) fn shared_branches(&self, other: &Self) -> usize {
        use std::collections::HashSet;

        fn branches(root: Option<&Node>) -> impl Iterator<Item = *const Node> {
            let mut pending = root.into_iter().collect::<Vec<_>>();
            std::iter::from_fn(move || {
                while let Some(node) = pending.pop() {
                    if let Kind::Branch(left, right) = &node.kind {
                        pending.extend([left.as_ref(), right.as_ref()]);
                        return Some(std::ptr::from_ref(node));
                    }
                }
                None
            })
        }

        let original: HashSet<_> = branches(self.0.as_deref()).collect();
        branches(other.0.as_deref())
            .filter(|branch| original.contains(branch))
            .count()
    }
}

/// Join at equal heights, then rebuild only the path to the join.
fn join_nodes(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height > right.height + 1 {
        let (a, b) = left.children();
        balance(a.clone(), join_nodes(b.clone(), right))
    } else if right.height > left.height + 1 {
        let (a, b) = right.children();
        balance(join_nodes(left, a.clone()), b.clone())
    } else {
        Node::branch(left, right)
    }
}

/// The join can increase one child's height by one. Rotate if needed.
fn balance(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height > right.height + 1 {
        let (a, b) = left.children();
        if a.height >= b.height {
            Node::branch(a.clone(), Node::branch(b.clone(), right))
        } else {
            let (c, d) = b.children();
            Node::branch(
                Node::branch(a.clone(), c.clone()),
                Node::branch(d.clone(), right),
            )
        }
    } else if right.height > left.height + 1 {
        let (a, b) = right.children();
        if b.height >= a.height {
            Node::branch(Node::branch(left, a.clone()), b.clone())
        } else {
            let (c, d) = a.children();
            Node::branch(
                Node::branch(left, c.clone()),
                Node::branch(d.clone(), b.clone()),
            )
        }
    } else {
        Node::branch(left, right)
    }
}
