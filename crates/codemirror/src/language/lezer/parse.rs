use std::{cell::Cell, fmt};

use super::{
    Action, ActionKind, Parser, State, SyntaxTree, Tokenizer,
    context::{Context, ContextUpdate},
    tokenize::{ActionLimit, Candidate, TokenCache},
    tree::{Roots, TreeBuilder},
};

mod glr;
pub(super) mod incremental;

use incremental::{CheckpointHistory, SuffixReuse};

pub(in crate::language) use glr::session;
pub use incremental::{ReuseStats, TreeFragment};

/// The parser could not finish a parse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// The lexer could not read a token, or the state has no action for it.
    UnexpectedToken {
        offset: usize,
        state: u16,
        term: Option<u16>,
    },
    /// More than one distinct action matches the lookahead in [`Parser::parse`].
    Ambiguous {
        offset: usize,
        state: u16,
        term: u16,
    },
    /// A tokenizer or specializer returned a reserved or out-of-range term.
    InvalidTokenTerm { offset: usize, term: u16 },
    /// An external token has an invalid absolute UTF-16 end offset.
    InvalidTokenEnd { offset: usize, end: usize },
    /// The action needs a stack frame or goto that does not exist.
    InvalidTransition { offset: usize, state: u16 },
    /// The parser or tokenizer exceeded its work or progress limit.
    LimitExceeded { offset: usize },
}

