//! Configuration hooks for changing a transaction before it is applied.

use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{Annotation, EditError, EditorState, Extension, Facet, StateEffect, Text, Transaction};

/// Which old-document ranges a transaction is allowed to change.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ChangePermission {
    #[default]
    Allow,
    Reject,
    Protect(Vec<Range<usize>>),
}

/// Metadata that an extender can add without changing the document or selection.
#[derive(Debug, Default)]
pub struct TransactionExtension {
    pub effects: Vec<StateEffect>,
    pub annotations: Vec<Annotation>,
    pub scroll_into_view: bool,
}

type ChangeFilter = Arc<dyn Fn(&EditorState, &Transaction) -> ChangePermission + Send + Sync>;
type Filter =
    Arc<dyn Fn(&EditorState, Transaction) -> Result<Transaction, EditError> + Send + Sync>;
type Extender = Arc<dyn Fn(&EditorState, &Transaction) -> TransactionExtension + Send + Sync>;
type List<T> = Facet<T, Vec<T>>;

static CHANGE_FILTERS: LazyLock<List<ChangeFilter>> = LazyLock::new(Facet::collect);
static FILTERS: LazyLock<List<Filter>> = LazyLock::new(Facet::collect);
static EXTENDERS: LazyLock<List<Extender>> = LazyLock::new(Facet::collect);

/// Register a change filter. All protected ranges are combined before filtering.
pub fn change_filter(
    filter: impl Fn(&EditorState, &Transaction) -> ChangePermission + Send + Sync + 'static,
) -> Extension {
    CHANGE_FILTERS.of(Arc::new(filter))
}

/// Register a hook that can replace the proposed transaction. Later hooks run first.
pub fn transaction_filter(
    filter: impl Fn(&EditorState, Transaction) -> Result<Transaction, EditError> + Send + Sync + 'static,
) -> Extension {
    FILTERS.of(Arc::new(filter))
}

/// Register a hook that adds metadata after filtering, even when filtering is disabled.
///
/// Each extender reads the same transaction, without additions from other extenders.
pub fn transaction_extender(
    extend: impl Fn(&EditorState, &Transaction) -> TransactionExtension + Send + Sync + 'static,
) -> Extension {
    EXTENDERS.of(Arc::new(extend))
}

impl EditorState {
    pub(crate) fn filter_transaction(
        &self,
        mut transaction: Transaction,
    ) -> Result<Transaction, EditError> {
        if !self.extensions.active() {
            return Ok(transaction);
        }

        let context = self.context();
        let source = self.text();
        validate_candidate(&transaction, source)?;
        if !transaction.unfiltered {
            let mut protected = Vec::new();
            let mut rejected = false;
            for filter in context.facet(&CHANGE_FILTERS)?.iter() {
                match filter(self, &transaction) {
                    ChangePermission::Allow => {}
                    ChangePermission::Reject => {
                        rejected = true;
                        break;
                    }
                    ChangePermission::Protect(ranges) => protected.extend(ranges),
                }
            }
            if rejected || !protected.is_empty() {
                transaction = filter_changes(transaction, source, rejected, protected)?;
            }
            for filter in context.facet(&FILTERS)?.iter().rev() {
                transaction = filter(self, transaction)?;
                validate_candidate(&transaction, source)?;
            }
        }

        let mut additions = TransactionExtension::default();
        for extend in context.facet(&EXTENDERS)?.iter().rev() {
            let extra = extend(self, &transaction);
            additions.effects.extend(extra.effects);
            additions.annotations.extend(extra.annotations);
            additions.scroll_into_view |= extra.scroll_into_view;
        }
        transaction.effects.extend(additions.effects);
        transaction.annotations.extend(additions.annotations);
        transaction.scroll_into_view |= additions.scroll_into_view;
        Ok(transaction)
    }
}

fn validate_candidate(transaction: &Transaction, source: &Text) -> Result<(), EditError> {
    let changes = transaction.changes();
    changes.check(source)?;
    if let Some(selection) = transaction.selection() {
        selection.check(|pos| changes.is_boundary_after(source, pos))?;
    }
    Ok(())
}

fn filter_changes(
    mut transaction: Transaction,
    source: &Text,
    rejected: bool,
    protected: Vec<Range<usize>>,
) -> Result<Transaction, EditError> {
    let back = if rejected {
        let all = std::mem::take(&mut transaction.changes);
        all.desc(source.len())?.inverted()
    } else {
        let protected = normalize_ranges(source, protected)?;
        let all = std::mem::take(&mut transaction.changes);
        let (allowed, denied) = all.split_protected(source.len(), &protected)?;
        let back = denied.map(&allowed.desc(source.len())?, false)?.inverted();
        transaction.changes = allowed;
        back
    };
    transaction.selection = transaction
        .selection
        .map(|selection| selection.map_desc(&back))
        .transpose()?;
    transaction.effects = StateEffect::map_all(&transaction.effects, &back);
    Ok(transaction)
}

fn normalize_ranges(
    source: &Text,
    mut ranges: Vec<Range<usize>>,
) -> Result<Vec<Range<usize>>, EditError> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if !source.is_valid_range(range.clone()) {
            return Err(EditError::InvalidRange);
        }
        if range.is_empty() {
            continue;
        }
        if let Some(previous) = merged.last_mut()
            && previous.end >= range.start
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    Ok(merged)
}
