//! Host-driven focus effects from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt`.

use std::sync::{Arc, LazyLock};

use crate::{
    AnnotationType, ConfigurationError, EditorState, Extension, Facet, StateEffect, Transaction,
};

type Provider = Arc<dyn Fn(&EditorState, bool) -> Option<StateEffect> + Send + Sync>;

static FOCUS_CHANGE_EFFECTS: LazyLock<Facet<Provider, Vec<Provider>>> =
    LazyLock::new(Facet::collect);
static IS_FOCUS_CHANGE: LazyLock<AnnotationType<bool>> = LazyLock::new(AnnotationType::new);

/// Register a provider of one typed effect for a focus transition.
/// Providers run in extension precedence order. Return `None` to contribute no effect.
/// Each provider reads the same state, before any focus effects are applied.
pub fn focus_change_effect(
    provider: impl Fn(&EditorState, bool) -> Option<StateEffect> + Send + Sync + 'static,
) -> Extension {
    FOCUS_CHANGE_EFFECTS.of(Arc::new(provider))
}

/// Build a focus transaction for a change in host-reported focus.
///
/// Pass the last reported focus and the current focus. Equal values do not run
/// providers. If focus changed but no provider returned an effect, the host
/// can report a focus-only view update without dispatching a transaction.
///
/// Pass the current state after any preceding edits, including reconfiguration.
/// Dispatch the result separately from those edits, against the state used to
/// build it. This is a proposal: dispatch still runs the state's filters and
/// history rules. The builder does not track focus, schedule work, or use layout.
pub fn focus_change_transaction(
    state: &EditorState,
    previous_focused: bool,
    focused: bool,
) -> Result<Option<Transaction>, ConfigurationError> {
    if previous_focused == focused {
        return Ok(None);
    }
    let providers = state.context().facet(&FOCUS_CHANGE_EFFECTS)?;
    let effects: Vec<_> = providers
        .iter()
        .filter_map(|provider| provider(state, focused))
        .collect();
    Ok((!effects.is_empty()).then(|| {
        Transaction::default()
            .with_effects(effects)
            .with_annotation(IS_FOCUS_CHANGE.of(true))
    }))
}

impl Transaction {
    /// Whether this transaction carries the focus builder's marker.
    /// A filter that replaces the transaction can discard this marker.
    pub fn is_focus_change(&self) -> bool {
        self.annotation(&IS_FOCUS_CHANGE) == Some(&true)
    }
}