impl ParseError {
    pub(crate) fn map_offset(mut self, map: impl FnOnce(usize) -> usize) -> Self {
        let offset = match &mut self {
            Self::UnexpectedToken { offset, .. }
            | Self::Ambiguous { offset, .. }
            | Self::InvalidTokenTerm { offset, .. }
            | Self::InvalidTokenEnd { offset, .. }
            | Self::InvalidTransition { offset, .. }
            | Self::LimitExceeded { offset } => offset,
        };
        *offset = map(*offset);
        self
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedToken {
                offset,
                state,
                term: Some(term),
            } => {
                write!(
                    f,
                    "unexpected Lezer term {term} in state {state} at byte {offset}"
                )
            }
            Self::UnexpectedToken {
                offset,
                state,
                term: None,
            } => {
                write!(f, "no Lezer token in state {state} at byte {offset}")
            }
            Self::Ambiguous {
                offset,
                state,
                term,
            } => {
                write!(
                    f,
                    "ambiguous Lezer actions for term {term} in state {state} at byte {offset}"
                )
            }
            Self::InvalidTokenTerm { offset, term } => {
                write!(f, "invalid Lezer token term {term} at byte {offset}")
            }
            Self::InvalidTokenEnd { offset, end } => {
                write!(
                    f,
                    "invalid Lezer token end at UTF-16 offset {end} for byte {offset}"
                )
            }
            Self::InvalidTransition { offset, state } => {
                write!(
                    f,
                    "invalid Lezer transition in state {state} at byte {offset}"
                )
            }
            Self::LimitExceeded { offset } => {
                write!(f, "Lezer parse limit exceeded at byte {offset}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Debug)]
pub(super) struct Work(Cell<Option<usize>>);

impl Work {
    fn new(units: usize) -> Self {
        Self(Cell::new(Some(units)))
    }

    fn remaining(&self) -> usize {
        self.0.get().expect("parse stops when work is exhausted")
    }

    pub(super) fn spend(&self, offset: usize, units: usize) -> Result<(), ParseError> {
        let remaining = self.0.get().and_then(|left| left.checked_sub(units));
        self.0.set(remaining);
        remaining
            .map(|_| ())
            .ok_or(ParseError::LimitExceeded { offset })
    }
}

const REDUCTION_GOTO_SLOTS: usize = 32;

#[derive(Clone, Copy)]
struct ReductionGoto {
    source: u16,
    term: u16,
    target: u16,
}

#[derive(Default)]
pub(super) struct ReductionGotos([Option<ReductionGoto>; REDUCTION_GOTO_SLOTS]);

#[derive(Clone, Copy)]
struct Frame {
    state: u16,
    start: usize,
    output_base: Roots,
}

#[derive(Clone)]
struct Stack {
    frames: Vec<Frame>,
    trial_frames: Option<TrialFrames>,
    roots: Roots,
    pos: usize,
    // Skipped tokens move `pos` but must not extend the next ordinary reduction.
    reduce_pos: usize,
    context: Option<Context>,
}

// Until a JSON force trial ends, the original prefix below `start` is
// untouched. The journal holds the original suffix in reverse order.
#[derive(Clone)]
struct TrialFrames {
    start: usize,
    saved: Vec<Frame>,
}

/// A read-only view of the active parse stack.
pub struct StackView<'a> {
    pub(super) parser: &'a Parser,
    stack: &'a Stack,
    pub(super) work: &'a Work,
}

impl StackView<'_> {
    /// The active LR state ID.
    pub fn state(&self) -> u16 {
        self.stack.state()
    }

    /// The current input position in bytes. Context hooks see the position
    /// after the action; their input cursor starts at the token or rule start.
    pub fn byte_offset(&self) -> usize {
        self.stack.pos
    }

    /// The current branch's context, if a tracker of this type is set.
    pub fn context<C: 'static>(&self) -> Option<&C> {
        self.context_value()?.get()
    }

    pub(super) fn context_value(&self) -> Option<&Context> {
        self.stack.context.as_ref()
    }

    /// Check the first action for `term`, simulating default and lookahead
    /// reductions without changing the real stack. Like Lezer's probe,
    /// this treats any non-reduction action as a match. Gotos must be exact.
    pub fn can_shift(&self, term: u16) -> Result<bool, ParseError> {
        let pos = self.stack.pos;
        let tables = &self.parser.tables;
        let mut prefix = self.stack.frames.len();
        let mut extra = Vec::new();
        if term > self.parser.max_term {
            return Ok(false);
        }
        loop {
            self.work.spend(pos, 1)?;
            let current = extra
                .last()
                .copied()
                .unwrap_or_else(|| self.stack.frames[prefix - 1].state);
            let state = tables.state(current).expect("validated state");
            let action = if let Some(action) = state.default_reduce {
                Some(action)
            } else {
                tables.probe_action(state, term, || self.work.spend(pos, 1))?
            };
            let Some(action) = action else {
                return Ok(false);
            };
            let ActionKind::Reduce {
                term, depth, stay, ..
            } = action.kind()
            else {
                return Ok(true);
            };
            let Some((base, from)) = reduction_base(prefix + extra.len(), depth, stay) else {
                return Ok(false);
            };
            let source = if from < prefix {
                self.stack.frames[from].state
            } else {
                extra[from - prefix]
            };
            let target = if stay {
                source
            } else if let Some(target) = self.parser.strict_goto(source, term, pos, self.work)? {
                target
            } else {
                return Ok(false);
            };
            extra.truncate(base.saturating_sub(prefix));
            prefix = prefix.min(base);
            extra.push(target);
        }
    }
}

fn reduction_base(len: usize, depth: u16, stay: bool) -> Option<(usize, usize)> {
    // A stay reduction removes the entry into the skip rule and restores the
    // state that was current before that entry.
    let base = len.checked_sub(usize::from(depth) + if stay { 2 } else { 0 })?;
    let source = if stay { base } else { base.checked_sub(1)? };
    Some((base, source))
}

impl Parser {
    /// Parse the complete input. Uses strict gotos and returns an error rather
    /// than splitting the stack, recovering, or returning a partial tree.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        let work = self.work_budget(source, 128);
        let mut tree = TreeBuilder::default();
        let (roots, end) =
            self.run_lr::<false>(source, Stack::new(self), &mut tree, work, None, None)?;
        Ok(tree.finish(self, roots, end, None))
    }

