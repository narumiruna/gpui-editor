use std::{fmt, ops::Range, sync::Arc};

use super::{
    super::{Tokenizer, tokenize::ReadExtent, tree::OutputPrefix},
    ParseError, Parser, Stack, SyntaxTree, TreeBuilder, Work,
    glr::GlrMode,
};

mod suffix;

use suffix::Decision;
pub(super) use suffix::SuffixReuse;

const CHECKPOINT_INTERVAL: usize = 64;
const MAX_CHECKPOINT_DEPTH: usize = 64;

/// A candidate unchanged range of an earlier incremental tree.
///
/// Both ranges are in UTF-8 bytes. Pass a prefix starting at zero in both
/// documents, or a shifted suffix ending at EOF in both documents. The parser
/// checks the ranges and compares the actual bytes before reusing parse state.
/// See [`Parser::parse_incremental_fragments`] to pass both kinds at once.
#[derive(Clone, Debug)]
pub struct TreeFragment<'a> {
    pub tree: &'a SyntaxTree,
    pub old_range: Range<usize>,
    pub new_range: Range<usize>,
}

/// Reuse from a prefix checkpoint, not including shifted-suffix decisions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReuseStats {
    /// Byte offset at which LR parsing resumed; zero if it started fresh.
    pub resumed_at: usize,
    /// Number of immutable strict LR node records shared with the earlier
    /// tree's output or private prefix seed.
    pub nodes_reused: usize,
}

struct Checkpoint {
    stack: Stack,
    output: OutputPrefix,
    reads: ReadExtent,
    work_spent: usize,
}

pub(in super::super) struct CheckpointHistory {
    source: Box<str>,
    checkpoints: Vec<Arc<Checkpoint>>,
    decisions: Vec<Decision>,
    reused_suffix_actions: usize,
    initial_work: usize,
    stats: ReuseStats,
}

impl fmt::Debug for CheckpointHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckpointHistory")
            .field("source_len", &self.source.len())
            .field("checkpoints", &self.checkpoints.len())
            .field("suffix_decisions", &self.decisions.len())
            .field("reused_suffix_actions", &self.reused_suffix_actions)
            .field("stats", &self.stats)
            .finish()
    }
}

impl Parser {
    /// Parse with zero or one candidate fragment.
    ///
    /// Call with `None` to collect parse state for the first parse. See
    /// [`Self::parse_incremental_fragments`] for reuse rules.
    pub fn parse_incremental(
        &self,
        source: &str,
        fragment: Option<TreeFragment<'_>>,
    ) -> Result<SyntaxTree, ParseError> {
        self.parse_incremental_fragments(source, fragment.as_slice())
    }

