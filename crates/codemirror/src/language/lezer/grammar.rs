use std::{fmt, sync::Arc};

use super::{
    Action, ActionKind, ContextTracker, STATE_SIZE, SerializedTables, Specializer, TableError,
    Tables, Tokenizer, action, context::ErasedTracker, tokenize,
};

/// The supported fields of a v14 grammar spec. Select one entry from `topRules`.
/// Node properties do not affect this parser.
#[derive(Clone, Copy, Debug)]
pub struct GrammarSpec<'a, T = u8> {
    pub tables: SerializedTables<'a>,
    /// Space-separated names in term ID order, including the error node.
    pub node_names: &'a str,
    pub max_term: u16,
    pub repeat_node_count: u16,
    /// Tokenizer slots in priority order. State masks index this list.
    pub tokenizers: &'a [T],
    /// `[start state, root term]` from one entry in `topRules`.
    pub top_rule: [u16; 2],
    /// Offset of the token precedence list in `stateData`.
    pub token_prec: usize,
}

/// An invalid table or invalid metadata in a grammar spec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrammarError {
    Tables(TableError),
    InvalidMetadata(&'static str),
    InvalidState { state: u16 },
    InvalidAction { offset: usize },
    InvalidGoto { offset: usize },
    InvalidTokenData { offset: usize },
    InvalidTokenPrecedence { offset: usize },
    InvalidLocalTokenData { offset: usize },
    InvalidLocalTokenPrecedence { offset: usize },
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tables(error) => error.fmt(f),
            Self::InvalidMetadata(field) => write!(f, "invalid Lezer {field}"),
            Self::InvalidState { state } => write!(f, "invalid Lezer parser state {state}"),
            Self::InvalidAction { offset } => write!(f, "invalid Lezer action at word {offset}"),
            Self::InvalidGoto { offset } => write!(f, "invalid Lezer goto group at word {offset}"),
            Self::InvalidTokenData { offset } => write!(f, "invalid Lezer DFA at word {offset}"),
            Self::InvalidTokenPrecedence { offset } => {
                write!(f, "invalid Lezer token precedence at word {offset}")
            }
            Self::InvalidLocalTokenData { offset } => {
                write!(f, "invalid Lezer local DFA at word {offset}")
            }
            Self::InvalidLocalTokenPrecedence { offset } => {
                write!(
                    f,
                    "invalid Lezer local token precedence list at word {offset}"
                )
            }
        }
    }
}

impl std::error::Error for GrammarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tables(error) => Some(error),
            _ => None,
        }
    }
}

/// A validated v14 parser for strict LR, bounded GLR, and bounded recovery.
#[derive(Debug)]
pub struct Parser {
    pub(super) tables: Tables,
    pub(super) names: Arc<[Box<str>]>,
    pub(super) top_rule: [u16; 2],
    pub(super) eof: u16,
    pub(super) max_term: u16,
    pub(super) tokenizers: Box<[Tokenizer]>,
    pub(super) precedence: Box<[u16]>,
    pub(super) specializers: Box<[Specializer]>,
    pub(super) dynamic_precedence: Box<[Option<i8>]>,
    pub(super) context_tracker: Option<ErasedTracker>,
}

impl Parser {
    /// Decode a spec with numeric tokenizer groups and no specializers.
    pub fn deserialize(spec: GrammarSpec<'_>) -> Result<Self, GrammarError> {
        Self::deserialize_spec(spec, &[])
    }

    /// Decode a spec with shared, local, or external tokenizers and optional
    /// specialization handlers in generated order. Check LR stack transitions
    /// and progress during parsing.
    pub fn deserialize_with_hooks(
        spec: GrammarSpec<'_, Tokenizer>,
        specializers: &[Specializer],
    ) -> Result<Self, GrammarError> {
        Self::deserialize_spec(spec, specializers)
    }