    fn run_lr<const CHECK_EDITOR_DEPTH: bool>(
        &self,
        source: &str,
        mut stack: Stack,
        tree: &mut TreeBuilder,
        work: Work,
        mut history: Option<&mut CheckpointHistory>,
        mut suffix: Option<SuffixReuse<'_>>,
    ) -> Result<(Roots, usize), ParseError> {
        let mut idle = self.progress_budget(stack.frames.len());
        let mut tokens = TokenCache::new(source, self, false);
        if let Some(history) = &history {
            tokens.reads = Some(Cell::new(history.last_reads()));
        }
        let mut actions = Vec::new();
        loop {
            let start = stack.pos;
            work.spend(start, 1)?;
            idle.spend(start, 1)?;
            let state = self.tables.state(stack.state()).expect("validated state");
            let at_depth_limit = CHECK_EDITOR_DEPTH && stack.frames.len() >= glr::MAX_STACK_DEPTH;
            if let Some(action) = state.default_reduce {
                if at_depth_limit {
                    stack.check_depth(action)?;
                }
                stack.reduce(self, state, action, &work, tree, &mut tokens)?;
                continue;
            }
            // Default reductions can put the stack below the cut without
            // recovery. Otherwise let GLR handle the cut before token scan.
            if at_depth_limit && stack.frames.len() > glr::MAX_STACK_DEPTH {
                return Err(ParseError::LimitExceeded { offset: start });
            }
            let view = stack.view(self, &work);
            let main = if let Some(suffix) = &mut suffix {
                suffix.actions(&mut tokens, &view, state, &mut actions)?
            } else {
                tokens
                    .actions(&view, state, ActionLimit::Deterministic, &mut actions)?
                    .main
            };
            if let Some(&candidate) = actions.first() {
                if at_depth_limit {
                    stack.check_action_depth(self, candidate, &work)?;
                }
                stack.apply(self, state, candidate, &work, tree, &mut tokens)?;
                if stack.pos > start {
                    idle = self.progress_budget(stack.frames.len());
                    if let (Some(history), Some(reads)) = (&mut history, &tokens.reads) {
                        history.record(&stack, tree, reads.get(), &work);
                    }
                }
            } else if start == source.len() && state.is_accepting() {
                if let (Some(suffix), Some(history)) = (suffix, history) {
                    suffix.finish(history);
                }
                return Ok((stack.roots, start));
            } else {
                return Err(stack.unexpected_token(main.map(|token| token.term)));
            }
        }
    }

    fn work_budget(&self, source: &str, per_byte: usize) -> Work {
        // Long lookahead can be revisited at each byte, even in a DFA.
        let local_words = self
            .tokenizers
            .iter()
            .map(|tokenizer| match tokenizer {
                Tokenizer::Local(group) => group.data.len(),
                _ => 0,
            })
            .sum::<usize>();
        let fixed = 1024
            + self.tables.states.len()
            + self.tables.state_data.len()
            + self.tables.goto.len()
            + self.tables.token_data.len()
            + local_words
            + self.specializers.len();
        Work::new(fixed.saturating_add(source.len().saturating_mul(per_byte)))
    }

    fn progress_budget(&self, depth: usize) -> Work {
        Work::new(256 + (self.tables.state_count() + depth).saturating_mul(8))
    }

    fn strict_goto(
        &self,
        source: u16,
        term: u16,
        pos: usize,
        work: &Work,
    ) -> Result<Option<u16>, ParseError> {
        self.tables
            .lookup_goto(source, term, false, |words| work.spend(pos, words))
    }

    fn reduction_goto(
        &self,
        source: u16,
        term: u16,
        pos: usize,
        work: &Work,
        tokens: &mut TokenCache<'_>,
    ) -> Result<Option<u16>, ParseError> {
        let Some(cache) = tokens.reduction_gotos.as_deref_mut() else {
            return self.strict_goto(source, term, pos, work);
        };
        // Strict reductions reuse the same packed transitions across forks.
        // Charge the probe, and still meter the full table walk on a miss.
        work.spend(pos, 1)?;
        let index = ((usize::from(source) * 31) ^ usize::from(term)) & (REDUCTION_GOTO_SLOTS - 1);
        let slot = &mut cache.0[index];
        if let Some(entry) = slot
            && entry.source == source
            && entry.term == term
        {
            return Ok(Some(entry.target));
        }
        let target = self.strict_goto(source, term, pos, work)?;
        if let Some(target) = target {
            *slot = Some(ReductionGoto {
                source,
                term,
                target,
            });
        }
        Ok(target)
    }
}

impl Stack {
    fn new(parser: &Parser) -> Self {
        Self {
            frames: vec![Frame {
                state: parser.top_rule[0],
                start: 0,
                output_base: Roots::default(),
            }],
            trial_frames: None,
            roots: Roots::default(),
            pos: 0,
            reduce_pos: 0,
            context: parser.context_tracker.as_ref().map(|t| t.start.clone()),
        }
    }

