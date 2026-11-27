//! Caller-driven parsing with byte-based tree queries and explicit wakeups.
//!
//! The service publishes trees immediately, without an editor state update.
//! Both kinds of skip block prefix availability. A job controls how it builds
//! a tree, including any mounted children. There are no Lezer tree fragments,
//! mixed-parse wrappers, or view workers. A step is not a time limit.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

/// One parse job. The service keeps it between work calls.
pub trait ParseJob {
    type Tree;
    type Error;

    /// Advance by one step. Report pending work or publish an owned tree.
    /// The service rejects published extents and skips outside the documented
    /// bounds with a panic. A job error restarts the job but keeps the last tree.
    fn advance(
        &mut self,
        request: ParseRequest<'_, Self::Tree>,
    ) -> Result<ParseStep<Self::Tree>, Self::Error>;

    /// Discard in-flight state, but keep parser configuration. The next step
    /// can use the previous snapshot and its dirty ranges to reuse output.
    fn restart(&mut self);
}

/// Input for one step. All input positions use UTF-8 bytes.
pub struct ParseRequest<'a, T> {
    pub text: &'a Text,
    /// Requested end of the parse. A job can publish less or parse farther.
    pub upto: usize,
    pub viewport: Range<usize>,
    /// Most recent snapshot, even if it is from an earlier text version.
    /// Check dirty ranges before reusing it. Old tree positions are unchanged.
    pub previous: Option<&'a ParseSnapshot<T>>,
    /// Edits and regions woken since `previous`, in current text coordinates.
    /// These are invalidation hints, not offsets for mapping old tree nodes.
    pub dirty_ranges: &'a [Range<usize>],
    /// Reported skips that have not been woken, in current text coordinates.
    pub skipped_ranges: &'a [ParseSkip],
}

/// Result of one step.
pub enum ParseStep<T> {
    Pending,
    Publish {
        tree: T,
        /// The end of the prefix this tree describes, including reported skips.
        parsed_to: usize,
        /// All skipped regions in that prefix, not just new skips this step.
        skipped: Vec<ParseSkip>,
    },
}

impl<T> ParseStep<T> {
    /// Publish a tree with no skips.
    pub fn publish(tree: T, parsed_to: usize) -> Self {
        Self::Publish {
            tree,
            parsed_to,
            skipped: Vec::new(),
        }
    }
}

/// Work omitted from a snapshot. Each range must be nonempty and use UTF-8
/// boundaries within the snapshot's prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseSkip {
    /// Retry when the range overlaps the visible range. Only report this for
    /// regions outside the step's viewport.
    UntilVisible(Range<usize>),
    /// Retry when the owner calls [`ParseService::resume_deferred`].
    Deferred(Range<usize>),
}

impl ParseSkip {
    pub fn range(&self) -> &Range<usize> {
        match self {
            Self::UntilVisible(range) | Self::Deferred(range) => range,
        }
    }

    fn map(self, changes: &ChangeSet) -> Option<Self> {
        let range = map_range(changes, self.range(), Assoc::After, Assoc::Before);
        (range.start < range.end).then_some(match self {
            Self::UntilVisible(_) => Self::UntilVisible(range),
            Self::Deferred(_) => Self::Deferred(range),
        })
    }
}

/// An owned tree published by a job. Check the service's availability before
/// relying on coverage; a snapshot can contain placeholder trees.
#[derive(Debug)]
pub struct ParseSnapshot<T> {
    tree: T,
    parsed_to: usize,
}

impl<T> ParseSnapshot<T> {
    pub fn tree(&self) -> &T {
        &self.tree
    }

    pub fn parsed_to(&self) -> usize {
        self.parsed_to
    }
}

/// A native parse scheduler for one document. Callers decide when to run it.
pub struct ParseService<J: ParseJob> {
    text: Text,
    job: J,
    viewport: Range<usize>,
    snapshot: Option<ParseSnapshot<J::Tree>>,
    stale: bool,
    dirty: Vec<Range<usize>>,
    skipped: Vec<ParseSkip>,
}

