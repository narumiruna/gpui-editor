use std::{borrow::Cow, cmp::Reverse, collections::VecDeque};

use super::{
    super::tokenize::{ActionLimit, Candidate, Token, TokenCache},
    Action, ActionKind, ParseError, Parser, Roots, Stack, State, SyntaxTree, TreeBuilder, Work,
    reduction_base,
};

mod recovery;
pub(in crate::language) mod session;
#[cfg(test)]
mod tests;

use recovery::{Recovery, Stopped};

const MAX_BRANCHES: usize = 32;
pub(super) const MAX_STACK_DEPTH: usize = 2800;
// Lezer counts pushed frames, not the start frame. Only an advancing shift
// can enter the extra frame. Default reductions and the cut run before the
// next token scan or fork.
const CUT_DEPTH: usize = MAX_STACK_DEPTH + 1;
// Lezer 1.4.13 retains at most 12 stacks after it advances a position.
const EDITOR_FRONTIER: usize = 12;
// The pinned C++ corpus peaks at 36 live branches before pruning. Allow
// one more frontier of transient branches, but still bound every split.
const EDITOR_MAX_BRANCHES: usize = 4 * EDITOR_FRONTIER;
// The nested C++ template case uses 123k units in 108 bytes. This budget
// gives it 138k units, including the fixed grammar allowance.
const EDITOR_WORK_PER_BYTE: usize = 768;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GlrMode {
    Strict,
    EditorStrict,
    Pruned,
    Recovering,
    WastRecovering,
    JsonRecovering,
}

impl Parser {
    /// Parse the complete input, splitting the stack on distinct LR actions.
    ///
    /// This uses the same tables, hooks, strict gotos, and byte ranges as
    /// [`Self::parse`]. A branch with no action is discarded unless it accepts
    /// at end of input. Equivalent stacks at the same offset are merged.
    /// With a tracker, the contexts and frame starts must also match.
    /// The highest dynamic precedence score wins. When equal scores merge,
    /// prefer more records in the local branch, then the later branch. At end
    /// of input, the first accepting branch with the highest score wins.
    /// This returns one tree, not a forest, and does not use Lezer's packed
    /// buffer sizes to break ties.
    ///
    /// There is no recovery. If no branch accepts, report the furthest token
    /// failure (the first on a tie). Invalid transitions and hook errors abort
    /// the parse. Exceeding the shared work limit, a branch's progress limit,
    /// or 32 active branches returns [`ParseError::LimitExceeded`].
    pub fn parse_glr(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.run_glr(source, GlrMode::Strict)
    }

    // The pinned strict LR parser applies the stack cut before scanning the
    // next token. A stalled parse does not try error recovery.
    pub(in crate::language) fn parse_editor_strict(
        &self,
        source: &str,
    ) -> Result<SyntaxTree, ParseError> {
        self.run_glr(source, GlrMode::EditorStrict)
    }

    /// Use the pinned editor's score-based frontier limit without recovery.
    /// Keep the native stack equality and tree selection rules.
    pub(crate) fn parse_glr_pruned(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.run_glr(source, GlrMode::Pruned)
    }

    pub(super) fn run_glr(&self, source: &str, mode: GlrMode) -> Result<SyntaxTree, ParseError> {
        let mut parse = Glr::new(self, Cow::Borrowed(source), mode);
        loop {
            if let Some(roots) = parse.round()? {
                return parse.finish(roots).map(|(tree, _)| tree);
            }
        }
    }
}

// The input and parser stay fixed. All mutable LR state survives a round.
struct Glr<'parser, 'input> {
    parser: &'parser Parser,
    mode: GlrMode,
    recover: bool,
    max_branches: usize,
    work: Work,
    tokens: TokenCache<'input>,
    tree: TreeBuilder,
    actions: Vec<Candidate>,
    pending: VecDeque<Branch>,
    advanced: Vec<Branch>,
    split: bool,
    cursor: Position,
    failure: Option<(usize, ParseError)>,
    recovery_distance: usize,
}

