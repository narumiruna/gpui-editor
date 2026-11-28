//! Native diff, merge chunks, and headless merge views.
//!
//! Adapted from `@codemirror/merge` 6.13.0. See
//! `docs/licenses/codemirror-merge.txt` for the upstream MIT notice.
//!
//! Offsets are UTF-8 bytes. Chunks preserve LF and CRLF and end at real EOF.
//! The diff uses Unicode scalars and a bounded work budget. Incremental update
//! windows use byte margins and line boundaries. Chunk grouping and precision
//! can differ from upstream, including at ambiguous line endings.
//!
//! Unified merge exposes control metadata and commands, not a rendered view.
//! Navigation requests scrolling through the native transaction flag, not an
//! upstream scroll range. There are no custom widget or button callbacks.

use std::{convert::Infallible, ops::Range, sync::Arc, time::Duration};

use crate::{ChangeDesc, ConfigurationError, EditError, Text};

mod diff;
mod projection;
mod session;
mod side_by_side;
mod update;

pub use diff::{
    Diff, DiffChange, DiffConfig, DiffOverride, InvalidDiffOverride, diff, presentable_diff,
};
pub use projection::{CollapseConfig, MergeProjection, ProjectionRow};
pub use session::{
    MergeChunks, MergeControl, MergeControlKind, UnifiedMerge, UnifiedMergeConfig, accept_chunk,
    get_chunks, get_original_doc, go_to_next_chunk, go_to_previous_chunk,
    original_doc_change_effect, reject_chunk, unified_merge_state, unified_merge_view,
    unified_merge_view_with_override, unified_merge_view_with_timeout,
};
pub use side_by_side::{MergeView, MergeViewConfig, RevertControl};

#[derive(Clone, Debug, Default)]
struct SessionDiff {
    timeout: Duration,
    diff_override: Option<DiffOverride>,
}

/// One of the documents in a comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// A group of changes that covers one or more lines on at least one side.
///
/// Ranges start on line boundaries. They include the last line ending when it
/// exists, keep CRLF together, and never extend beyond the end of the document.
/// An empty range represents an insertion point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
    a: Range<usize>,
    b: Range<usize>,
    changes: Vec<DiffChange>,
    precise: bool,
}

impl Chunk {
    /// The line range in the requested document.
    pub fn range(&self, side: Side) -> Range<usize> {
        match side {
            Side::A => self.a.clone(),
            Side::B => self.b.clone(),
        }
    }

    /// Changes relative to the two starts of this chunk.
    pub fn changes(&self) -> &[DiffChange] {
        &self.changes
    }