    /// Parse with the same strict LR rules and limits as [`Self::parse`].
    ///
    /// Pass an empty slice to collect parse state for the first parse. After
    /// edits, pass candidate fragments in any order. One parse can combine:
    ///
    /// - The furthest valid prefix checkpoint, which restores the LR stack
    ///   and shares immutable output nodes. See [`SyntaxTree::reuse_stats`].
    /// - The longest valid shifted suffix with recorded decisions. The first
    ///   scan at a position can reuse a consuming token/action decision if the
    ///   LR state matches. The parser still runs LR transitions and builds
    ///   output nodes. The tree retains decisions only from the input it
    ///   traversed. See [`SyntaxTree::reused_suffix_actions`].
    ///
    /// The parser ignores invalid fragments, interior ranges, unshifted
    /// suffixes, and trees without incremental history or from another parser
    /// configuration. Separate parsers cannot share parse state, even if they
    /// use the same tables.
    /// Use [`ExternalTokenizer::new_deterministic`](super::super::ExternalTokenizer::new_deterministic)
    /// and [`Self::with_deterministic_context_tracker`] to opt in those hooks.
    /// Specialization callbacks disable reuse, but immutable text tables from
    /// [`Specializer::context_free`](super::super::Specializer::context_free)
    /// are eligible. Only built-in tokenizers with no context tracker are
    /// eligible for shifted suffix reuse. Prefix checkpoints are sparse and
    /// omit deep stacks. There is no reuse of GLR branches or recovered output.
    /// The tree owns a copy of the input when it retains parse state.
    pub fn parse_incremental_fragments(
        &self,
        source: &str,
        fragments: &[TreeFragment<'_>],
    ) -> Result<SyntaxTree, ParseError> {
        self.parse_incremental_inner(source, fragments, None)
    }

    /// Try strict incremental parsing, then run [`Self::parse_recovering`]
    /// from the start if strict parsing fails.
    ///
    /// Recovery uses its own limits and output. On success, the tree keeps
    /// only the completed strict prefix checkpoints as a private seed for
    /// later strict parses. The same fragment checks apply to this seed.
    /// The recovered tree reports no node or suffix reuse. Neither GLR
    /// branches nor recovered nodes can seed another parse.
    pub fn parse_incremental_recovering(
        &self,
        source: &str,
        fragments: &[TreeFragment<'_>],
    ) -> Result<SyntaxTree, ParseError> {
        self.parse_incremental_inner(source, fragments, Some(GlrMode::Recovering))
    }

    // Use the deterministic result only if the parse does not need a cut.
    pub(in crate::language) fn parse_incremental_wast_recovering(
        &self,
        source: &str,
        fragments: &[TreeFragment<'_>],
    ) -> Result<SyntaxTree, ParseError> {
        self.parse_incremental_inner(source, fragments, Some(GlrMode::WastRecovering))
    }

    // JSON recovery can wrap a long error list in each discarded trial.
    pub(crate) fn parse_incremental_json_recovering(
        &self,
        source: &str,
        fragments: &[TreeFragment<'_>],
    ) -> Result<SyntaxTree, ParseError> {
        self.parse_incremental_inner(source, fragments, Some(GlrMode::JsonRecovering))
    }

    fn parse_incremental_inner(
        &self,
        source: &str,
        fragments: &[TreeFragment<'_>],
        recovery: Option<GlrMode>,
    ) -> Result<SyntaxTree, ParseError> {
        let wast = recovery == Some(GlrMode::WastRecovering);
        if self
            .context_tracker
            .as_ref()
            .is_some_and(|t| !t.deterministic)
            || self.specializers.iter().any(|s| !s.is_context_free())
            || self
                .tokenizers
                .iter()
                .any(|t| matches!(t, Tokenizer::External(t) if !t.deterministic))
        {
            if wast {
                return self.run_glr(source, GlrMode::WastRecovering);
            }
            return self.parse(source).or_else(|error| match recovery {
                Some(mode) => self.run_glr(source, mode),
                None => Err(error),
            });
        }
        let work = self.work_budget(source, 128);
        let mut history = CheckpointHistory {
            source: source.into(),
            checkpoints: Vec::new(),
            decisions: Vec::new(),
            reused_suffix_actions: 0,
            initial_work: work.remaining(),
            stats: ReuseStats::default(),
        };
        let suffix = SuffixReuse::new(self, source, fragments);
        let (stack, mut tree) = history
            .resume(self, source, fragments, &work)
            .unwrap_or_else(|| (Stack::new(self), TreeBuilder::default()));
        let result = if wast || recovery == Some(GlrMode::JsonRecovering) {
            self.run_lr::<true>(source, stack, &mut tree, work, Some(&mut history), suffix)
        } else {
            self.run_lr::<false>(source, stack, &mut tree, work, Some(&mut history), suffix)
        };
        match result {
            Ok((roots, end)) => Ok(tree.finish(self, roots, end, Some(history))),
            Err(error) => {
                let Some(mode) = recovery else {
                    return Err(error);
                };
                // The LR loop commits suffix decisions only on acceptance.
                // Independent recovery publishes none of this strict output.
                // Drop the unfrozen records before starting recovery.
                history.stats = ReuseStats::default();
                let seed =
                    (!history.checkpoints.is_empty()).then(|| tree.into_prefix_seed(history));
                self.run_glr(source, mode)
                    .map(|tree| tree.with_prefix_seed(seed))
            }
        }
    }
}

impl SyntaxTree {
    /// Report prefix checkpoint reuse in the published tree. If strict
    /// parsing failed and recovery built this tree, both values are zero.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.checkpoint_history()
            .map_or_else(ReuseStats::default, |history| history.stats)
    }

    /// Number of token/action decisions reused from a shifted suffix.
    /// This does not count shared syntax nodes or skipped LR transitions.
    pub fn reused_suffix_actions(&self) -> usize {
        self.checkpoint_history()
            .map_or(0, |history| history.reused_suffix_actions)
    }
}

impl CheckpointHistory {
    fn resume(
        &mut self,
        parser: &Parser,
        source: &str,
        fragments: &[TreeFragment<'_>],
        work: &Work,
    ) -> Option<(Stack, TreeBuilder)> {
        let (tree, checkpoints, checkpoint) = fragments
            .iter()
            .filter_map(|fragment| {
                let checkpoints = prefix_checkpoints(parser, source, fragment, self.initial_work)?;
                Some((fragment.tree, checkpoints, checkpoints.last()?))
            })
            .max_by_key(|(_, _, checkpoint)| checkpoint.stack.pos)?;
        // Charge the work of the old prefix to the new budget. This preserves
        // the position and timing of work-limit errors in the remaining parse.
        work.spend(checkpoint.stack.pos, checkpoint.work_spent)
            .expect("checkpoint work fits in the new budget");
        let prefix = TreeBuilder::resume(tree, checkpoint.output);
        self.checkpoints.extend_from_slice(checkpoints);
        self.stats = ReuseStats {
            resumed_at: checkpoint.stack.pos,
            nodes_reused: prefix.node_count(),
        };
        Some((checkpoint.stack.clone(), prefix))
    }

    pub(super) fn last_reads(&self) -> ReadExtent {
        self.checkpoints
            .last()
            .map_or_else(ReadExtent::default, |checkpoint| checkpoint.reads)
    }

    pub(super) fn record(
        &mut self,
        stack: &Stack,
        tree: &mut TreeBuilder,
        mut reads: ReadExtent,
        work: &Work,
    ) {
        if stack.frames.len() > MAX_CHECKPOINT_DEPTH
            || self
                .checkpoints
                .last()
                .is_some_and(|last| stack.pos - last.stack.pos < CHECKPOINT_INTERVAL)
        {
            return;
        }
        // The caller only records after consuming input. Even lookahead from a
        // failed scan or a cached token is in the cumulative read extent. All
        // cache keys start before this position, so no cache slot can hit on
        // resume. The idle budget has just reset and the stack holds the exact
        // LR frames, output roots and context value.
        reads.end = reads.end.max(stack.pos);
        self.checkpoints.push(Arc::new(Checkpoint {
            stack: stack.clone(),
            output: tree.checkpoint(),
            reads,
            work_spent: self.initial_work - work.remaining(),
        }));
    }
}

fn prefix_checkpoints<'a>(
    parser: &Parser,
    source: &str,
    fragment: &TreeFragment<'a>,
    work_budget: usize,
) -> Option<&'a [Arc<Checkpoint>]> {
    let TreeFragment {
        tree,
        old_range,
        new_range,
    } = fragment;
    if old_range.start != 0
        || new_range.start != 0
        || old_range.end != new_range.end
        || !tree.belongs_to(parser)
    {
        return None;
    }
    let previous = tree.checkpoint_history()?;
    let end = old_range.end;
    if previous.source.get(..end)? != source.get(..end)? {
        return None;
    }
    let same_eof = end == source.len() && end == previous.source.len();
    // Read extent, EOF dependency and work spent all grow monotonically.
    let count = previous.checkpoints.partition_point(|checkpoint| {
        checkpoint.reads.end <= end
            && (!checkpoint.reads.eof || same_eof)
            && checkpoint.work_spent <= work_budget
    });
    Some(&previous.checkpoints[..count])
}
