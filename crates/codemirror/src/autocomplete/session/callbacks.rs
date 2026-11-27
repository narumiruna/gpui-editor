use std::{
    cmp::Ordering,
    sync::{Arc, LazyLock},
};

use crate::{ConfigurationError, EditorState, Extension, Facet};

use super::CompletionItem;

pub(super) type ActivateOnCompletion = dyn Fn(&CompletionItem) -> bool + Send + Sync;
pub(super) type CompareCompletions =
    dyn Fn(&CompletionItem, &CompletionItem) -> Ordering + Send + Sync;

type Value<T> = Arc<T>;
type Inputs<T> = Facet<Value<T>, Vec<Value<T>>>;
type Resolved<T> = Option<Value<T>>;

pub(super) struct Callback<T: ?Sized> {
    inputs: Inputs<T>,
    resolved: Facet<Value<T>, Resolved<T>>,
    resolve: Extension,
}

impl<T: ?Sized + Send + Sync + 'static> Callback<T> {
    fn new() -> Self {
        let inputs = Inputs::<T>::collect();
        let resolved =
            Facet::new(|values: &[&Value<T>]| values.first().map(|&value| value.clone()));
        let input_facet = inputs.reader();
        let resolve = resolved.compute_many_with_dependencies(
            |deps| deps.facet(&inputs),
            move |state| {
                let inputs = state.facet(&input_facet)?;
                let Some(first) = inputs.first() else {
                    return Ok(Vec::new());
                };
                if inputs.iter().any(|value| !Arc::ptr_eq(first, value)) {
                    return Err(ConfigurationError::InvalidFieldData);
                }
                Ok(vec![first.clone()])
            },
        );
        Self {
            inputs,
            resolved,
            resolve,
        }
    }

    fn of(&self, value: Value<T>) -> Extension {
        Extension::group([self.inputs.of(value), self.resolve.clone()])
    }

    pub(super) fn read(&self, state: &EditorState) -> Result<Arc<Resolved<T>>, ConfigurationError> {
        state.context().facet(&self.resolved)
    }
}

pub(super) static ACTIVATION: LazyLock<Callback<ActivateOnCompletion>> =
    LazyLock::new(Callback::new);
pub(super) static COMPARISON: LazyLock<Callback<CompareCompletions>> = LazyLock::new(Callback::new);

/// Configure reactivation for sessions reading this state's extensions. Reuse
/// the same extension when installing it more than once. Distinct callbacks
/// conflict. An explicit session builder policy takes precedence.
pub fn activate_on_completion(
    activate: impl Fn(&CompletionItem) -> bool + Send + Sync + 'static,
) -> Extension {
    ACTIVATION.of(Arc::new(activate))
}

/// Order completions with equal scores before adjacent deduplication. Distinct
/// comparators conflict. An explicit session builder comparator takes precedence.
pub fn compare_completion_ties(
    compare: impl Fn(&CompletionItem, &CompletionItem) -> Ordering + Send + Sync + 'static,
) -> Extension {
    COMPARISON.of(Arc::new(compare))
}
