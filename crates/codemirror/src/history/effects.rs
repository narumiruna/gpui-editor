use std::sync::{Arc, LazyLock};

use crate::{
    AppliedEdit, ChangeDesc, EditError, EditorState, Extension, Facet, StateContext, StateEffect,
    Transaction,
};

type Inverter =
    Arc<dyn Fn(&StateContext<'_>, &Transaction, &ChangeDesc) -> Vec<StateEffect> + Send + Sync>;
static INVERTERS: LazyLock<Facet<Inverter, Vec<Inverter>>> = LazyLock::new(Facet::collect);

/// Register an effect inverter for [`crate::History`]. The callback receives
/// the state before the transaction, the accepted transaction, and its change
/// description. Return effects in coordinates of the state before the transaction.
/// History also calls the inverter when it replays an event.
pub fn inverted_effects(
    invert: impl Fn(&StateContext<'_>, &Transaction, &ChangeDesc) -> Vec<StateEffect>
    + Send
    + Sync
    + 'static,
) -> Extension {
    INVERTERS.of(Arc::new(invert))
}

pub(super) fn apply(
    state: &mut EditorState,
    transaction: Transaction,
    record: bool,
) -> Result<(Option<AppliedEdit>, Vec<StateEffect>), EditError> {
    let inversion = if record && state.extensions.active() {
        let inverters = state.context().facet(&INVERTERS)?;
        (!inverters.is_empty()).then(|| (inverters, state.clone(), transaction.clone()))
    } else {
        None
    };
    let edit = state.apply_resolved(transaction)?;
    let effects = inversion.map_or_else(Vec::new, |(inverters, start, mut accepted)| {
        // Use the same changes as the state fields.
        accepted.changes = edit
            .as_ref()
            .map_or_else(Default::default, |edit| edit.forward.clone());
        let changes = accepted
            .changes()
            .desc(start.text().len())
            .expect("accepted changes fit the old document");
        let context = start.context();
        inverters
            .iter()
            .flat_map(|invert| invert(&context, &accepted, &changes))
            .collect()
    });
    Ok((edit, effects))
}
