use std::{cmp::Reverse, collections::HashSet, ops::Range};

use super::{
    super::reduction_base, ActionKind, ActionLimit, Branch, CUT_DEPTH, GlrMode, MAX_BRANCHES,
    MAX_STACK_DEPTH, ParseError, Parser, Position, State, SyntaxTree, Token, TokenCache,
    TreeBuilder, Work,
};
use crate::language::lezer::{action, tree::Roots};

pub(super) const DISTANCE: usize = 5;
const CUT_TO: usize = 2000 + 1;
const REDUCE_COST: i64 = 100;
const DELETE_COST: i64 = 190;
const INSERT_COST: i64 = 200;
const MAX_INSERTS: usize = 4;
const MAX_FORCED_REDUCTIONS: usize = 10;
// Upstream counts words in its stack of triplets, excluding the start frame.
const DAMPEN_INSERT_DEPTH: usize = 40;
const MAX_INSERT_DEPTH: usize = 100;

impl Parser {
    /// Parse the complete input with bounded error recovery.
    ///
    /// Uses the GLR engine and places error nodes in the result. A missing
    /// shift or forced reduction can add a zero-width marker; deletion marks
    /// the range of a token or one unrecognized Unicode scalar. A supplementary
    /// scalar uses two recovery steps, but the parser never scans inside it.
    /// A deleted named token is a child of its marker. Consecutive markers in a frame
    /// can coalesce. Recorded repeat groups and long tokenizer lookahead
    /// can separate error markers. Neither appears in the final tree.
    /// Insertion runs the shift hook for the missing term. Deletion does not
    /// run a hook. A forced reduction runs the ordinary reduce hook.
    ///
    /// The search is based on `@lezer/lr` 1.4.13, not a full port. It tries up
    /// to four insertions and ten forced reductions per stopped branch, then
    /// uses score-based pruning for a window of five advances of the
    /// reduction position. Insertions use actual shift or goto entries in
    /// the regular action list, not skip actions. Reductions use exact
    /// gotos. At 2800 pushed frames, if default reductions have not reduced
    /// the depth, the parser tries forced reductions to cut the stack to
    /// 2000 pushed frames. There is no backup reduction search. The parser
    /// uses Rust GLR stack equality, not buffer sizes.
    ///
    /// At EOF, it tries forced reductions but not insertions. If the winning
    /// trial spent all ten reductions and its stack shrank, only that branch
    /// can run more forced reductions. Each further batch of ten must shrink
    /// the stack to allow another batch. If the branch cannot finish, it
    /// returns partial output under the normal top node and adds an EOF marker
    /// unless the active frame's last output record is an error. The root spans
    /// the complete input.
    ///
    /// Hook errors and invalid ordinary transitions are not repaired. All
    /// branches share a work budget. Forks inherit the progress budget and
    /// only input consumption resets it. The parser limits the GLR frontier
    /// to 32 branches and each stack to 2800 frames, with one extra frame
    /// only for a consuming shift with an applicable positive-depth reduction.
    /// It limits trial actions to 32 even though it follows only the first one.
    /// Exceeding these limits returns [`ParseError::LimitExceeded`]. This mode
    /// neither uses nor creates incremental checkpoints.
    pub fn parse_recovering(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.run_glr(source, GlrMode::Recovering)
    }
}

pub(super) struct Stopped {
    pub(super) branch: Branch,
    pub(super) token: Option<Token>,
}

enum Trial {
    Advanced,
    Stopped,
    Exhausted,
}

pub(super) struct Recovery<'a, 'input> {
    pub(super) parser: &'a Parser,
    pub(super) work: &'a Work,
    pub(super) tree: &'a mut TreeBuilder,
    pub(super) tokens: &'a mut TokenCache<'input>,
}