    fn deserialize_spec<T: Clone + Into<Tokenizer>>(
        spec: GrammarSpec<'_, T>,
        specializers: &[Specializer],
    ) -> Result<Self, GrammarError> {
        let tables = Tables::deserialize(spec.tables).map_err(GrammarError::Tables)?;
        let names: Arc<[Box<str>]> = spec.node_names.split(' ').map(Box::from).collect();
        let named = names.len();
        let eof = u16::try_from(named + usize::from(spec.repeat_node_count))
            .ok()
            .filter(|&eof| eof <= spec.max_term && spec.max_term < u16::MAX)
            .ok_or(GrammarError::InvalidMetadata("maxTerm or node count"))?;
        if names.iter().any(|name| name.is_empty()) {
            return Err(GrammarError::InvalidMetadata("nodeNames"));
        }
        let [start, root] = spec.top_rule;
        let in_states = |state| usize::from(state) < tables.state_count();
        if tables.state(start).is_none_or(|state| state.is_skipped())
            || root == 0
            || usize::from(root) >= named
        {
            return Err(GrammarError::InvalidMetadata("topRules entry"));
        }
        if spec.tokenizers.len() > u32::BITS as usize {
            return Err(GrammarError::InvalidMetadata("tokenizers"));
        }
        let tokenizers: Box<[Tokenizer]> =
            spec.tokenizers.iter().cloned().map(Into::into).collect();
        let valid_token = |term| is_token_term(term, named, eof, spec.max_term);
        for tokenizer in &tokenizers {
            match tokenizer {
                Tokenizer::Internal(id) if *id >= u16::BITS as u8 => {
                    return Err(GrammarError::InvalidMetadata("tokenizers"));
                }
                Tokenizer::Local(group) => group.validate(valid_token)?,
                _ => {}
            }
        }
        if specializers.iter().any(|specializer| {
            !valid_token(specializer.term) || !specializer.result_terms_valid(valid_token)
        }) {
            return Err(GrammarError::InvalidMetadata("specialized"));
        }
        let valid_action = |action: Action| {
            action.has_valid_flags()
                && match action.kind() {
                    ActionKind::Shift(state) | ActionKind::Goto(state) => in_states(state),
                    ActionKind::Skip => true,
                    ActionKind::Reduce {
                        term,
                        depth,
                        stay,
                        repeat,
                    } => {
                        term != 0
                            && term != eof
                            && term <= spec.max_term
                            && (!stay || depth > 0)
                            && (!repeat || (usize::from(term) >= named && term < eof))
                    }
                }
        };
        let rows = tables.states.as_chunks::<STATE_SIZE>().0.iter();
        for (state, row) in rows.clone().enumerate() {
            if row[0] & !3 != 0
                || u64::from(row[3]) >> spec.tokenizers.len() != 0
                || row[4..]
                    .iter()
                    .any(|&code| code != 0 && !valid_action(Action::from_raw(code)))
            {
                return Err(GrammarError::InvalidState {
                    state: state as u16,
                });
            }
        }
        action::validate(
            &tables.state_data,
            rows.flat_map(|row| [row[1], row[2]]),
            |term, action| {
                term.is_none_or(|term| term <= spec.max_term)
                    && (term.is_some_and(|term| term != eof) || action.is_reduce())
                    && valid_action(action)
            },
        )
        .map_err(|offset| GrammarError::InvalidAction { offset })?;
        if tables.goto[0] > spec.max_term + 1 {
            return Err(GrammarError::InvalidGoto { offset: 0 });
        }
        super::validate_goto(&tables.goto, |group| {
            in_states(group.target) && group.sources.iter().copied().all(in_states)
        })
        .map_err(|offset| GrammarError::InvalidGoto { offset })?;
        tokenize::validate(&tables.token_data, |term| {
            term != 0 && term != eof && term <= spec.max_term
        })
        .map_err(|offset| GrammarError::InvalidTokenData { offset })?;
        let precedence = tokenize::precedence(&tables.state_data, spec.token_prec, spec.max_term)
            .map_err(|offset| GrammarError::InvalidTokenPrecedence { offset })?;
        Ok(Self {
            tables,
            names,
            top_rule: spec.top_rule,
            eof,
            max_term: spec.max_term,
            tokenizers,
            precedence,
            specializers: specializers.into(),
            dynamic_precedence: Box::default(),
            context_tracker: None,
        })
    }

    /// Set the grammar's context tracker without opting in to checkpoints.
    pub fn with_context_tracker<C: Eq + Send + Sync + 'static>(
        self,
        tracker: ContextTracker<C>,
    ) -> Self {
        self.set_context_tracker(tracker.erase(false))
    }

    /// Set the context tracker and opt it in to strict LR prefix checkpoints.
    /// A resumed parse can omit prefix updates and restore their context value.
    /// Both hooks must be pure functions of the old context value, action term,
    /// stack values, cursor, and fixed, immutable configuration. Read document
    /// text only through [`TokenInput`](super::TokenInput) and propagate all input
    /// and stack errors. Do not use mutable state, object identity, or side effects.
    /// Context values must be immutable and their equality must be pure: equal
    /// values must have the same effect on hooks and tokenizers. Rust does not
    /// check this contract. This does not opt in the tokenizer or specializers.
    pub fn with_deterministic_context_tracker<C: Eq + Send + Sync + 'static>(
        self,
        tracker: ContextTracker<C>,
    ) -> Self {
        self.set_context_tracker(tracker.erase(true))
    }

    fn set_context_tracker(mut self, tracker: ErasedTracker) -> Self {
        // Trees use the names allocation as the strict parser identity. Make
        // a new allocation if old trees still refer to the previous tracker.
        Arc::make_mut(&mut self.names);
        self.context_tracker = Some(tracker);
        self
    }

    /// Set generated `dynamicPrecedences` for GLR and recovering parses.
    /// Terms must be distinct and in range, excluding the error and EOF terms.
    /// Scores must be in `-10..=10`, as required by the generator.
    pub fn with_dynamic_precedence(mut self, entries: &[(u16, i8)]) -> Result<Self, GrammarError> {
        let invalid = GrammarError::InvalidMetadata("dynamicPrecedences");
        let mut scores = vec![None; usize::from(self.max_term) + 1];
        for &(term, score) in entries {
            if term == 0 || term == self.eof || !(-10..=10).contains(&score) {
                return Err(invalid);
            }
            if scores
                .get_mut(usize::from(term))
                .ok_or(invalid)?
                .replace(score)
                .is_some()
            {
                return Err(invalid);
            }
        }
        self.dynamic_precedence = scores.into();
        Ok(self)
    }

    pub(super) fn is_token_term(&self, term: u16) -> bool {
        is_token_term(term, self.names.len(), self.eof, self.max_term)
    }

    pub fn tables(&self) -> &Tables {
        &self.tables
    }
}

fn is_token_term(term: u16, named: usize, eof: u16, max_term: u16) -> bool {
    term != 0 && term <= max_term && (usize::from(term) < named || term > eof)
}
