use std::{any::Any, ops::Range, sync::Arc};

use super::{
    Parser,
    parse::{ParseError, Work, incremental::CheckpointHistory},
};

mod arena;
mod stage;

use arena::Arena;

// Reserved internal record. Like repeat groups, it is removed by GLR compaction.
const LOOKAHEAD_BARRIER: u16 = u16::MAX;
const MIN_DEFERRED_REPEAT_CHILDREN: usize = 32;

#[derive(Debug)]
struct Node {
    term: u16,
    range: Range<usize>,
    children: Vec<usize>,
}

// Stacks share immutable nodes and root lists. A fork need not copy any of
// its output, and reductions cannot change the roots of another branch.
#[derive(Clone, Copy, Default)]
pub(super) struct Roots {
    tail: Option<usize>,
    pub(super) len: usize,
}

type Link = (usize, Option<usize>);

struct DeferredChildren {
    node: usize,
    tail: usize,
    count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct OutputPrefix {
    nodes: usize,
    links: usize,
}

#[derive(Default)]
pub(super) struct TreeBuilder {
    nodes: Arena<Node>,
    links: Arena<Link>,
    records_repeats: bool,
    defer_named_below: usize,
    defer_repeat: Option<u16>,
    deferred: Vec<DeferredChildren>,
}

impl TreeBuilder {
    pub(super) fn for_glr(recover: bool, defer_named_below: usize) -> Self {
        Self {
            records_repeats: recover,
            defer_named_below,
            ..Self::default()
        }
    }

    pub(super) fn for_wast_recovery(repeat: u16) -> Self {
        Self {
            defer_repeat: Some(repeat),
            ..Self::for_glr(true, 0)
        }
    }

    pub(super) fn records_repeats(&self) -> bool {
        self.records_repeats
    }

    pub(super) fn checkpoint(&mut self) -> OutputPrefix {
        OutputPrefix {
            nodes: self.nodes.freeze(),
            links: self.links.freeze(),
        }
    }

    pub(super) fn resume(tree: &SyntaxTree, prefix: OutputPrefix) -> Self {
        let output = tree.incremental.as_ref().expect("tree has checkpoints");
        Self {
            nodes: output
                .prefix_nodes
                .as_deref()
                .unwrap_or(&tree.nodes)
                .prefix(prefix.nodes),
            links: output.links.prefix(prefix.links),
            ..Self::default()
        }
    }

    pub(super) fn into_prefix_seed(self, history: CheckpointHistory) -> IncrementalOutput {
        IncrementalOutput {
            history,
            prefix_nodes: Some(Box::new(self.nodes.without_tail())),
            links: self.links.without_tail(),
        }
    }

    pub(super) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn append(&mut self, roots: &mut Roots, node: usize) {
        let link = self.links.len();
        self.links.push((node, roots.tail));
        roots.tail = Some(link);
        roots.len += 1;
    }

    fn pop(&self, roots: &mut Roots) -> Option<usize> {
        let (node, prev) = self.links[roots.tail?];
        roots.tail = prev;
        roots.len -= 1;
        Some(node)
    }

    pub(super) fn leaf(&mut self, roots: &mut Roots, term: u16, range: Range<usize>) {
        self.append(roots, self.nodes.len());
        self.nodes.push(Node {
            term,
            range,
            children: Vec::new(),
        });
    }

    pub(super) fn lookahead_barrier(&mut self, roots: &mut Roots, pos: usize) {
        if roots
            .tail
            .is_none_or(|link| self.nodes[self.links[link].0].term != LOOKAHEAD_BARRIER)
        {
            self.leaf(roots, LOOKAHEAD_BARRIER, pos..pos);
        }
    }

    pub(super) fn error(
        &mut self,
        roots: &mut Roots,
        frame_base: usize,
        mut range: Range<usize>,
        deleted: Option<u16>,
    ) {
        if let Some(term) = deleted {
            let base = roots.len;
            self.leaf(roots, term, range.clone());
            self.wrap(roots, base, 0, range);
            return;
        }
        if roots.len > frame_base {
            let (last, _) = self.links[roots.tail.expect("nonempty roots")];
            let node = &self.nodes[last];
            if node.term == 0 {
                if range.is_empty() {
                    return;
                }
                if node.range.end == range.start {
                    range.start = node.range.start;
                    // An error can have one child: the named token it deleted.
                    let children = node.children.clone();
                    self.pop(roots);
                    self.append(roots, self.nodes.len());
                    self.nodes.push(Node {
                        term: 0,
                        range,
                        children,
                    });
                    return;
                }
            }
        }
        self.leaf(roots, 0, range);
    }