    fn view<'a>(&'a self, parser: &'a Parser, work: &'a Work) -> StackView<'a> {
        StackView {
            parser,
            stack: self,
            work,
        }
    }

    fn unexpected_token(&self, term: Option<u16>) -> ParseError {
        ParseError::UnexpectedToken {
            offset: self.pos,
            state: self.state(),
            term,
        }
    }

    fn state(&self) -> u16 {
        self.frames.last().expect("stack has a start state").state
    }

    fn push(&mut self, state: u16, start: usize) {
        self.frames.push(Frame {
            state,
            start,
            output_base: self.roots,
        });
    }

    fn apply(
        &mut self,
        parser: &Parser,
        state: State,
        (action, token): Candidate,
        work: &Work,
        tree: &mut TreeBuilder,
        tokens: &mut TokenCache<'_>,
    ) -> Result<(), ParseError> {
        let start = self.pos;
        match action.kind() {
            ActionKind::Reduce { .. } => {
                return self.reduce(parser, state, action, work, tree, tokens);
            }
            ActionKind::Goto(target) => {
                self.push(target, start);
                return Ok(());
            }
            ActionKind::Shift(target) => {
                let skipped = parser
                    .tables
                    .state(target)
                    .expect("validated shift target")
                    .is_skipped();
                if !skipped && (token.end > start || token.term < parser.eof) {
                    self.reduce_pos = token.end;
                }
                self.push(
                    target,
                    if skipped {
                        start
                    } else {
                        start.min(self.reduce_pos)
                    },
                );
                self.pos = token.end;
            }
            ActionKind::Skip => self.pos = token.end,
        }
        self.update_context(
            parser,
            parser
                .context_tracker
                .as_ref()
                .and_then(|t| t.shift.as_deref()),
            token.term,
            start,
            work,
            tokens,
        )?;
        if usize::from(token.term) < parser.names.len() {
            tree.leaf(&mut self.roots, token.term, start..token.end);
        }
        Ok(())
    }

    fn reduce(
        &mut self,
        parser: &Parser,
        state: State,
        action: Action,
        work: &Work,
        tree: &mut TreeBuilder,
        tokens: &mut TokenCache<'_>,
    ) -> Result<(), ParseError> {
        let ActionKind::Reduce {
            term,
            depth,
            repeat,
            stay,
        } = action.kind()
        else {
            unreachable!("validated reduction");
        };
        let invalid = ParseError::InvalidTransition {
            offset: self.pos,
            state: self.state(),
        };
        let named = usize::from(term) < parser.names.len();
        let update = parser
            .context_tracker
            .as_ref()
            .and_then(|t| t.reduce.as_deref());
        if depth == 0 {
            let target = parser
                .reduction_goto(self.state(), term, self.pos, work, tokens)?
                .ok_or(invalid)?;
            if named {
                self.reduce_pos = self.pos;
            }
            self.push(target, self.reduce_pos);
            if named {
                tree.leaf(&mut self.roots, term, self.pos..self.pos);
            }
            return self.update_context(parser, update, term, self.reduce_pos, work, tokens);
        }

        let (base, from) = reduction_base(self.frames.len(), depth, stay).ok_or(invalid)?;
        let Frame {
            start, output_base, ..
        } = self.frames[base];
        let target = if stay {
            None
        } else {
            Some(
                parser
                    .reduction_goto(self.frames[from].state, term, self.pos, work, tokens)?
                    .ok_or(invalid)?,
            )
        };
        // A stay reduction leaves the base frame unchanged.
        let changed = base + usize::from(stay);
        if let Some(journal) = &mut self.trial_frames
            && changed < journal.start
        {
            let frames = &self.frames[changed..journal.start];
            // Pay for saving and restoring before changing any original frame.
            work.spend(self.pos, 2 * frames.len())?;
            journal.saved.extend(frames.iter().rev().copied());
            journal.start = changed;
        }
        if let Some(target) = target {
            self.frames[base].state = target;
        }
        self.frames.truncate(base + 1);
        // Recovery keeps recorded repeat groups until it has placed all
        // error markers. Other non-node reductions leave the roots unchanged.
        if named || (repeat && tree.records_repeats()) {
            if named && start == self.reduce_pos {
                self.reduce_pos = self.pos;
            }
            let end = if state.is_skipped() {
                self.pos
            } else {
                self.reduce_pos
            };
            if end < start || output_base.len > self.roots.len {
                return Err(invalid);
            }
            tree.reduce(
                &mut self.roots,
                output_base,
                term,
                start..end,
                self.pos,
                work,
            )?;
        }
        self.update_context(parser, update, term, start, work, tokens)
    }

    fn update_context(
        &mut self,
        parser: &Parser,
        update: Option<&ContextUpdate>,
        term: u16,
        start: usize,
        work: &Work,
        tokens: &mut TokenCache<'_>,
    ) -> Result<(), ParseError> {
        if let Some(update) = update {
            let mut input = tokens.cursor(start, work);
            let next = update(term, &self.view(parser, work), &mut input)?;
            work.spend(self.pos, 0)?;
            if let Some(next) = next {
                self.context = Some(next);
            }
        }
        Ok(())
    }
}
