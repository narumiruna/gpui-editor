use std::ops::Range;

use super::{Change, ChangeDesc, ChangeSet, EditError};

impl ChangeSet {
    /// Split changes using sorted, disjoint protected ranges in the old document.
    /// Insertions at the edge of a protected range remain allowed.
    pub(crate) fn split_protected(
        self,
        length: usize,
        protected: &[Range<usize>],
    ) -> Result<(Self, ChangeDesc), EditError> {
        self.check_length(length)?;
        let mut allowed = Vec::new();
        let mut denied = Vec::new();
        let mut blockers = protected.iter().peekable();
        for Change { range, mut insert } in self.changes {
            while blockers.peek().is_some_and(|r| r.end <= range.start) {
                blockers.next();
            }
            if range.is_empty() {
                if blockers.peek().is_some_and(|r| r.start < range.start) {
                    denied.push((range, insert.len()));
                } else {
                    allowed.push(Change::new(range, insert));
                }
                continue;
            }
            let mut start = range.start;
            while start < range.end {
                while blockers.peek().is_some_and(|r| r.end <= start) {
                    blockers.next();
                }
                let (skip, next) = match blockers.peek() {
                    Some(r) if r.start <= start => (true, r.end.min(range.end)),
                    Some(r) => (false, r.start.min(range.end)),
                    None => (false, range.end),
                };
                if skip {
                    denied.push((start..next, std::mem::take(&mut insert).len()));
                } else {
                    allowed.push(Change::new(start..next, std::mem::take(&mut insert)));
                }
                start = next;
            }
        }
        let mut allowed = Self::new(allowed)?;
        if self.scope.is_some() {
            allowed = allowed.bind_length(length);
        }
        Ok((allowed, ChangeDesc::new(length, denied)?))
    }
}
