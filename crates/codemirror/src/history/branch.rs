use std::collections::VecDeque;

use crate::{Assoc, ChangeDesc, ChangeSet, EditError, EditorSelection, StateEffect};

const NATIVE_LIMIT: usize = 100;
const PRUNE_MARGIN: usize = 20;
const MAX_PREVIOUS_SELECTIONS: usize = 200;

#[derive(Clone, Debug)]
pub(super) struct Branch<M> {
    pub entries: VecDeque<Entry<M>>,
    /// Selection changes before the oldest edit, or in a branch with no edits.
    pub base_selections: Option<Vec<EditorSelection>>,
}

impl<M> Default for Branch<M> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            base_selections: None,
        }
    }
}

impl<M> Branch<M> {
    pub fn selections(&self) -> &[EditorSelection] {
        self.entries.back().map_or_else(
            || self.base_selections.as_deref().unwrap_or_default(),
            |entry| &entry.selections_after,
        )
    }

    fn selections_mut(&mut self) -> &mut Vec<EditorSelection> {
        if let Some(entry) = self.entries.back_mut() {
            &mut entry.selections_after
        } else {
            self.base_selections.get_or_insert_with(Vec::new)
        }
    }

    pub fn add_selection(&mut self, selection: EditorSelection) {
        let selections = self.selections_mut();
        if selections.last() != Some(&selection) {
            selections.drain(..selections.len().saturating_sub(MAX_PREVIOUS_SELECTIONS));
            selections.push(selection);
        }
    }

    pub fn pop_selection(&mut self) {
        self.selections_mut().pop();
    }

    pub fn store(&mut self, entry: Entry<M>, replace: bool, min_depth: Option<usize>) {
        if replace {
            self.entries.pop_back();
        }
        let len = self.entries.len();
        let mut remove = match min_depth {
            Some(depth) => {
                // Upstream counts the selection-only base, even when its last
                // selection has been replayed. For `to = len - 1`, its slice
                // starts at `to - depth - 1` when `len > depth + 20`.
                let len = len + usize::from(self.base_selections.is_some()) + 1;
                if len > depth.saturating_add(PRUNE_MARGIN) {
                    len - depth - 2
                } else {
                    0
                }
            }
            None if !replace && len >= NATIVE_LIMIT => {
                len - NATIVE_LIMIT + 1 + usize::from(self.base_selections.is_some())
            }
            None => 0,
        };
        if remove != 0 && self.base_selections.take().is_some() {
            remove -= 1;
        }
        self.entries.drain(..remove);
        self.entries.push_back(entry);
    }
}

#[derive(Clone, Debug)]
pub(super) struct Entry<M> {
    /// Replay this edit to go from the current document to the other side of the event.
    pub changes: ChangeSet,
    pub effects: Vec<StateEffect>,
    /// Saved selection in the document after replay.
    pub selection: EditorSelection,
    /// Native selection saved in the document before replay. If absent, infer
    /// it from `selection` as CodeMirror does.
    pub opposite_selection: Option<EditorSelection>,
    pub before: M,
    pub after: M,
    /// Selection changes after this event, in the document before replay.
    pub selections_after: Vec<EditorSelection>,
    /// Deferred mapping for the older events, in the document after replay.
    pub mapped: Option<ChangeDesc>,
}

