use std::{
    hash::BuildHasher,
    sync::{Arc, LazyLock},
};

use crate::{
    AnnotationType, ChangeSet, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Facet, StateEffect, StateField, Text, Transaction,
    extension::{Extensions, Reconfiguration},
    state_config::PRESERVE_REPLACEMENTS,
};

use super::{
    Collab, CollabError, OriginStates, SharedEffects, SharedEffectsContext, TransactionContext,
    Update,
};

/// Settings for the collaboration state extension.
#[derive(Clone, Default)]
pub struct CollabConfig {
    /// A unique ID for this peer. Omit it to generate one for this extension.
    pub client_id: Option<String>,
    /// The initial version of the authority's document. On a default config,
    /// zero leaves this setting unspecified when combining extensions.
    pub start_version: usize,
    explicit_start_version: bool,
    share: Option<Arc<SharedEffects>>,
}

impl CollabConfig {
    pub fn new(client_id: impl Into<String>, start_version: usize) -> Self {
        Self {
            client_id: Some(client_id.into()),
            start_version,
            explicit_start_version: true,
            share: None,
        }
    }

    /// Set the initial version explicitly, including zero. The client ID
    /// can still be generated if none is configured.
    pub fn with_start_version(mut self, version: usize) -> Self {
        self.start_version = version;
        self.explicit_start_version = true;
        self
    }

    /// Select the effects to send from each accepted local transaction.
    pub fn with_shared_effects(
        self,
        share: impl Fn(&Transaction) -> Vec<StateEffect> + Send + Sync + 'static,
    ) -> Self {
        self.with_shared_effects_context(move |context| share(context.transaction()))
    }

    /// Select effects using the accepted transaction and its text and selection
    /// snapshots. This replaces any previously configured sharing callback.
    pub fn with_shared_effects_context(
        self,
        share: impl Fn(&TransactionContext) -> Vec<StateEffect> + Send + Sync + 'static,
    ) -> Self {
        self.with_shared_effects_state(move |context| share(context.transaction))
    }