impl Recovery<'_, '_> {
    pub(super) fn run(
        &mut self,
        position: usize,
        stopped: Vec<Stopped>,
        advanced: &mut Vec<Branch>,
    ) -> Result<Option<Roots>, ParseError> {
        let mut restarted = false;
        let mut finished: Option<(Branch, bool)> = None;
        for Stopped { mut branch, token } in stopped {
            if branch.delete_tail {
                // The marker already spans the scalar. Give this branch the
                // second delete cost and prune pass without running LR actions.
                self.work.spend(position, 1)?;
                branch.idle.spend(position, 1)?;
                branch.delete_tail = false;
                branch.score = branch.score.saturating_sub(DELETE_COST);
                self.keep(branch, advanced)?;
                continue;
            }
            if self.dead_end(&branch) {
                if restarted && position < self.tokens.source().len() {
                    continue;
                }
                if !restarted {
                    restarted = true;
                    self.mark(&mut branch, position..position, None);
                    branch.stack.frames.pop();
                    if self.advance(&mut branch)? {
                        self.keep(branch, advanced)?;
                        continue;
                    }
                }
            }
            if position == self.tokens.source().len() {
                let depth = branch.stack.frames.len();
                let trial = self.trial(&mut branch)?;
                if finished
                    .as_ref()
                    .is_none_or(|(best, _)| best.score < branch.score)
                {
                    let unwind =
                        matches!(trial, Trial::Exhausted) && branch.stack.frames.len() < depth;
                    finished = Some((branch, unwind));
                }
                continue;
            }
            if self.state(&branch).forced_reduce.is_some() {
                let mut fork = branch.fork(self.work)?;
                if matches!(self.trial(&mut fork)?, Trial::Advanced) {
                    self.keep(fork, advanced)?;
                }
            }
            let lookahead = token.map_or(0, |token| token.term);
            for (term, target) in self.insertions(&branch, lookahead)? {
                let mut insert = branch.fork(self.work)?;
                insert.idle.spend(position, 1)?;
                insert.stack.push(target, position);
                self.mark(&mut insert, position..position, None);
                insert.stack.update_context(
                    self.parser,
                    self.parser
                        .context_tracker
                        .as_ref()
                        .and_then(|tracker| tracker.shift.as_deref()),
                    term,
                    position,
                    self.work,
                    self.tokens,
                )?;
                insert.stack.reduce_pos = position;
                insert.score = insert.score.saturating_sub(INSERT_COST);
                if self.advance(&mut insert)? {
                    self.keep(insert, advanced)?;
                }
            }
            let (end, deleted) = match token.filter(|token| token.end > position) {
                Some(token) => (
                    token.end,
                    (usize::from(token.term) < self.parser.names.len()).then_some(token.term),
                ),
                None => {
                    let scalar = self.tokens.source()[position..].chars().next().unwrap();
                    branch.delete_tail = scalar.len_utf16() == 2;
                    (position + scalar.len_utf8(), None)
                }
            };
            self.mark(&mut branch, position..end, deleted);
            branch.stack.pos = end;
            branch.stack.reduce_pos = end;
            branch.idle = self.parser.progress_budget(branch.stack.frames.len());
            branch.score = branch.score.saturating_sub(DELETE_COST);
            self.keep(branch, advanced)?;
        }
        let Some((mut branch, unwind)) = finished else {
            return Ok(None);
        };
        if unwind {
            self.finish_eof(&mut branch)?;
        }
        if !self.state(&branch).is_accepting() {
            self.mark(&mut branch, position..position, None);
        }
        Ok(Some(branch.stack.roots))
    }

    fn state(&self, branch: &Branch) -> State {
        self.parser
            .tables
            .state(branch.stack.state())
            .expect("validated state")
    }

    fn dead_end(&self, branch: &Branch) -> bool {
        let state = self.state(branch);
        branch.stack.frames.len() == 2
            && state.default_reduce.is_none()
            && self.parser.tables.state_data[state.actions_offset as usize] == u16::MAX
    }

    fn mark(&mut self, branch: &mut Branch, range: Range<usize>, deleted: Option<u16>) {
        let stack = &mut branch.stack;
        let frame_base = stack.frames.last().expect("start frame").output_base.len;
        self.tree
            .error(&mut stack.roots, frame_base, range, deleted);
    }