impl<'parser, 'input> Glr<'parser, 'input> {
    fn new(parser: &'parser Parser, source: Cow<'input, str>, mode: GlrMode) -> Self {
        let recover = matches!(
            mode,
            GlrMode::Recovering | GlrMode::WastRecovering | GlrMode::JsonRecovering
        );
        let (max_branches, work_per_byte) = if mode == GlrMode::Pruned {
            (EDITOR_MAX_BRANCHES, EDITOR_WORK_PER_BYTE)
        } else {
            (MAX_BRANCHES, 512)
        };
        let work = parser.work_budget(&source, work_per_byte);
        let mut tokens = TokenCache::new(source, parser, recover);
        if matches!(mode, GlrMode::Strict | GlrMode::EditorStrict) {
            tokens.reduction_gotos = Some(Box::default());
        }
        Self {
            parser,
            mode,
            recover,
            max_branches,
            work,
            tokens,
            tree: match mode {
                // WAST has one anonymous repeat term, just before EOF.
                GlrMode::WastRecovering => TreeBuilder::for_wast_recovery(parser.eof - 1),
                GlrMode::JsonRecovering => TreeBuilder::for_glr(true, parser.names.len()),
                _ => TreeBuilder::for_glr(recover, 0),
            },
            actions: Vec::new(),
            pending: VecDeque::from([Branch {
                stack: Stack::new(parser),
                idle: parser.progress_budget(1),
                score: 0,
                records: 0,
                lookahead: 0,
                delete_tail: false,
            }]),
            advanced: Vec::new(),
            split: false,
            cursor: Position {
                byte: 0,
                at_boundary: true,
            },
            failure: None,
            recovery_distance: 0,
        }
    }

