use crate::ChangeSet;

use super::{SyntaxTree, TreeFragment};

/// Unchanged edges of an edit in the original source. Use this with parsers
/// that collapse CRLF to LF before calling the LR runtime.
pub(crate) struct NormalizedEdges<'a> {
    tree: &'a SyntaxTree,
    prefix_end: usize,
    suffix_len: usize,
}

impl<'a> NormalizedEdges<'a> {
    /// `changes` must be valid for the original document.
    pub(crate) fn new(tree: &'a SyntaxTree, old_len: usize, changes: &ChangeSet) -> Self {
        let mut edited = changes.changes().map(|(range, _)| range);
        let first = edited.next();
        let last = edited.last().or(first);
        Self {
            tree,
            prefix_end: first.map_or(old_len, |range| range.start),
            suffix_len: last.map_or(0, |range| old_len - range.end),
        }
    }

    pub(crate) fn fragments(
        self,
        source: &str,
        normalized_len: usize,
        normalized_at: impl Fn(usize) -> usize,
    ) -> Vec<TreeFragment<'a>> {
        // An edit can join or split CRLF. Leave a touching CR out of the
        // prefix and a touching LF out of the suffix. The parser also checks
        // the original and new normalized bytes and the old scan extent.
        let end = self.prefix_end - usize::from(source[..self.prefix_end].ends_with('\r'));
        let end = normalized_at(end);
        let mut fragments = vec![TreeFragment {
            tree: self.tree,
            old_range: 0..end,
            new_range: 0..end,
        }];
        let start = source.len() - self.suffix_len;
        let start = normalized_at(start + usize::from(source[start..].starts_with('\n')));
        let old_end = self.tree.root().range().end;
        if start < normalized_len
            && let Some(old_start) = old_end.checked_sub(normalized_len - start)
            && start != old_start
        {
            fragments.push(TreeFragment {
                tree: self.tree,
                old_range: old_start..old_end,
                new_range: start..normalized_len,
            });
        }
        fragments
    }
}
