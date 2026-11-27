use std::sync::{Arc, LazyLock};

use crate::{EditorState, Extension, Facet, Transaction};

const DEFAULT_DEPTH: usize = 100;
const DEFAULT_DELAY: u64 = 500;

type Join = Arc<dyn Fn(&Transaction, bool) -> bool + Send + Sync>;

#[derive(Default)]
struct Input {
    min_depth: Option<usize>,
    new_group_delay: Option<u64>,
    join_to_event: Option<Join>,
}

type HistoryConfig = Facet<Input, Option<Config>>;
static CONFIG: LazyLock<HistoryConfig> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Input]| {
        (!inputs.is_empty()).then(|| Config {
            min_depth: inputs
                .iter()
                .filter_map(|input| input.min_depth)
                .max()
                .unwrap_or(DEFAULT_DEPTH),
            new_group_delay: inputs
                .iter()
                .filter_map(|input| input.new_group_delay)
                .min()
                .unwrap_or(DEFAULT_DELAY),
            join_to_event: inputs
                .iter()
                .filter_map(|input| input.join_to_event.clone())
                .collect(),
        })
    })
});

pub(super) struct Config {
    pub min_depth: usize,
    new_group_delay: u64,
    join_to_event: Vec<Join>,
}

impl Config {
    pub fn needs_transaction(&self) -> bool {
        !self.join_to_event.is_empty()
    }

    pub fn joins(&self, transaction: Option<&Transaction>, adjacent: bool) -> bool {
        if self.needs_transaction() {
            let transaction = transaction.expect("configured transaction");
            self.join_to_event
                .iter()
                .any(|join| join(transaction, adjacent))
        } else {
            adjacent
        }
    }
}

impl EditorState {
    /// Enable the CodeMirror history policy for a native [`crate::History`] or
    /// [`crate::view::EditorView`] that uses this state's extensions. The minimum
    /// branch depth defaults to 100. The exclusive grouping delay defaults to
    /// 500 milliseconds. Use [`Transaction::with_time`] to supply timestamps.
    /// If there are multiple inputs, use the largest explicit depth and
    /// shortest explicit delay. The policy retains up to 20 extra branch
    /// entries before it prunes. It does not record edits on a standalone
    /// `EditorState`.
    pub fn history_config(min_depth: Option<usize>, new_group_delay: Option<u64>) -> Extension {
        CONFIG.of(Input {
            min_depth,
            new_group_delay,
            ..Input::default()
        })
    }

    /// Enable the CodeMirror history policy with a custom edit join predicate.
    /// The inputs are the accepted transaction and whether its changes touch
    /// the previous edit. The predicates replace the default adjacency check
    /// and run in extension precedence order until one returns `true`.
    /// Time and user event checks still apply. Composition updates can join
    /// even when the predicates do not permit it.
    pub fn history_join_to_event(
        join: impl Fn(&Transaction, bool) -> bool + Send + Sync + 'static,
    ) -> Extension {
        CONFIG.of(Input {
            join_to_event: Some(Arc::new(join)),
            ..Input::default()
        })
    }

    pub(super) fn configured_history(&self) -> Arc<Option<Config>> {
        self.context().facet(&CONFIG).expect("typed history policy")
    }
}

/// No wall clock is read by the native editor. Configured histories use the
/// upstream clock reset value, except when the host sets an explicit boundary.
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Clock {
    #[default]
    Reset,
    Boundary,
    Missing,
    At(u64),
}

impl Clock {
    pub fn recorded(time: Option<u64>) -> Self {
        time.map_or(Self::Missing, Self::At)
    }

    pub fn within(self, now: Option<u64>, config: Option<&Config>) -> bool {
        let before = match self {
            Self::At(time) => time,
            Self::Reset if config.is_some() => 0,
            _ => return false,
        };
        let Some(now) = now else { return false };
        match now.checked_sub(before) {
            Some(elapsed) => {
                elapsed < config.map_or(DEFAULT_DELAY, |policy| policy.new_group_delay)
            }
            None => config.is_some(),
        }
    }

    pub fn joins_composition(self, now: Option<u64>, config: Option<&Config>) -> bool {
        if config.is_some() {
            !matches!(self, Self::Boundary)
        } else {
            matches!((self, now), (Self::At(before), Some(now)) if now >= before)
        }
    }

    pub fn isolated(self, config: Option<&Config>) -> Self {
        match self {
            Self::Boundary => Self::Boundary,
            Self::At(0) if config.is_some() => Self::At(0),
            _ => Self::Reset,
        }
    }
}