    /// Changes in document coordinates.
    pub fn absolute_changes(&self) -> impl Iterator<Item = DiffChange> + '_ {
        self.changes.iter().map(|ch| DiffChange {
            a: offset(&ch.a, self.a.start),
            b: offset(&ch.b, self.b.start),
        })
    }

    /// False if this chunk was built using a coarse diff.
    pub fn precise(&self) -> bool {
        self.precise
    }

    pub fn build(a: &Text, b: &Text) -> Vec<Self> {
        Self::build_with_config(a, b, DiffConfig::default())
    }

    pub fn build_with_config(a: &Text, b: &Text, config: DiffConfig) -> Vec<Self> {
        Self::build_with_timeout(a, b, config, Duration::ZERO)
    }

    /// Build chunks with the advisory time limit of [`DiffConfig::diff_with_timeout`].
    pub fn build_with_timeout(
        a: &Text,
        b: &Text,
        config: DiffConfig,
        timeout: Duration,
    ) -> Vec<Self> {
        let (a, b) = (a.as_str(), b.as_str());
        build(a, b, config.presentable_diff_with_timeout(a, b, timeout))
    }

    /// Build chunks from the checked override and the native display and line
    /// rules. Return an error if the override returns invalid ranges.
    pub fn build_with_override(
        a: &Text,
        b: &Text,
        diff_override: &DiffOverride,
    ) -> Result<Vec<Self>, InvalidDiffOverride> {
        let (a, b) = (a.as_str(), b.as_str());
        Ok(build(a, b, diff_override.presentable_diff(a, b)?))
    }

    /// Copy this chunk from the other document into `destination`. Return the
    /// new destination without changing either input.
    ///
    /// Use the same documents from which this chunk was built. Return
    /// [`EditError::InvalidRange`] if either range cannot be sliced. This does
    /// not detect edits that leave the old ranges valid.
    pub fn apply(&self, a: &Text, b: &Text, destination: Side) -> Result<Text, EditError> {
        let (source, source_range, target, target_range) = match destination {
            Side::A => (b, self.b.clone(), a, self.a.clone()),
            Side::B => (a, self.a.clone(), b, self.b.clone()),
        };
        let insert = source.slice(source_range).ok_or(EditError::InvalidRange)?;
        target.replace(target_range, insert)
    }

    /// Update the chunks after editing one side. Supply both current documents
    /// and a change description from the previous version of `edited`.
    ///
    /// Recompute areas around the edits and reuse chunks outside them.
    /// The result can group changes differently from a full build. If the
    /// update ranges are inconsistent, rebuild from the current documents.
    pub fn update(
        chunks: &[Self],
        a: &Text,
        b: &Text,
        edited: Side,
        changes: &ChangeDesc,
        config: DiffConfig,
    ) -> Vec<Self> {
        Self::update_with_timeout(chunks, a, b, edited, changes, config, Duration::ZERO)
    }

    /// Update chunks as in [`Self::update`]. Each rebuilt window starts a new
    /// advisory time limit. Zero disables the limit.
    pub fn update_with_timeout(
        chunks: &[Self],
        a: &Text,
        b: &Text,
        edited: Side,
        changes: &ChangeDesc,
        config: DiffConfig,
        timeout: Duration,
    ) -> Vec<Self> {
        Self::update_with(chunks, a, b, edited, changes, |a, b| {
            Ok::<_, Infallible>(config.presentable_diff_with_timeout(a, b, timeout))
        })
        .unwrap()
    }

    /// Update chunks with the same windows as [`Self::update`]. For each
    /// rebuilt window, the callback receives the current slice from A followed
    /// by the current slice from B. An empty edit does not call the override.
    /// Invalid callback ranges return an error instead of replacing the chunks.
    pub fn update_with_override(
        chunks: &[Self],
        a: &Text,
        b: &Text,
        edited: Side,
        changes: &ChangeDesc,
        diff_override: &DiffOverride,
    ) -> Result<Vec<Self>, InvalidDiffOverride> {
        Self::update_with(chunks, a, b, edited, changes, |a, b| {
            diff_override.presentable_diff(a, b)
        })
    }

    fn build_for_session(
        a: &Text,
        b: &Text,
        config: DiffConfig,
        policy: &SessionDiff,
    ) -> Result<Arc<[Self]>, ConfigurationError> {
        let chunks = match &policy.diff_override {
            Some(diff_override) => Self::build_with_override(a, b, diff_override)
                .map_err(|_| ConfigurationError::InvalidFieldData)?,
            None => Self::build_with_timeout(a, b, config, policy.timeout),
        };
        Ok(chunks.into())
    }

    fn update_for_session(
        chunks: &[Self],
        a: &Text,
        b: &Text,
        edited: Side,
        changes: &ChangeDesc,
        config: DiffConfig,
        policy: &SessionDiff,
    ) -> Result<Arc<[Self]>, ConfigurationError> {
        let chunks = match &policy.diff_override {
            Some(diff_override) => {
                Self::update_with_override(chunks, a, b, edited, changes, diff_override)
                    .map_err(|_| ConfigurationError::InvalidFieldData)?
            }
            None => {
                Self::update_with_timeout(chunks, a, b, edited, changes, config, policy.timeout)
            }
        };
        Ok(chunks.into())
    }

    fn update_with<E>(
        chunks: &[Self],
        a: &Text,
        b: &Text,
        edited: Side,
        changes: &ChangeDesc,
        diff: impl Fn(&str, &str) -> Result<Diff, E>,
    ) -> Result<Vec<Self>, E> {
        let rebuild = |a: &str, b: &str| Ok(build(a, b, diff(a, b)?));
        update::update(chunks, a, b, edited, changes, rebuild)
            .unwrap_or_else(|| rebuild(a.as_str(), b.as_str()))
    }
}