impl<M: Clone> Entry<M> {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.effects.is_empty()
    }

    pub fn reverse_selection(&self, length: usize) -> Result<EditorSelection, EditError> {
        if let Some(selection) = self
            .selections_after
            .first()
            .or(self.opposite_selection.as_ref())
        {
            return Ok(selection.clone());
        }
        let mapping = self.changes.desc(length)?.inverted();
        self.selection.map_desc_with_assoc(&mapping, Assoc::After)
    }

    pub fn touches(&self, incoming: &Self, length: usize) -> bool {
        let Ok(incoming) = incoming.changes.desc(length) else {
            return false;
        };
        let Ok(previous) = self.changes.desc(incoming.new_length()) else {
            return false;
        };
        incoming.changed_ranges().any(|(_, incoming)| {
            previous.changed_ranges().any(|(previous, _)| {
                incoming.end >= previous.start && incoming.start <= previous.end
            })
        })
    }

    pub fn join(&self, mut next: Self, length: usize) -> Result<Self, EditError> {
        let current = next.changes.desc(length)?.new_length();
        let previous = self.changes.desc(current)?;
        next.changes = next.changes.compose(&self.changes, length)?;
        next.effects = map_effects(&next.effects, &previous);
        next.effects.extend(self.effects.iter().cloned());
        next.selection = self.selection.clone();
        next.before = self.before.clone();
        next.mapped = self.mapped.clone();
        Ok(next)
    }

    pub fn map(&self, mapping: &ChangeDesc) -> Result<Self, EditError> {
        let changes = self.changes.map(mapping, false)?;
        let after_replay = mapping.map(&self.changes.desc(mapping.length())?, true)?;
        let selection = self.selection.map_desc(&after_replay)?;
        let mapped = match &self.mapped {
            Some(previous) => previous.compose(&after_replay)?,
            None => after_replay,
        };
        Ok(Self {
            changes,
            effects: map_effects(&self.effects, mapping),
            selection,
            opposite_selection: self
                .opposite_selection
                .as_ref()
                .map(|s| s.map_desc(mapping))
                .transpose()?,
            before: self.before.clone(),
            after: self.after.clone(),
            selections_after: map_selections(&self.selections_after, mapping)?,
            mapped: Some(mapped),
        })
    }
}

fn map_effects(effects: &[StateEffect], mapping: &ChangeDesc) -> Vec<StateEffect> {
    effects
        .iter()
        .filter_map(|effect| effect.map(mapping))
        .collect()
}

pub(super) fn map_selections(
    selections: &[EditorSelection],
    mapping: &ChangeDesc,
) -> Result<Vec<EditorSelection>, EditError> {
    selections.iter().map(|s| s.map_desc(mapping)).collect()
}

/// A prepared branch update. Only map the first surviving event, and defer
/// the rest until replay passes that event.
pub(super) struct BranchMapping<M> {
    removed: usize,
    top: MappedTop<M>,
}

enum MappedTop<M> {
    Event(Entry<M>),
    Base(Vec<EditorSelection>),
}

impl<M: Clone> BranchMapping<M> {
    pub fn new(
        branch: &Branch<M>,
        skip: usize,
        mut mapping: ChangeDesc,
    ) -> Result<Self, EditError> {
        let mut carried = Vec::new();
        for (removed, entry) in branch.entries.iter().rev().skip(skip).enumerate() {
            let mut entry = entry.map(&mapping)?;
            entry.selections_after.append(&mut carried);
            if !entry.is_empty() {
                return Ok(Self {
                    removed,
                    top: MappedTop::Event(entry),
                });
            }
            // These selections are already in the final document since the
            // mapped event has no effect. Move them to the next surviving event.
            mapping = entry
                .mapped
                .take()
                .expect("mapped edit carries a deferred mapping");
            carried = entry.selections_after;
        }
        let mut base = map_selections(
            branch.base_selections.as_deref().unwrap_or_default(),
            &mapping,
        )?;
        base.append(&mut carried);
        Ok(Self {
            removed: branch.entries.len() - skip,
            top: MappedTop::Base(base),
        })
    }

    pub fn apply(self, branch: &mut Branch<M>) -> bool {
        branch.entries.truncate(branch.entries.len() - self.removed);
        match self.top {
            MappedTop::Event(top) => *branch.entries.back_mut().expect("mapped top event") = top,
            MappedTop::Base(base) => branch.base_selections = (!base.is_empty()).then_some(base),
        }
        self.removed > 0
    }
}