impl<J: ParseJob> ParseService<J> {
    /// Start with the entire text as the viewport. Does not run the job.
    pub fn new(text: Text, job: J) -> Self {
        let viewport = 0..text.len();
        Self {
            text,
            job,
            viewport,
            snapshot: None,
            stale: false,
            dirty: Vec::new(),
            skipped: Vec::new(),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn viewport(&self) -> Range<usize> {
        self.viewport.clone()
    }

    /// Set the visible range and mark any overlapping viewport skips dirty.
    /// Returns whether skips were woken. Does not advance the job.
    pub fn set_viewport(&mut self, viewport: Range<usize>) -> Result<bool, EditError> {
        if !self.text.is_valid_range(viewport.clone()) {
            return Err(EditError::InvalidRange);
        }
        let woke = self.wake_visible(&viewport);
        self.viewport = viewport;
        Ok(woke)
    }

    /// The latest snapshot for the current text, even if incomplete. Returns
    /// `None` before the first publication and after an edit until publication.
    pub fn syntax_tree(&self) -> Option<&ParseSnapshot<J::Tree>> {
        self.snapshot.as_ref().filter(|_| !self.stale)
    }

    /// Test whether one published snapshot covers `0..upto` without any
    /// skipped or dirty bytes. Positions past the end are clamped to EOF.
    pub fn available(&self, upto: usize) -> bool {
        let upto = upto.min(self.text.len());
        self.covers(upto) && self.skipped.iter().all(|skip| skip.range().start >= upto)
    }

    /// Try to make a tree available through `upto` in at most `steps` calls to
    /// `advance`. Treat `0..upto` as visible while parsing, then use the saved
    /// viewport to wake any new skips that are visible there. Only return a
    /// tree if the requested prefix is ready. A zero budget is read-only.
    /// A position inside a character rounds up.
    pub fn ensure(
        &mut self,
        upto: usize,
        steps: usize,
    ) -> Result<Option<&ParseSnapshot<J::Tree>>, J::Error> {
        let mut upto = upto.min(self.text.len());
        while !self.text.is_char_boundary(upto) {
            upto += 1;
        }
        if steps != 0 && !self.available(upto) {
            let viewport = 0..upto;
            self.wake_visible(&viewport);
            let result = self.run_to(viewport.clone(), steps);
            if viewport != self.viewport {
                self.wake_visible(&self.viewport.clone());
            }
            result?;
        }
        Ok(self.syntax_tree().filter(|_| self.available(upto)))
    }

    /// Spend at most `steps` calls to the job towards the saved viewport end.
    /// A skip does not trigger another parse until it is woken.
    pub fn work(&mut self, steps: usize) -> Result<(), J::Error> {
        self.run_to(self.viewport.clone(), steps)
    }

    /// Pending invalidation hints in current text coordinates. Ranges are
    /// sorted and merged. A deletion can leave a zero-width anchor.
    pub fn dirty_ranges(&self) -> &[Range<usize>] {
        &self.dirty
    }

    /// Reported skips that have not been woken, in current text coordinates.
    pub fn skipped_ranges(&self) -> &[ParseSkip] {
        &self.skipped
    }

    /// Apply edits atomically, map the viewport and outstanding ranges, and
    /// restart the job. Keep the old tree only as a reuse hint for the job.
    pub fn apply_changes(&mut self, changes: &ChangeSet) -> Result<(), EditError> {
        if changes.is_empty() {
            return Ok(());
        }
        changes.apply_to(&mut self.text)?;
        self.viewport = map_range(changes, &self.viewport, Assoc::Before, Assoc::After);
        for range in &mut self.dirty {
            *range = map_range(changes, range, Assoc::Before, Assoc::After);
        }
        self.skipped = self
            .skipped
            .drain(..)
            .filter_map(|skip| skip.map(changes))
            .collect();
        let (mut old_end, mut new_end) = (0, 0);
        for (old, inserted) in changes.changes() {
            let start = new_end + (old.start - old_end);
            new_end = start + inserted.len();
            old_end = old.end;
            self.dirty.push(start..new_end);
        }
        merge_ranges(&mut self.dirty);
        self.stale = true;
        self.job.restart();
        Ok(())
    }

    /// Mark all deferred regions dirty and restart the job if any were woken.
    /// Call this after the deferred parsers are ready. The service does not
    /// poll futures or schedule callbacks.
    pub fn resume_deferred(&mut self) -> bool {
        self.wake(|skip| matches!(skip, ParseSkip::Deferred(_)))
    }

    fn wake_visible(&mut self, viewport: &Range<usize>) -> bool {
        !viewport.is_empty()
            && self.wake(|skip| {
                matches!(skip, ParseSkip::UntilVisible(range)
                    if range.start < viewport.end && viewport.start < range.end)
            })
    }

    fn wake(&mut self, matches: impl Fn(&ParseSkip) -> bool) -> bool {
        let before = self.skipped.len();
        self.skipped.retain(|skip| {
            if matches(skip) {
                self.dirty.push(skip.range().clone());
                false
            } else {
                true
            }
        });
        if self.skipped.len() == before {
            return false;
        }
        merge_ranges(&mut self.dirty);
        self.job.restart();
        true
    }

    fn covers(&self, upto: usize) -> bool {
        self.syntax_tree().is_some_and(|snapshot| {
            snapshot.parsed_to >= upto && self.dirty.first().is_none_or(|range| range.start >= upto)
        })
    }

    fn run_to(&mut self, viewport: Range<usize>, steps: usize) -> Result<(), J::Error> {
        let upto = viewport.end;
        for _ in 0..steps {
            if self.covers(upto) {
                break;
            }
            let step = self.job.advance(ParseRequest {
                text: &self.text,
                upto,
                viewport: viewport.clone(),
                previous: self.snapshot.as_ref(),
                dirty_ranges: &self.dirty,
                skipped_ranges: &self.skipped,
            });
            match step {
                Ok(ParseStep::Pending) => {}
                Ok(ParseStep::Publish {
                    tree,
                    parsed_to,
                    skipped,
                }) => {
                    assert!(self.text.is_char_boundary(parsed_to), "invalid parse end");
                    assert!(
                        skipped.iter().all(|skip| {
                            let range = skip.range();
                            !range.is_empty()
                                && range.end <= parsed_to
                                && self.text.is_valid_range(range.clone())
                        }),
                        "invalid skipped parse range"
                    );
                    self.snapshot = Some(ParseSnapshot { tree, parsed_to });
                    self.dirty.clear();
                    self.skipped = skipped;
                    self.stale = false;
                }
                Err(error) => {
                    self.job.restart();
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

/// Adapt a synchronous native parser. One step parses the entire text;
/// the service cannot interrupt it or reuse its previous output.
pub fn whole_document<T, E>(
    parse: impl FnMut(&Text) -> Result<T, E>,
) -> impl ParseJob<Tree = T, Error = E> {
    WholeDocument(parse)
}

struct WholeDocument<F>(F);

impl<T, E, F: FnMut(&Text) -> Result<T, E>> ParseJob for WholeDocument<F> {
    type Tree = T;
    type Error = E;

    fn advance(&mut self, request: ParseRequest<'_, T>) -> Result<ParseStep<T>, E> {
        Ok(ParseStep::publish(
            (self.0)(request.text)?,
            request.text.len(),
        ))
    }

    fn restart(&mut self) {}
}

fn map_range(changes: &ChangeSet, range: &Range<usize>, start: Assoc, end: Assoc) -> Range<usize> {
    changes.map_pos(range.start, start)..changes.map_pos(range.end, end)
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_unstable_by_key(|range| range.start);
    let mut count = 0;
    for index in 0..ranges.len() {
        if count > 0 && ranges[index].start <= ranges[count - 1].end {
            ranges[count - 1].end = ranges[count - 1].end.max(ranges[index].end);
        } else {
            ranges.swap(count, index);
            count += 1;
        }
    }
    ranges.truncate(count);
}