fn build(a: &str, b: &str, diff: Diff) -> Vec<Chunk> {
    let Diff {
        mut changes,
        precise,
    } = diff;
    if changes.is_empty() {
        return Vec::new();
    }
    align_line_changes(a, b, &mut changes);
    let (mut lines_a, mut lines_b) = (Lines::new(a), Lines::new(b));
    let mut chunks = Vec::new();
    for change in changes {
        let (from_a, from_b) = (
            lines_a.at(change.a.start).start,
            lines_b.at(change.b.start).start,
        );
        let (line_a, line_b) = (lines_a.at(change.a.end), lines_b.at(change.b.end));
        let (to_a, to_b) = if line_a.start == change.a.end && line_b.start == change.b.end {
            (change.a.end, change.b.end)
        } else {
            (line_a.end, line_b.end)
        };
        let relative = DiffChange {
            a: change.a.start - from_a..change.a.end - from_a,
            b: change.b.start - from_b..change.b.end - from_b,
        };
        if let Some(previous) = chunks.last_mut()
            && near(previous, from_a, from_b)
        {
            previous.a.end = to_a;
            previous.b.end = to_b;
            previous.changes.push(DiffChange {
                a: offset(&relative.a, from_a - previous.a.start),
                b: offset(&relative.b, from_b - previous.b.start),
            });
        } else {
            chunks.push(Chunk {
                a: from_a..to_a,
                b: from_b..to_b,
                changes: vec![relative],
                precise,
            });
        }
    }
    chunks
}

// A pure insertion or deletion can start at the preceding line ending and
// stop just before an identical ending. Shift the change past the first ending
// so the chunk starts on the changed line. Upstream shifts only the chunk
// start, which can leave a negative relative offset.
fn align_line_changes(a: &str, b: &str, changes: &mut [DiffChange]) {
    for i in 0..changes.len() {
        let (next_a, next_b) = changes
            .get(i + 1)
            .map_or((a.len(), b.len()), |next| (next.a.start, next.b.start));
        let change = &mut changes[i];
        if change.a.is_empty() == change.b.is_empty() {
            continue;
        }
        let width = ending_width(a, change.a.start);
        if width != 0
            && change.a.len().max(change.b.len()) > width
            && change.a.end + width <= next_a
            && change.b.end + width <= next_b
            && ending_width(b, change.b.start) == width
            && ending_width(a, change.a.end) == width
            && ending_width(b, change.b.end) == width
        {
            change.a = offset(&change.a, width);
            change.b = offset(&change.b, width);
        }
    }
}

fn ending_width(text: &str, at: usize) -> usize {
    match text.as_bytes().get(at..) {
        Some([b'\r', b'\n', ..]) => 2,
        Some([b'\n', ..]) if at == 0 || text.as_bytes()[at - 1] != b'\r' => 1,
        _ => 0,
    }
}

fn offset(range: &Range<usize>, by: usize) -> Range<usize> {
    range.start + by..range.end + by
}

fn near(previous: &Chunk, start_a: usize, start_b: usize) -> bool {
    start_a.saturating_sub(previous.a.end) <= 1 || start_b.saturating_sub(previous.b.end) <= 1
}

// Diff positions on each side are ordered. Scan each line ending at most once.
struct Lines<'a> {
    remaining: std::str::MatchIndices<'a, char>,
    len: usize,
    line: Range<usize>,
    has_ending: bool,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut remaining = text.match_indices('\n');
        let ending = remaining.next();
        Self {
            remaining,
            len: text.len(),
            line: 0..ending.map_or(text.len(), |(pos, _)| pos + 1),
            has_ending: ending.is_some(),
        }
    }

    fn at(&mut self, pos: usize) -> Range<usize> {
        while self.has_ending && pos >= self.line.end {
            let ending = self.remaining.next();
            self.line = self.line.end..ending.map_or(self.len, |(pos, _)| pos + 1);
            self.has_ending = ending.is_some();
        }
        self.line.clone()
    }
}