    pub(super) fn reduce(
        &mut self,
        roots: &mut Roots,
        base: Roots,
        term: u16,
        range: Range<usize>,
        pos: usize,
        work: &Work,
    ) -> Result<(), ParseError> {
        let count = roots.len - base.len;
        // WAST recovery trials can wrap the same wide list of deleted tokens
        // at successive positions. Do not copy it into each discarded trial.
        let defer = (term != 0 && usize::from(term) < self.defer_named_below && count > 1)
            || (Some(term) == self.defer_repeat && count >= MIN_DEFERRED_REPEAT_CHILDREN);
        if defer {
            let tail = roots.tail.expect("nonempty roots");
            if self.nodes[self.links[tail].0].range.end <= range.end {
                // The tail has no skipped record to move. The frame's saved
                // prefix is immutable. Keep the suffix without copying it.
                // Materialize it only if this node is in the selected tree.
                work.spend(pos, 1)?;
                self.deferred.push(DeferredChildren {
                    node: self.nodes.len(),
                    tail,
                    count,
                });
                *roots = base;
                self.leaf(roots, term, range);
                return Ok(());
            }
        }
        work.spend(pos, count)?;
        self.wrap(roots, base.len, term, range);
        Ok(())
    }

    fn wrap(&mut self, roots: &mut Roots, base: usize, term: u16, range: Range<usize>) {
        let mut trailing = Vec::new();
        let mut children = Vec::new();
        while roots.len > base {
            let node = self.pop(roots).expect("root above base");
            // Leave trailing skipped nodes outside the new node.
            if children.is_empty() && self.nodes[node].range.end > range.end {
                trailing.push(node);
            } else {
                children.push(node);
            }
        }
        // A lookahead record alone does not make a reduction sink. If there
        // are skipped nodes to move, keep the barrier in their record order.
        if trailing
            .iter()
            .all(|&node| self.nodes[node].term == LOOKAHEAD_BARRIER)
        {
            trailing.clear();
        }
        children.reverse();
        self.append(roots, self.nodes.len());
        self.nodes.push(Node {
            term,
            range,
            children,
        });
        for node in trailing.into_iter().rev() {
            self.append(roots, node);
        }
    }

    pub(super) fn finish(
        mut self,
        parser: &Parser,
        roots: Roots,
        end: usize,
        history: Option<CheckpointHistory>,
    ) -> SyntaxTree {
        self.finish_root(parser, roots, end);
        debug_assert!(self.deferred.is_empty());
        self.into_tree(parser, history)
    }

    pub(super) fn finish_glr(
        mut self,
        parser: &Parser,
        roots: Roots,
        end: usize,
        work: &Work,
    ) -> Result<SyntaxTree, ParseError> {
        self.finish_root(parser, roots, end);
        self.compact_glr_output(parser.names.len(), end, work)?;
        Ok(self.into_tree(parser, None))
    }

    fn finish_root(&mut self, parser: &Parser, mut roots: Roots, end: usize) {
        let mut children = Vec::with_capacity(roots.len);
        while let Some(node) = self.pop(&mut roots) {
            children.push(node);
        }
        children.reverse();
        self.nodes.push(Node {
            term: parser.top_rule[1],
            range: 0..end,
            children,
        });
    }

