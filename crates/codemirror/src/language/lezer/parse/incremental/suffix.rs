use std::cell::Cell;

use super::{
    super::super::{
        Action, ActionKind, ParseError, Parser, StackView, State, Tokenizer,
        tokenize::{ActionLimit, Candidate, ReadExtent, Token, TokenCache},
    },
    CheckpointHistory, TreeFragment,
};

#[derive(Clone, Copy)]
pub(super) struct Decision {
    start: usize,
    state: u16,
    action: Action,
    term: u16,
    token_len: usize,
    read_len: usize,
    read_eof: bool,
    work: usize,
}

struct ShiftedDecisions<'a> {
    old_start: usize,
    new_start: usize,
    decisions: &'a [Decision],
}

impl<'a> ShiftedDecisions<'a> {
    fn new(parser: &Parser, source: &str, fragment: &TreeFragment<'a>) -> Option<Self> {
        let TreeFragment {
            tree,
            old_range,
            new_range,
        } = fragment;
        let previous = tree.checkpoint_history()?;
        if previous.decisions.is_empty()
            || old_range.start == new_range.start
            || old_range.is_empty()
            || old_range.end != previous.source.len()
            || new_range.end != source.len()
            || !tree.belongs_to(parser)
            || previous.source.get(old_range.clone())? != source.get(new_range.clone())?
        {
            return None;
        }
        let first = previous
            .decisions
            .partition_point(|entry| entry.start < old_range.start);
        (first < previous.decisions.len()).then_some(Self {
            old_start: old_range.start,
            new_start: new_range.start,
            decisions: &previous.decisions[first..],
        })
    }

    fn at(&mut self, start: usize, state: u16) -> Option<Decision> {
        let old = self.old_start + start.checked_sub(self.new_start)?;
        // The parser moves forward, so no earlier entry can match later.
        while let Some((&entry, rest)) = self.decisions.split_first() {
            if entry.start >= old {
                return (entry.start == old && entry.state == state)
                    .then_some(Decision { start, ..entry });
            }
            self.decisions = rest;
        }
        None
    }
}

pub(in super::super) struct SuffixReuse<'a> {
    previous: Option<ShiftedDecisions<'a>>,
    decisions: Vec<Decision>,
    reused: usize,
    last_scan: Option<usize>,
}

impl<'a> SuffixReuse<'a> {
    pub(in super::super) fn new(
        parser: &Parser,
        source: &str,
        fragments: &[TreeFragment<'a>],
    ) -> Option<Self> {
        // The caller already excluded specialization callbacks. Even
        // deterministic tokenizers and trackers can use absolute positions,
        // read behind the cursor or depend on the full stack.
        if parser.context_tracker.is_some()
            || parser
                .tokenizers
                .iter()
                .any(|tokenizer| matches!(tokenizer, Tokenizer::External(_)))
        {
            return None;
        }
        Some(Self {
            previous: fragments
                .iter()
                .filter_map(|fragment| ShiftedDecisions::new(parser, source, fragment))
                .min_by_key(|suffix| suffix.new_start),
            decisions: Vec::new(),
            reused: 0,
            last_scan: None,
        })
    }

    pub(in super::super) fn actions(
        &mut self,
        tokens: &mut TokenCache<'_>,
        stack: &StackView<'_>,
        state: State,
        actions: &mut Vec<Candidate>,
    ) -> Result<Option<Token>, ParseError> {
        let start = stack.byte_offset();
        let scan = |tokens: &mut TokenCache<'_>, actions: &mut Vec<_>| {
            Ok(tokens
                .actions(stack, state, ActionLimit::Deterministic, actions)?
                .main)
        };
        // Only the first scan at a position has no live cache slots for that
        // position. Repeated scans may depend on which slots were filled.
        if self.last_scan.replace(start) == Some(start) {
            return scan(tokens, actions);
        }
        fn reads<'a>(tokens: &'a TokenCache<'_>) -> &'a Cell<ReadExtent> {
            tokens.reads.as_ref().expect("incremental reads")
        }
        if let Some(entry) = self
            .previous
            .as_mut()
            .and_then(|p| p.at(start, stack.state()))
        {
            // Matching state gives the same tokenizer mask and action lists.
            // Identical bytes through EOF cover all probes, including losing
            // and failed probes and text specialization. Only this scan is
            // skipped; all its work was charged at the current byte offset.
            stack.work.spend(start, entry.work)?;
            ReadExtent::observe(Some(reads(tokens)), start + entry.read_len, entry.read_eof);
            actions.clear();
            actions.push((
                entry.action,
                Token {
                    term: entry.term,
                    end: start + entry.token_len,
                },
            ));
            self.decisions.push(entry);
            self.reused += 1;
            // The strict LR loop only uses the main token on an empty action list.
            return Ok(None);
        }
        let before = stack.work.remaining();
        let prior_reads = reads(tokens).take();
        let main = scan(tokens, actions)?;
        let scan_reads = reads(tokens).get();
        ReadExtent::observe(Some(reads(tokens)), prior_reads.end, prior_reads.eof);
        if let Some(&(action, token)) = actions.first()
            && token.end > start
            && matches!(action.kind(), ActionKind::Shift(_) | ActionKind::Skip)
        {
            // The action will advance the input. Its cache slots would then
            // be stale, so the next scan need not restore them on replay.
            self.decisions.push(Decision {
                start,
                state: stack.state(),
                action,
                term: token.term,
                token_len: token.end - start,
                read_len: scan_reads.end.max(token.end) - start,
                read_eof: scan_reads.eof,
                work: before - stack.work.remaining(),
            });
        }
        Ok(main)
    }

    pub(in super::super) fn finish(self, history: &mut CheckpointHistory) {
        history.decisions = self.decisions;
        history.reused_suffix_actions = self.reused;
    }
}
