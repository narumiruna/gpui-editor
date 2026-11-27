use std::borrow::Cow;

use crate::language::lezer::{SyntaxTree, TreeFragment};

/// The logical input and its mapping to the original UTF-8 byte offsets.
pub(in crate::language) struct Input<'a> {
    pub(in crate::language) text: Cow<'a, str>,
    breaks: Vec<(usize, usize)>,
    source: &'a str,
}

impl<'a> Input<'a> {
    pub(in crate::language) fn new(source: &'a str, normalize_lone_cr: bool) -> Self {
        let breaks: Vec<_> = source
            .match_indices("\r\n")
            .enumerate()
            .map(|(removed, (at, _))| (at, at - removed))
            .collect();
        let mut text = if breaks.is_empty() {
            Cow::Borrowed(source)
        } else {
            Cow::Owned(source.replace("\r\n", "\n"))
        };
        if normalize_lone_cr && text.contains('\r') {
            text = Cow::Owned(text.replace('\r', "\n"));
        }
        Self {
            text,
            breaks,
            source,
        }
    }

    pub(in crate::language) fn byte(&self, at: usize) -> usize {
        at + self.breaks.partition_point(|&(_, logical)| logical < at)
    }

    pub(in crate::language) fn fragment<'t>(
        &self,
        tree: &'t SyntaxTree,
        mut end: usize,
    ) -> TreeFragment<'t> {
        // An edit next to a CR can form or split a CRLF at the boundary.
        // Do not claim that CR as part of the unchanged logical prefix.
        if end > 0 && self.source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        let end = end - self.breaks.partition_point(|&(original, _)| original < end);
        TreeFragment {
            tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    }
}
