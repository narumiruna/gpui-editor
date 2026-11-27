use std::{
    any::Any,
    fmt,
    marker::PhantomData,
    sync::{Arc, LazyLock},
};

use crate::{ChangeDesc, Transaction};

static USER_EVENT: LazyLock<AnnotationType<String>> = LazyLock::new(AnnotationType::new);
static TIME: LazyLock<AnnotationType<u64>> = LazyLock::new(AnnotationType::new);
static ADD_TO_HISTORY: LazyLock<AnnotationType<bool>> = LazyLock::new(AnnotationType::new);
static REMOTE: LazyLock<AnnotationType<bool>> = LazyLock::new(AnnotationType::new);

impl Transaction {
    /// The shared annotation key for event names and their dot-separated subtypes.
    pub fn user_event_annotation() -> &'static AnnotationType<String> {
        &USER_EVENT
    }

    pub fn with_user_event(self, event: impl Into<String>) -> Self {
        self.with_annotation(USER_EVENT.of(event.into()))
    }

    pub fn user_event(&self) -> Option<&str> {
        self.annotation(&USER_EVENT).map(String::as_str)
    }

    /// Match a user event or one of its dot-separated subtypes.
    pub fn is_user_event(&self, event: &str) -> bool {
        self.user_event().is_some_and(|actual| {
            actual == event
                || actual
                    .strip_prefix(event)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    }

    /// The shared annotation key for a timestamp in milliseconds.
    pub fn time_annotation() -> &'static AnnotationType<u64> {
        &TIME
    }

    /// Record input time for timed undo groups. Consecutive values use the same
    /// clock and represent milliseconds. Without a time, edits do not join by time.
    pub fn with_time(self, milliseconds: u64) -> Self {
        self.with_annotation(TIME.of(milliseconds))
    }

    pub fn time(&self) -> Option<u64> {
        self.annotation(&TIME).copied()
    }

    /// The shared annotation key for inclusion in undo history.
    pub fn add_to_history_annotation() -> &'static AnnotationType<bool> {
        &ADD_TO_HISTORY
    }

    /// Set whether the transaction is recorded. When false, apply its changes
    /// and map the existing undo and redo entries without adding an entry.
    pub fn with_add_to_history(self, add: bool) -> Self {
        self.with_annotation(ADD_TO_HISTORY.of(add))
    }

    pub fn add_to_history(&self) -> bool {
        self.annotation(&ADD_TO_HISTORY).copied().unwrap_or(true)
    }

    /// The shared annotation key for edits received from another source.
    pub fn remote_annotation() -> &'static AnnotationType<bool> {
        &REMOTE
    }

    /// Mark the origin. This annotation does not bypass filters or change history.
    pub fn with_remote(self, remote: bool) -> Self {
        self.with_annotation(REMOTE.of(remote))
    }

    pub fn is_remote(&self) -> bool {
        self.annotation(&REMOTE) == Some(&true)
    }
}

type Value = Arc<dyn Any + Send + Sync>;

/// A unique key for a typed value in transaction metadata.
pub struct AnnotationType<T> {
    id: Arc<()>,
    marker: PhantomData<fn() -> T>,
}

impl<T> AnnotationType<T> {
    pub fn new() -> Self {
        Self {
            id: Arc::new(()),
            marker: PhantomData,
        }
    }
}

impl<T: Send + Sync + 'static> AnnotationType<T> {
    pub fn of(&self, value: T) -> Annotation {
        Annotation {
            id: self.id.clone(),
            value: Arc::new(value),
        }
    }
}

impl<T> Default for AnnotationType<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for AnnotationType<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            marker: PhantomData,
        }
    }
}

/// A typed value associated with a transaction as a whole.
#[derive(Clone)]
pub struct Annotation {
    id: Arc<()>,
    value: Value,
}

impl Annotation {
    pub fn get<T: Send + Sync + 'static>(&self, kind: &AnnotationType<T>) -> Option<&T> {
        if Arc::ptr_eq(&self.id, &kind.id) {
            self.value.downcast_ref()
        } else {
            None
        }
    }
}

impl fmt::Debug for Annotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Annotation").finish_non_exhaustive()
    }
}

type Mapper = Arc<dyn Fn(&Value, &ChangeDesc) -> Option<Value> + Send + Sync>;

/// A unique key for an effect with an optional rule for mapping its positions.
pub struct StateEffectType<T> {
    key: AnnotationType<T>,
    mapper: Option<Mapper>,
}

impl<T: Send + Sync + 'static> StateEffectType<T> {
    /// Define an effect that does not change when it is mapped.
    pub fn new() -> Self {
        Self {
            key: AnnotationType::new(),
            mapper: None,
        }
    }

    /// Define a positional effect. Return `None` to discard it.
    pub fn with_mapping(
        map: impl Fn(&T, &ChangeDesc) -> Option<T> + Send + Sync + 'static,
    ) -> Self {
        Self {
            key: AnnotationType::new(),
            mapper: Some(Arc::new(move |value, changes| {
                Some(Arc::new(map(value.downcast_ref::<T>()?, changes)?))
            })),
        }
    }

    pub fn of(&self, value: T) -> StateEffect {
        StateEffect {
            annotation: self.key.of(value),
            mapper: self.mapper.clone(),
        }
    }
}

impl<T: Send + Sync + 'static> Default for StateEffectType<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for StateEffectType<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            mapper: self.mapper.clone(),
        }
    }
}

/// A transaction effect with a typed value and optional position mapping.
#[derive(Clone)]
pub struct StateEffect {
    annotation: Annotation,
    mapper: Option<Mapper>,
}

impl StateEffect {
    pub fn get<T: Send + Sync + 'static>(&self, kind: &StateEffectType<T>) -> Option<&T> {
        self.annotation.get(&kind.key)
    }

    pub fn map(&self, changes: &ChangeDesc) -> Option<Self> {
        let Some(map) = &self.mapper else {
            return Some(self.clone());
        };
        let value = map(&self.annotation.value, changes)?;
        Some(Self {
            annotation: Annotation {
                id: self.annotation.id.clone(),
                value,
            },
            mapper: self.mapper.clone(),
        })
    }

    pub(crate) fn same_value(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.annotation.id, &other.annotation.id)
            && Arc::ptr_eq(&self.annotation.value, &other.annotation.value)
    }

    /// Map effects in order and omit those discarded by their mapping rules.
    /// Call each rule even for an empty change description.
    pub fn map_all(effects: &[Self], changes: &ChangeDesc) -> Vec<Self> {
        effects
            .iter()
            .filter_map(|effect| effect.map(changes))
            .collect()
    }
}

impl fmt::Debug for StateEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateEffect").finish_non_exhaustive()
    }
}