    // Consume one position round, including reductions, forks, recovery and
    // pruning. A round can scan a whole token or process the virtual second
    // UTF-16 deletion step at the same byte offset.
    fn round(&mut self) -> Result<Option<Roots>, ParseError> {
        let (parser, mode, recover, max_branches) =
            (self.parser, self.mode, self.recover, self.max_branches);
        let cut_depth = recover || mode == GlrMode::EditorStrict;
        let Self {
            work,
            tokens,
            tree,
            actions,
            pending,
            advanced,
            split,
            cursor,
            failure,
            recovery_distance,
            ..
        } = self;
        let work = &*work;
        let position = cursor.byte;
        let mut stopped = Vec::new();
        let mut accepted: Option<(i64, _)> = None;
        'branches: while let Some(mut branch) = pending.pop_front() {
            if branch.position(branch.stack.pos) != *cursor {
                advanced.push(branch);
                continue;
            }
            if branch.delete_tail {
                stopped.push(Stopped {
                    branch,
                    token: None,
                });
                continue;
            }
            // Process all reductions (and zero-width shifts) at the earliest
            // position before moving on. Other branches may be further ahead.
            while branch.stack.pos == position {
                work.spend(position, 1)?;
                branch.idle.spend(position, 1)?;
                let state = parser
                    .tables
                    .state(branch.stack.state())
                    .expect("validated state");
                if let Some(action) = state.default_reduce {
                    if cut_depth {
                        branch.stack.check_depth(action)?;
                    }
                    branch.apply(
                        parser,
                        state,
                        (
                            action,
                            Token {
                                term: parser.eof,
                                end: position,
                            },
                        ),
                        work,
                        tree,
                        tokens,
                    )?;
                    continue;
                }
                let state = if cut_depth && branch.stack.frames.len() > MAX_STACK_DEPTH {
                    *split = true;
                    Recovery {
                        parser,
                        work,
                        tree,
                        tokens,
                    }
                    .cut_stack(&mut branch)?
                } else {
                    state
                };
                let available = max_branches - pending.len() - advanced.len() - stopped.len();
                let scan = tokens.actions(
                    &branch.stack.view(parser, work),
                    state,
                    ActionLimit::Split(available),
                    actions,
                )?;
                branch.observe_lookahead(scan.lookahead, work, tree)?;
                let Some((last, alternatives)) = actions.split_last() else {
                    if position == tokens.source().len() && state.is_accepting() {
                        if accepted.is_none_or(|(score, _)| branch.score > score) {
                            accepted = Some((branch.score, branch.stack.roots));
                        }
                    } else if recover {
                        stopped.push(Stopped {
                            branch,
                            token: scan.main,
                        });
                        continue 'branches;
                    } else if failure.is_none_or(|(offset, _)| position > offset) {
                        *failure = Some((
                            position,
                            branch
                                .stack
                                .unexpected_token(scan.main.map(|token| token.term)),
                        ));
                    }
                    break;
                };
                // As in Lezer, run the last action on the original stack.
                for &candidate in alternatives {
                    *split = true;
                    if cut_depth {
                        branch.stack.check_action_depth(parser, candidate, work)?;
                    }
                    let mut fork = branch.fork(work)?;
                    fork.apply(parser, state, candidate, work, tree, tokens)?;
                    if fork.stack.pos > position {
                        advanced.push(fork);
                    } else {
                        pending.push_back(fork);
                    }
                }
                if cut_depth {
                    branch.stack.check_action_depth(parser, *last, work)?;
                }
                branch.apply(parser, state, *last, work, tree, tokens)?;
            }
            if branch.stack.pos > position {
                advanced.push(branch);
            }
        }
        let mut finished = accepted.map(|(_, roots)| roots);
        if recover && finished.is_none() && !stopped.is_empty() {
            if advanced.is_empty() && *recovery_distance == 0 {
                *recovery_distance = recovery::DISTANCE;
            }
            if *recovery_distance > 0 {
                *split = true;
                finished = Recovery {
                    parser,
                    work,
                    tree,
                    tokens,
                }
                .run(position, stopped, advanced)?;
            }
        }
        if finished.is_some() {
            return Ok(finished);
        }
        if advanced.is_empty() {
            return Err(failure.expect("at least one stopped branch").1);
        }
        if *recovery_distance == 0 {
            merge_equivalent(advanced, position, work)?;
            if mode == GlrMode::Pruned && advanced.len() > EDITOR_FRONTIER {
                work.spend(position, advanced.len())?;
                // Stable sort keeps the earlier branch when scores tie.
                advanced.sort_by_key(|branch| Reverse(branch.score));
                advanced.truncate(EDITOR_FRONTIER);
            }
        } else {
            work.spend(position, advanced.len())?;
            *recovery_distance = recovery::prune(advanced, *recovery_distance, *cursor);
        }
        *cursor = advanced
            .iter()
            .map(|b| b.position(b.stack.pos))
            .min()
            .expect("live branch");
        pending.extend(advanced.drain(..));
        Ok(None)
    }

    fn finish(self, roots: Roots) -> Result<(SyntaxTree, usize), ParseError> {
        let end = self.cursor.byte;
        let before = self.work.remaining();
        if self.recover {
            self.work.spend(end, roots.len)?;
        }
        let tree = if self.split || self.recover {
            self.tree.finish_glr(self.parser, roots, end, &self.work)?
        } else {
            self.tree.finish(self.parser, roots, end, None)
        };
        Ok((tree, before - self.work.remaining()))
    }
}

impl Stack {
    pub(super) fn check_depth(&self, action: Action) -> Result<(), ParseError> {
        if self.frames.len() >= MAX_STACK_DEPTH
            && matches!(
                action.kind(),
                ActionKind::Shift(_) | ActionKind::Goto(_) | ActionKind::Reduce { depth: 0, .. }
            )
        {
            return Err(ParseError::LimitExceeded { offset: self.pos });
        }
        Ok(())
    }

    pub(super) fn check_action_depth(
        &self,
        parser: &Parser,
        (action, token): Candidate,
        work: &Work,
    ) -> Result<(), ParseError> {
        if self.frames.len() == MAX_STACK_DEPTH
            && token.end > self.pos
            && let ActionKind::Shift(target) = action.kind()
        {
            let state = parser.tables.state(target).expect("validated shift target");
            // Default reductions take priority over the forced slot. At positive
            // depth the goto source is already in the stack, before the new frame.
            if let Some(reduce) = state.default_reduce.or(state.forced_reduce)
                && let ActionKind::Reduce {
                    term, depth, stay, ..
                } = reduce.kind()
                && depth > 0
                && let Some((_, from)) = reduction_base(CUT_DEPTH, depth, stay)
                && (stay
                    || parser
                        .strict_goto(self.frames[from].state, term, self.pos, work)?
                        .is_some())
            {
                return Ok(());
            }
        }
        self.check_depth(action)
    }
}