    /// Select effects using both state views of the accepted transaction.
    /// The queued update keeps the resolved fields and facets available to the host.
    pub fn with_shared_effects_state(
        mut self,
        share: impl Fn(&SharedEffectsContext<'_>) -> Vec<StateEffect> + Send + Sync + 'static,
    ) -> Self {
        self.share = Some(Arc::new(share));
        self
    }
}

struct ConfigInput {
    config: CollabConfig,
    generated_id: String,
}

struct Config {
    client_id: String,
    start_version: usize,
    share: Option<Arc<SharedEffects>>,
}

static CONFIG: LazyLock<Facet<ConfigInput, Result<Config, ConfigurationError>>> =
    LazyLock::new(|| Facet::new(combine_config));

fn combine_config(inputs: &[&ConfigInput]) -> Result<Config, ConfigurationError> {
    let configs = || inputs.iter().map(|input| &input.config);
    let client_id = combine(
        configs().filter_map(|config| config.client_id.as_ref()),
        |a, b| a == b,
    )?;
    let start_version = combine(
        configs().filter_map(|config| {
            (config.explicit_start_version || config.start_version != 0)
                .then_some(config.start_version)
        }),
        usize::eq,
    )?;
    let share = combine(
        configs().filter_map(|config| config.share.as_ref()),
        |a, b| Arc::ptr_eq(a, b),
    )?;
    Ok(Config {
        client_id: client_id
            .or_else(|| inputs.first().map(|input| &input.generated_id))
            .cloned()
            .unwrap_or_default(),
        start_version: start_version.unwrap_or_default(),
        share: share.cloned(),
    })
}

fn combine<T>(
    mut inputs: impl Iterator<Item = T>,
    equal: impl Fn(&T, &T) -> bool,
) -> Result<Option<T>, ConfigurationError> {
    let first = inputs.next();
    if let Some(first) = &first
        && !inputs.all(|input| equal(first, &input))
    {
        return Err(ConfigurationError::InvalidFieldData);
    }
    Ok(first)
}

struct Acknowledged {
    from: Arc<()>,
    next: Collab,
}

static ACKNOWLEDGED: LazyLock<AnnotationType<Acknowledged>> = LazyLock::new(AnnotationType::new);

static COLLAB_STATE: LazyLock<StateField<Collab>> = LazyLock::new(|| {
    StateField::new(
        |state| {
            let config = state.facet(&CONFIG)?;
            let config = config.as_ref().as_ref().map_err(|error| *error)?;
            let mut session =
                Collab::from_text(config.client_id.clone(), config.start_version, state.text());
            session.share.clone_from(&config.share);
            Ok(session)
        },
        |previous: &Collab, update| {
            let transaction = update.transaction();
            let text = update.state().text();
            let config = update.state().facet(&CONFIG)?;
            let config = config.as_ref().as_ref().map_err(|error| *error)?;
            let mut next = if let Some(ack) = transaction.annotation(&ACKNOWLEDGED) {
                if !Arc::ptr_eq(&previous.generation, &ack.from)
                    || text != &ack.next.optimistic_text
                {
                    return Err(ConfigurationError::InvalidFieldData);
                }
                ack.next.clone()
            } else {
                let start = update.start();
                let start_config = start.facet(&CONFIG)?;
                let start_config = start_config.as_ref().as_ref().map_err(|error| *error)?;
                let mut next = previous.clone();
                // Publish the reconfiguration transaction with its start policy.
                next.client_id.clone_from(&start_config.client_id);
                next.share.clone_from(&start_config.share);
                next.record(transaction.clone(), *start, *update.state())
                    .map_err(|_| ConfigurationError::InvalidFieldData)?;
                next
            };
            if previous.client_id != config.client_id {
                next.identity = Arc::new(());
                next.generation = Arc::new(());
            }
            next.client_id.clone_from(&config.client_id);
            next.share.clone_from(&config.share);
            Ok(next)
        },
    )
});

/// Install collaboration state. Ordinary transactions, including local undo
/// and redo, queue their accepted changes and selected effects automatically.
/// Multiple instances combine settings. Conflicting settings reject the
/// configuration with [`ConfigurationError::InvalidFieldData`].
/// Reload from the authority before replacing a session. Do not change a live
/// client's ID while it has unacknowledged or in-flight updates.
pub fn collab(config: CollabConfig) -> Extension {
    let random = std::hash::RandomState::new().hash_one(());
    let input = ConfigInput {
        config,
        generated_id: format!("{random:016x}"),
    };
    Extension::group([
        COLLAB_STATE.extension(),
        CONFIG.of(input),
        PRESERVE_REPLACEMENTS.of(()),
    ])
}

pub(crate) fn capture_origin_states(
    before: &Extensions,
    after: &Extensions,
    text: &Text,
    selection: &EditorSelection,
) -> Result<(), ConfigurationError> {
    if let Some(field) = LazyLock::get(&COLLAB_STATE)
        && let Some(session) = after.context(text, selection).field(field)?
    {
        session.capture_origin_states(before, after);
    }
    Ok(())
}

pub(super) fn origin_states(before: &Extensions, after: &Extensions) -> OriginStates {
    // This private field is the only registry entry that can own the queue
    // under construction. Capturing it would let the origin own itself and
    // would retain the entire pending vector at each earlier origin.
    let field = COLLAB_STATE.reader();
    OriginStates {
        start: before.capture_without(field),
        result: after.capture_without(field),
    }
}

/// Read the configured client ID, synchronized version, and sendable updates.
pub fn collab_state(state: &EditorState) -> Result<Option<Arc<Collab>>, ConfigurationError> {
    state.context().field(&COLLAB_STATE)
}

/// Return a snapshot of the local queue, in send order. This does not
/// acknowledge any updates. Use [`collab_state`] to read the queue without cloning it.
pub fn sendable_updates(state: &EditorState) -> Result<Vec<Update>, CollabError> {
    Ok(require_collab(state)?.sendable_updates().to_vec())
}

/// Return the authority version through the last received update.
pub fn get_synced_version(state: &EditorState) -> Result<usize, CollabError> {
    Ok(require_collab(state)?.synced_version())
}

/// Return this editor's client ID, or an empty string if collaboration is disabled.
pub fn get_client_id(state: &EditorState) -> Result<String, CollabError> {
    Ok(collab_state(state)
        .map_err(EditError::from)?
        .map_or_else(String::new, |session| session.client_id().to_owned()))
}

fn require_collab(state: &EditorState) -> Result<Arc<Collab>, CollabError> {
    collab_state(state)
        .map_err(EditError::from)?
        .ok_or(CollabError::NotConfigured)
}

pub(crate) fn validate_receipt(
    state: &EditorState,
    transaction: &Transaction,
    reconfiguration: Option<&Reconfiguration>,
) -> Result<(), ConfigurationError> {
    let mut receipts = transaction
        .annotations
        .iter()
        .filter_map(|annotation| annotation.get(&ACKNOWLEDGED));
    let Some(receipt) = receipts.next() else {
        return Ok(());
    };
    let current = collab_state(state)?;
    if receipts.next().is_some()
        || reconfiguration.is_some_and(|config| !config.has_facet(&CONFIG))
        || current.is_none_or(|current| !Arc::ptr_eq(&current.generation, &receipt.from))
    {
        return Err(ConfigurationError::InvalidFieldData);
    }
    Ok(())
}

/// Prepare an authority transaction for this state's collaboration extension.
/// Dispatch it to the same editor while the extension is still enabled, before
/// other shared edits or acknowledgements. Dispatch rejects a stale receipt,
/// multiple receipts in one transaction, or removal of the extension.
/// The transaction is not recorded as local undo, and history maps pending
/// events over it. Unlike upstream, even empty and acknowledgement-only
/// receipts bypass filters and opt out of history. Extenders can still run.
pub fn receive_updates(
    state: &EditorState,
    from_version: usize,
    updates: &[Update],
) -> Result<Transaction, CollabError> {
    let current = require_collab(state)?;
    let (next, transaction) =
        if let Some(receipt) = current.prepare(state.text(), from_version, updates)? {
            (receipt.next, receipt.transaction.unwrap_or_default())
        } else {
            ((*current).clone(), Transaction::default())
        };
    Ok(session_receipt(transaction, &current, next))
}

/// Prepare a receipt that replaces the text-only suffix queued since `start`
/// with one update. If the text matches `start`, remove the suffix instead.
/// The host must keep this suffix private until dispatching the receipt.
///
/// The receipt does not edit the document or add history. It uses the current
/// queue generation, so dispatch rejects it if subsequent updates change that
/// queue. The returned change set describes the grouped edit in `start` bytes.
/// Return `None` if there are no new queued updates.
pub fn prepare_text_update_group(
    start: &EditorState,
    state: &EditorState,
) -> Result<Option<(Transaction, ChangeSet)>, CollabError> {
    let initial = require_collab(start)?;
    let current = require_collab(state)?;
    initial.check_state(start.text())?;
    current.check_state(state.text())?;
    let length = initial.pending.len();
    let Some((prefix, suffix)) = current.pending.split_at_checked(length) else {
        return Err(CollabError::StateMismatch);
    };
    if !Arc::ptr_eq(&current.identity, &initial.identity)
        || current.version != initial.version
        || current.synced_text != initial.synced_text
        || prefix.iter().zip(initial.pending.iter()).any(|(a, b)| {
            a != b
                || match (&a.origin, &b.origin) {
                    (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                }
        })
    {
        return Err(CollabError::StateMismatch);
    }
    let Some((first, rest)) = suffix.split_first() else {
        return if start.text() == state.text() {
            Ok(None)
        } else {
            Err(CollabError::StateMismatch)
        };
    };
    if suffix.iter().any(|update| !update.effects.is_empty()) {
        return Err(CollabError::SharedEffectsInTextGroup);
    }
    if suffix
        .iter()
        .any(|update| update.client_id != current.client_id)
    {
        return Err(CollabError::StateMismatch);
    }
    let mut combined = first.edit.clone();
    for update in rest {
        combined = combined.compose(&update.edit)?;
    }
    let mut result = start.text().clone();
    combined.check_length(result.len())?;
    combined.changes.apply_to(&mut result)?;
    if &result != state.text() {
        return Err(CollabError::StateMismatch);
    }
    let mut pending = prefix.to_vec();
    let changes = if start.text() == state.text() {
        ChangeSet::default()
    } else {
        let changes = combined.changes.clone();
        let transaction = Transaction::new(changes.clone())
            .with_selection(state.selection().clone())
            .with_user_event("input.type.compose");
        let origin = Arc::new(TransactionContext::new(
            transaction,
            start.context(),
            state.context(),
        ));
        origin
            .states
            .get_or_init(|| origin_states(&start.extensions, &state.extensions));
        pending.push(Update {
            client_id: current.client_id.clone(),
            edit: combined,
            effects: Vec::new(),
            origin: Some(origin),
        });
        changes
    };
    let mut next = (*current).clone();
    next.pending = Arc::new(pending);
    next.generation = Arc::new(());
    Ok(Some((
        session_receipt(Transaction::default(), &current, next),
        changes,
    )))
}

fn session_receipt(transaction: Transaction, current: &Collab, next: Collab) -> Transaction {
    transaction
        .with_annotation(ACKNOWLEDGED.of(Acknowledged {
            from: current.generation.clone(),
            next,
        }))
        .with_add_to_history(false)
        .without_filtering()
}