    fn insertions(&self, branch: &Branch, lookahead: u16) -> Result<Vec<(u16, u16)>, ParseError> {
        let depth = branch.stack.frames.len() - 1;
        if depth >= MAX_INSERT_DEPTH {
            return Ok(Vec::new());
        }
        let tables = &self.parser.tables;
        let position = branch.stack.pos;
        let mut seen = HashSet::new();
        let mut next = Vec::new();
        for (term, action) in action::entries(
            &tables.state_data,
            self.state(branch).actions_offset as usize,
        ) {
            self.work.spend(position, 1)?;
            let Some(term) = term.filter(|&term| self.parser.is_token_term(term)) else {
                continue;
            };
            let (ActionKind::Shift(target) | ActionKind::Goto(target)) = action.kind() else {
                continue;
            };
            if seen.insert(target) {
                next.push((term, target));
            }
        }
        let many = next.len() > MAX_INSERTS;
        next.retain(|&(_, target)| target != branch.stack.state());
        if !many && depth < DAMPEN_INSERT_DEPTH {
            return Ok(next);
        }
        let mut best = Vec::new();
        for &(term, target) in &next {
            self.work.spend(position, 1)?;
            let state = tables.state(target).expect("validated shift target");
            if tables
                .probe_action(state, lookahead, || self.work.spend(position, 1))?
                .is_some()
            {
                best.push((term, target));
                if best.len() == MAX_INSERTS {
                    return Ok(best);
                }
            }
        }
        if depth < DAMPEN_INSERT_DEPTH {
            for entry in next {
                if !best.contains(&entry) {
                    best.push(entry);
                    if best.len() == MAX_INSERTS {
                        break;
                    }
                }
            }
        }
        Ok(best)
    }

    // A trial takes the first ordinary action until it consumes input or stalls.
    fn advance(&mut self, branch: &mut Branch) -> Result<bool, ParseError> {
        let position = branch.stack.pos;
        let mut actions = Vec::new();
        loop {
            self.work.spend(position, 1)?;
            branch.idle.spend(position, 1)?;
            let state = self.state(branch);
            let candidate = if let Some(action) = state.default_reduce {
                (
                    action,
                    Token {
                        term: self.parser.eof,
                        end: position,
                    },
                )
            } else {
                let scan = self.tokens.actions(
                    &branch.stack.view(self.parser, self.work),
                    state,
                    ActionLimit::Split(MAX_BRANCHES),
                    &mut actions,
                )?;
                branch.observe_lookahead(scan.lookahead, self.work, self.tree)?;
                let Some(&first) = actions.first() else {
                    return Ok(false);
                };
                first
            };
            branch
                .stack
                .check_action_depth(self.parser, candidate, self.work)?;
            branch.apply(
                self.parser,
                state,
                candidate,
                self.work,
                self.tree,
                self.tokens,
            )?;
            if branch.stack.pos > position {
                return Ok(true);
            }
        }
    }

    pub(super) fn cut_stack(&mut self, branch: &mut Branch) -> Result<State, ParseError> {
        debug_assert_eq!(branch.stack.frames.len(), CUT_DEPTH);
        while branch.stack.frames.len() > CUT_TO && self.force_reduce(branch, true)? {}
        if branch.stack.frames.len() > MAX_STACK_DEPTH {
            return Err(ParseError::LimitExceeded {
                offset: branch.stack.pos,
            });
        }
        Ok(self.state(branch))
    }

    fn trial(&mut self, branch: &mut Branch) -> Result<Trial, ParseError> {
        for _ in 0..MAX_FORCED_REDUCTIONS {
            if !self.force_reduce(branch, false)? {
                return Ok(Trial::Stopped);
            }
            if self.advance(branch)? {
                return Ok(Trial::Advanced);
            }
        }
        Ok(Trial::Exhausted)
    }