/// A pending surrogate deletion sorts before its scalar's end boundary.
/// Both positions carry the same valid byte offset.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Position {
    byte: usize,
    at_boundary: bool,
}

struct Branch {
    stack: Stack,
    idle: Work,
    score: i64,
    records: usize,
    lookahead: usize,
    delete_tail: bool,
}

impl Branch {
    fn position(&self, byte: usize) -> Position {
        Position {
            byte,
            at_boundary: !self.delete_tail,
        }
    }

    fn observe_lookahead(
        &mut self,
        end: usize,
        work: &Work,
        tree: &mut TreeBuilder,
    ) -> Result<(), ParseError> {
        // Like setLookAhead, emit a record only when this branch reaches
        // farther. It separates errors without affecting LR state or score.
        if end > self.lookahead {
            let pos = self.stack.pos;
            work.spend(pos, 1)?;
            tree.lookahead_barrier(&mut self.stack.roots, pos);
            self.lookahead = end;
        }
        Ok(())
    }

    fn fork(&self, work: &Work) -> Result<Self, ParseError> {
        work.spend(self.stack.pos, self.stack.frames.len())?;
        Ok(Self {
            stack: self.stack.clone(),
            idle: self.idle.clone(),
            score: self.score,
            records: 0,
            lookahead: self.lookahead,
            delete_tail: self.delete_tail,
        })
    }

    fn apply(
        &mut self,
        parser: &Parser,
        state: State,
        candidate: Candidate,
        work: &Work,
        tree: &mut TreeBuilder,
        tokens: &mut TokenCache<'_>,
    ) -> Result<(), ParseError> {
        let start = self.stack.pos;
        let (action, token) = candidate;
        let recorded = match action.kind() {
            ActionKind::Reduce { term, repeat, .. } => {
                let score = parser
                    .dynamic_precedence
                    .get(usize::from(term))
                    .copied()
                    .flatten()
                    .unwrap_or(0);
                self.score = self.score.saturating_add(i64::from(score));
                repeat || usize::from(term) < parser.names.len()
            }
            ActionKind::Shift(_) | ActionKind::Skip => usize::from(token.term) < parser.names.len(),
            ActionKind::Goto(_) => false,
        };
        self.records += usize::from(recorded);
        self.stack
            .apply(parser, state, candidate, work, tree, tokens)?;
        if self.stack.pos > start {
            self.idle = parser.progress_budget(self.stack.frames.len());
        }
        Ok(())
    }

    fn equivalent(&self, other: &Self, position: usize, work: &Work) -> Result<bool, ParseError> {
        work.spend(position, 1)?;
        let (a, b) = (&self.stack, &other.stack);
        if a.pos != b.pos
            || self.delete_tail != other.delete_tail
            || a.reduce_pos != b.reduce_pos
            || a.frames.len() != b.frames.len()
            || a.state() != b.state()
        {
            return Ok(false);
        }
        work.spend(position, a.frames.len())?;
        // Reduce hooks can read input at a frame's start. Old tree nodes do
        // not affect future actions. Keep the winner's ranges and roots intact.
        let compare_starts = a.context.is_some();
        Ok(a.frames
            .iter()
            .zip(&b.frames)
            .all(|(a, b)| a.state == b.state && (!compare_starts || a.start == b.start))
            && a.context == b.context)
    }
}

fn merge_equivalent(
    branches: &mut Vec<Branch>,
    position: usize,
    work: &Work,
) -> Result<(), ParseError> {
    let mut i = 0;
    'next: while i + 1 < branches.len() {
        let mut j = i + 1;
        while j < branches.len() {
            let (a, b) = (&branches[i], &branches[j]);
            if a.equivalent(b, position, work)? {
                let loser = if (a.score, a.records) > (b.score, b.records) {
                    j
                } else {
                    i
                };
                work.spend(position, branches.len() - loser)?;
                branches.remove(loser);
                if loser == i {
                    continue 'next;
                }
            } else {
                j += 1;
            }
        }
        i += 1;
    }
    Ok(())
}
