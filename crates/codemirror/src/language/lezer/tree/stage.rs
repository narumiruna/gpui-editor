use super::{
    Arena, DeferredChildren, Node, ParseError, Parser, Roots, SyntaxTree, TreeBuilder, Work,
};

impl TreeBuilder {
    // Existing records stay read-only. Roll back temporary records on both
    // success and limit errors, without moving or copying the live records.
    pub(in crate::language::lezer) fn with_staged_output(
        &mut self,
        parser: &Parser,
        end: usize,
        work: &Work,
        stage: impl FnOnce(&mut Self) -> Result<Roots, ParseError>,
    ) -> Result<SyntaxTree, ParseError> {
        let live_deferred = std::mem::take(&mut self.deferred);
        self.nodes.isolate_tail();
        self.links.isolate_tail();
        let result = stage(self)
            .and_then(|roots| self.finish_stage(parser, roots, end, work, &live_deferred));
        self.nodes.restore_tail();
        self.links.restore_tail();
        self.deferred = live_deferred;
        result
    }

    fn finish_stage(
        &mut self,
        parser: &Parser,
        roots: Roots,
        end: usize,
        work: &Work,
        live_deferred: &[DeferredChildren],
    ) -> Result<SyntaxTree, ParseError> {
        work.spend(end, roots.len)?;
        self.finish_root(parser, roots, end);
        enum Visit {
            Enter(usize),
            Leave(usize, usize),
        }
        let mut pending = vec![Visit::Enter(self.nodes.len() - 1)];
        let mut output = Arena::default();
        let mut children = Vec::new();
        // Visit only the selected roots and their descendants. Flatten repeat
        // groups and omit lookahead barriers without copying them into output.
        while let Some(visit) = pending.pop() {
            match visit {
                Visit::Enter(id) => {
                    let node = &self.nodes[id];
                    work.spend(end, 1 + node.children.len())?;
                    if usize::from(node.term) < parser.names.len() {
                        pending.push(Visit::Leave(id, children.len()));
                    }
                    let deferred = if live_deferred.last().is_some_and(|span| id <= span.node) {
                        live_deferred
                    } else {
                        &self.deferred
                    };
                    if node.children.is_empty()
                        && (Some(node.term) == self.defer_repeat
                            || (node.term != 0 && usize::from(node.term) < self.defer_named_below))
                        && let Some(span) = DeferredChildren::find(deferred, id, end, work)?
                    {
                        pending.extend(
                            span.reverse_children(&self.links, end, work)?
                                .map(Visit::Enter),
                        );
                    } else {
                        pending.extend(node.children.iter().rev().copied().map(Visit::Enter));
                    }
                }
                Visit::Leave(id, base) => {
                    let node = &self.nodes[id];
                    work.spend(end, 1 + children.len() - base)?;
                    let direct = children.split_off(base);
                    children.push(output.len());
                    output.push(Node {
                        term: node.term,
                        range: node.range.clone(),
                        children: direct,
                    });
                }
            }
        }
        Ok(SyntaxTree {
            names: parser.names.clone(),
            nodes: output,
            incremental: None,
            metadata: None,
        })
    }
}

impl SyntaxTree {
    // Staged and complete job output has no shared checkpoint blocks.
    // Return the number of mapped endpoints for the output work report.
    pub(in crate::language) fn project_glr_ranges(
        &mut self,
        original_byte: impl Fn(usize) -> usize,
    ) -> usize {
        assert!(self.incremental.is_none());
        let nodes = self.nodes.unshared();
        for node in nodes.iter_mut() {
            node.range = original_byte(node.range.start)..original_byte(node.range.end);
        }
        2 * nodes.len()
    }
}