    // Close a staged branch without scanning at the cutoff, treating it as
    // EOF, or running deletion/insertion trials there. Work and the branch's
    // idle budget also bound a forced reduction cycle.
    pub(super) fn close_at_cutoff(&mut self, branch: &mut Branch) -> Result<(), ParseError> {
        while !self.state(branch).is_accepting() {
            if !self.force_reduce(branch, false)? {
                let at = branch.stack.pos;
                self.mark(branch, at..at, None);
                break;
            }
        }
        Ok(())
    }

    // As in Lezer's forceAll, use only the forced slot after score selection.
    // Unlike forceAll, require net stack progress to repeat a batch.
    fn finish_eof(&mut self, branch: &mut Branch) -> Result<(), ParseError> {
        loop {
            let depth = branch.stack.frames.len();
            for _ in 0..MAX_FORCED_REDUCTIONS {
                if !self.force_reduce(branch, false)? {
                    return Ok(());
                }
            }
            if branch.stack.frames.len() >= depth {
                return Ok(());
            }
        }
    }

    fn force_reduce(&mut self, branch: &mut Branch, at_cut: bool) -> Result<bool, ParseError> {
        let position = branch.stack.pos;
        self.work.spend(position, 1)?;
        let state = self.state(branch);
        if !at_cut && position == self.tokens.source().len() && state.is_accepting() {
            return Ok(false);
        }
        let Some(action) = state.forced_reduce else {
            return Ok(false);
        };
        let ActionKind::Reduce {
            term, depth, stay, ..
        } = action.kind()
        else {
            unreachable!("validated reduction")
        };
        let Some((_, from)) = reduction_base(branch.stack.frames.len(), depth, stay) else {
            return Ok(false);
        };
        if !stay
            && self
                .parser
                .strict_goto(branch.stack.frames[from].state, term, position, self.work)?
                .is_none()
        {
            return Ok(false);
        }
        let mut ordinary = state.default_reduce == Some(action);
        if !ordinary {
            for (term, candidate) in action::entries(
                &self.parser.tables.state_data,
                state.actions_offset as usize,
            ) {
                self.work.spend(position, 1)?;
                if term.is_some() && action == candidate {
                    ordinary = true;
                    break;
                }
            }
        }
        branch.idle.spend(position, 1)?;
        branch.stack.check_depth(action)?;
        if !ordinary {
            self.mark(branch, position..position, None);
            branch.score = branch.score.saturating_sub(REDUCE_COST);
        }
        branch.stack.reduce_pos = position;
        branch.apply(
            self.parser,
            state,
            (
                action,
                Token {
                    term: self.parser.eof,
                    end: position,
                },
            ),
            self.work,
            self.tree,
            self.tokens,
        )?;
        Ok(true)
    }

    fn keep(&self, branch: Branch, advanced: &mut Vec<Branch>) -> Result<(), ParseError> {
        let position = branch.stack.pos;
        for other in advanced.iter_mut() {
            if branch.equivalent(other, position, self.work)? {
                if branch.score > other.score {
                    *other = branch;
                }
                return Ok(());
            }
        }
        if advanced.len() == MAX_BRANCHES {
            let (index, worst) = advanced
                .iter()
                .enumerate()
                .rev()
                .min_by_key(|(_, branch)| branch.score)
                .expect("nonempty frontier");
            if branch.score <= worst.score {
                return Ok(());
            }
            self.work.spend(position, advanced.len() - index)?;
            advanced.remove(index);
        }
        advanced.push(branch);
        Ok(())
    }
}

pub(super) fn prune(advanced: &mut Vec<Branch>, distance: usize, cursor: Position) -> usize {
    let keep = if distance == 1 { 1 } else { distance * 3 };
    if advanced.len() > keep {
        advanced.sort_by_key(|branch| Reverse(branch.score));
        advanced.truncate(keep);
    }
    distance
        - usize::from(
            advanced
                .iter()
                .any(|branch| branch.position(branch.stack.reduce_pos) > cursor),
        )
}
