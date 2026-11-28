use std::{any::Any, fmt, sync::Arc};

use super::{ParseError, StackView, TokenInput};

type Update<C> = dyn Fn(&C, u16, &StackView<'_>, &mut TokenInput<'_>) -> Result<Option<C>, ParseError>
    + Send
    + Sync;

/// An immutable context value and its updates for an `@context` declaration.
///
/// Each parse starts with `start`. GLR branches share values until a hook
/// returns a replacement. Keep values immutable, including their shared data.
/// Equal values must have the same effect on hooks, tokenizers, and specializers.
/// For a large context, use shared data so updates and comparisons stay cheap.
/// The parser does not meter arbitrary work in trusted hooks or equality tests.
pub struct ContextTracker<C> {
    start: C,
    shift: Option<Box<Update<C>>>,
    reduce: Option<Box<Update<C>>>,
    reduce_reads_start: bool,
}

impl<C: Eq + Send + Sync + 'static> ContextTracker<C> {
    pub fn new(start: C) -> Self {
        Self {
            start,
            shift: None,
            reduce: None,
            reduce_reads_start: false,
        }
    }

    /// Set the hook for a shifted term, including skipped and zero-width tokens.
    /// The cursor starts at the token's start in UTF-16 units. The stack is the
    /// result of the shift, with the old context still in place. Return
    /// `Ok(None)` to keep that value. Entering a skip rule by goto is not a shift.
    pub fn with_shift(
        mut self,
        update: impl Fn(&C, u16, &StackView<'_>, &mut TokenInput<'_>) -> Result<Option<C>, ParseError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.shift = Some(Box::new(update));
        self
    }

    /// Set the hook for a reduced term, including empty and anonymous rules.
    /// The cursor starts at the reduction's start in UTF-16 units. The stack
    /// is the result of the reduction, with the old context still in place.
    /// Return `Ok(None)` to keep that value.
    pub fn with_reduce(
        mut self,
        update: impl Fn(&C, u16, &StackView<'_>, &mut TokenInput<'_>) -> Result<Option<C>, ParseError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.reduce = Some(Box::new(update));
        self.reduce_reads_start = true;
        self
    }

    /// Set a reduce hook that uses only the context and the term. Recovery can
    /// merge equivalent stacks with different start offsets for this hook.
    pub(crate) fn with_reduce_term(
        mut self,
        update: impl Fn(&C, u16) -> Option<C> + Send + Sync + 'static,
    ) -> Self {
        self.reduce = Some(Box::new(move |context, term, _, _| {
            Ok(update(context, term))
        }));
        self.reduce_reads_start = false;
        self
    }

    pub(super) fn erase(self, deterministic: bool) -> ErasedTracker {
        ErasedTracker {
            deterministic,
            start: Context::new(self.start),
            shift: self.shift.map(erase_update),
            reduce: self.reduce.map(erase_update),
            reduce_reads_start: self.reduce_reads_start,
        }
    }
}

pub(super) type ContextUpdate = dyn Fn(u16, &StackView<'_>, &mut TokenInput<'_>) -> Result<Option<Context>, ParseError>
    + Send
    + Sync;

fn erase_update<C: Eq + Send + Sync + 'static>(update: Box<Update<C>>) -> Box<ContextUpdate> {
    Box::new(move |term, stack, input| {
        let context = stack.context::<C>().expect("context from this tracker");
        Ok(update(context, term, stack, input)?.map(Context::new))
    })
}

pub(super) struct ErasedTracker {
    pub(super) deterministic: bool,
    pub(super) start: Context,
    pub(super) shift: Option<Box<ContextUpdate>>,
    pub(super) reduce: Option<Box<ContextUpdate>>,
    pub(super) reduce_reads_start: bool,
}

impl fmt::Debug for ErasedTracker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextTracker")
            .field("deterministic", &self.deterministic)
            .finish_non_exhaustive()
    }
}

trait Value: Any + Send + Sync {
    fn equals(&self, other: &dyn Value) -> bool;
}

impl<C: Eq + Send + Sync + 'static> Value for C {
    fn equals(&self, other: &dyn Value) -> bool {
        (other as &dyn Any).downcast_ref::<C>() == Some(self)
    }
}

#[derive(Clone)]
pub(super) struct Context(Arc<dyn Value>);

impl Context {
    fn new<C: Eq + Send + Sync + 'static>(value: C) -> Self {
        Self(Arc::new(value))
    }

    pub(super) fn get<C: 'static>(&self) -> Option<&C> {
        (self.0.as_ref() as &dyn Any).downcast_ref()
    }
}

impl PartialEq for Context {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.equals(other.0.as_ref())
    }
}

impl Eq for Context {}