    fn into_tree(self, parser: &Parser, history: Option<CheckpointHistory>) -> SyntaxTree {
        SyntaxTree {
            names: Arc::clone(&parser.names),
            nodes: self.nodes,
            metadata: None,
            incremental: history.map(|history| IncrementalOutput {
                history,
                prefix_nodes: None,
                links: self.links.without_tail(),
            }),
        }
    }
}

#[derive(Debug)]
pub(super) struct IncrementalOutput {
    history: CheckpointHistory,
    // A recovered tree publishes different nodes. Its checkpoints refer only
    // to these frozen strict LR records, never to the recovery arena.
    prefix_nodes: Option<Box<Arena<Node>>>,
    links: Arena<Link>,
}

/// An owned syntax tree. It does not borrow the parser or the input text.
#[derive(Debug)]
pub struct SyntaxTree {
    names: Arc<[Box<str>]>,
    nodes: Arena<Node>,
    incremental: Option<IncrementalOutput>,
    metadata: Option<Arc<dyn Any + Send + Sync>>,
}

impl TreeBuilder {
    fn compact_glr_output(
        &mut self,
        named: usize,
        end: usize,
        work: &Work,
    ) -> Result<(), ParseError> {
        let mut deferred = self.deferred.iter().rev().peekable();
        let links = &self.links;
        let nodes = self.nodes.unshared();
        work.spend(end, nodes.len())?;
        let mut ids = vec![usize::MAX; nodes.len()];
        *ids.last_mut().expect("root node") = 0;
        let mut pending = Vec::new();
        // Every child was added before its parent. Remove lookahead barriers
        // and flatten repeat groups once, without recursing on deep lists.
        for id in (0..nodes.len()).rev() {
            let span = deferred.next_if(|span| span.node == id);
            if ids[id] == usize::MAX {
                continue;
            }
            if let Some(span) = span {
                materialize(links, span, &mut nodes[id].children, end, work)?;
            }
            let children = &nodes[id].children;
            work.spend(end, children.len())?;
            if children
                .iter()
                .any(|&child| usize::from(nodes[child].term) >= named)
            {
                pending.extend(children.iter().rev().copied());
                let mut children = std::mem::take(&mut nodes[id].children);
                children.clear();
                while let Some(child) = pending.pop() {
                    let node = &nodes[child];
                    if usize::from(node.term) >= named {
                        if Some(node.term) == self.defer_repeat
                            && node.children.is_empty()
                            && let Some(span) =
                                DeferredChildren::find(&self.deferred, child, end, work)?
                        {
                            pending.extend(span.reverse_children(links, end, work)?);
                        } else {
                            work.spend(end, node.children.len())?;
                            pending.extend(node.children.iter().rev().copied());
                        }
                    } else {
                        children.push(child);
                        ids[child] = 0;
                    }
                }
                nodes[id].children = children;
            } else {
                for &child in children {
                    ids[child] = 0;
                }
            }
        }
        let (mut old_id, mut new_id) = (0, 0);
        nodes.retain_mut(|node| {
            let id = old_id;
            old_id += 1;
            if ids[id] == usize::MAX {
                return false;
            }
            for child in &mut node.children {
                *child = ids[*child];
            }
            ids[id] = new_id;
            new_id += 1;
            true
        });
        nodes.shrink_to_fit();
        Ok(())
    }
}

impl DeferredChildren {
    fn find<'a>(
        spans: &'a [Self],
        node: usize,
        pos: usize,
        work: &Work,
    ) -> Result<Option<&'a Self>, ParseError> {
        let Some((first, last)) = spans.first().zip(spans.last()) else {
            return Ok(None);
        };
        if !(first.node..=last.node).contains(&node) {
            return Ok(None);
        }
        // The spans are in node order. Charge the maximum comparisons in
        // the binary search before reading the list.
        work.spend(pos, spans.len().ilog2() as usize + 2)?;
        Ok(spans
            .binary_search_by_key(&node, |span| span.node)
            .ok()
            .map(|index| &spans[index]))
    }

    fn reverse_children<'a>(
        &self,
        links: &'a Arena<Link>,
        pos: usize,
        work: &Work,
    ) -> Result<impl ExactSizeIterator<Item = usize> + 'a, ParseError> {
        work.spend(pos, self.count)?;
        let mut tail = Some(self.tail);
        Ok((0..self.count).map(move |_| {
            let (child, prev) = links[tail.expect("child link in deferred suffix")];
            tail = prev;
            child
        }))
    }
}

fn materialize(
    links: &Arena<Link>,
    span: &DeferredChildren,
    children: &mut Vec<usize>,
    pos: usize,
    work: &Work,
) -> Result<(), ParseError> {
    let children_in_reverse = span.reverse_children(links, pos, work)?;
    children.reserve_exact(children_in_reverse.len());
    children.extend(children_in_reverse);
    children.reverse();
    Ok(())
}

impl SyntaxTree {
    pub(in crate::language) fn with_metadata(mut self, metadata: impl Any + Send + Sync) -> Self {
        self.metadata = Some(Arc::new(metadata));
        self
    }

    pub(super) fn with_prefix_seed(mut self, seed: Option<IncrementalOutput>) -> Self {
        debug_assert!(self.incremental.is_none());
        self.incremental = seed;
        self
    }

    pub(crate) fn has_incremental_state(&self) -> bool {
        self.incremental.is_some()
    }

    pub(super) fn checkpoint_history(&self) -> Option<&CheckpointHistory> {
        self.incremental.as_ref().map(|output| &output.history)
    }

    pub(super) fn belongs_to(&self, parser: &Parser) -> bool {
        Arc::ptr_eq(&self.names, &parser.names)
    }

    /// The root covers the entire input, including skipped text.
    pub fn root(&self) -> SyntaxNode<'_> {
        SyntaxNode {
            tree: self,
            id: self.nodes.len() - 1,
        }
    }
}

/// A node in a [`SyntaxTree`].
#[derive(Clone, Copy, Debug)]
pub struct SyntaxNode<'a> {
    tree: &'a SyntaxTree,
    id: usize,
}

impl<'a> SyntaxNode<'a> {
    /// The grammar-local node ID.
    pub fn term(self) -> u16 {
        self.tree.nodes[self.id].term
    }

    pub fn name(self) -> &'a str {
        &self.tree.names[usize::from(self.term())]
    }

    pub(in crate::language) fn tree_metadata<T: Any>(self) -> Option<&'a T> {
        self.tree.metadata.as_ref()?.downcast_ref()
    }

    /// Whether this node has the reserved Lezer error term (0).
    pub fn is_error(self) -> bool {
        self.term() == 0
    }

    /// A UTF-8 byte range in the original input.
    pub fn range(self) -> Range<usize> {
        self.tree.nodes[self.id].range.clone()
    }

    /// The direct children in source order, including named skipped nodes.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.tree.nodes[self.id]
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }
}
