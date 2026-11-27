use std::ops::Range;

use super::{Chunk, Side};
use crate::{Assoc, ChangeDesc, MapMode, Text};

/// Context and size threshold for collapsing unchanged lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CollapseConfig {
    /// Number of context lines to keep next to each chunk.
    pub margin: usize,
    /// Minimum number of lines to hide, after subtracting context.
    /// Zero has the same effect as one.
    pub min_size: usize,
}

impl Default for CollapseConfig {
    fn default() -> Self {
        Self {
            margin: 3,
            min_size: 4,
        }
    }
}

/// One visual row in a fixed-height merge view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionRow {
    /// A visible line. `number` is one-based. `range` excludes the line ending.
    Line { number: usize, range: Range<usize> },
    /// A row that replaces the one-based, end-exclusive `lines` range.
    /// The byte range ends before the last hidden line's ending.
    Collapsed {
        lines: Range<usize>,
        range: Range<usize>,
    },
    /// A blank row. `at` is the byte position of the next row, or the document
    /// length if this spacer follows the last row.
    Spacer { at: usize },
}

/// Two row lists for a snapshot of a merge comparison.
///
/// This is the fixed-height counterpart of `@codemirror/merge` 6.13.0's
/// collapsed ranges and alignment spacers. It uses native [`Chunk`] boundaries
/// and UTF-8 byte offsets. Each line, collapsed range, and spacer takes one row.
/// It does not measure wrapping, widgets, or viewport heights.
#[derive(Clone, Debug)]
pub struct MergeProjection {
    a: Column,
    b: Column,
    gaps: Vec<Gap>,
}

impl MergeProjection {
    /// Project `a` and `b` using their current, ordered chunks. Use `None` to
    /// show all lines or `Some` to collapse unchanged ranges.
    ///
    /// # Panics
    ///
    /// Chunk offsets must be valid positions in the respective documents.
    pub fn new(a: &Text, b: &Text, chunks: &[Chunk], collapse: Option<CollapseConfig>) -> Self {
        let mut projection = Self::empty(a, b, chunks);
        if let Some(config) = collapse {
            for (i, gap) in projection.gaps.iter().enumerate() {
                let first = i == 0;
                let last = i == chunks.len();
                projection.a.collapse(&gap.a, first, last, config);
                projection.b.collapse(&gap.b, first, last, config);
            }
        }
        projection.rebuild_rows();
        projection
    }

    fn empty(a: &Text, b: &Text, chunks: &[Chunk]) -> Self {
        let mut gaps = Vec::with_capacity(chunks.len() + 1);
        let (mut pos_a, mut pos_b) = (0, 0);
        for i in 0..=chunks.len() {
            let chunk_a = chunks.get(i).map(|chunk| chunk.range(Side::A));
            let chunk_b = chunks.get(i).map(|chunk| chunk.range(Side::B));
            let end_a = chunk_a.as_ref().map(|range| range.start);
            let end_b = chunk_b.as_ref().map(|range| range.start);
            gaps.push(Gap {
                a: GapSide::new(a, pos_a, end_a),
                b: GapSide::new(b, pos_b, end_b),
                // Upstream syncs at the start of nonempty unchanged byte
                // ranges, not at a trailing empty line by itself.
                align: pos_a < end_a.unwrap_or(a.len()),
            });
            if let (Some(a), Some(b)) = (chunk_a, chunk_b) {
                (pos_a, pos_b) = (a.end, b.end);
            }
        }
        Self {
            a: Column::new(a),
            b: Column::new(b),
            gaps,
        }
    }

    /// View the rows on one side. Both sides have the same row count.
    pub fn rows(&self, side: Side) -> &[ProjectionRow] {
        &self.column(side).rows
    }

    /// The hidden ranges on one side, in UTF-8 bytes. The ranges describe
    /// replacement rows; they do not block transactions or cursor positions.
    pub fn collapsed_ranges(&self, side: Side) -> &[Range<usize>] {
        &self.column(side).collapsed
    }

    /// Expand the collapsed row that starts at `byte_start` on `side`.
    /// Also expand its peer if present and recompute alignment. Return false
    /// when no collapsed row starts there.
    pub fn expand(&mut self, side: Side, byte_start: usize) -> bool {
        if !self.column_mut(side).remove_collapse(byte_start) {
            return false;
        }
        let peer_start = self.gaps.iter().rev().find_map(|gap| {
            let (ours, peer) = match side {
                Side::A => (&gap.a, &gap.b),
                Side::B => (&gap.b, &gap.a),
            };
            Some(peer.at + byte_start.checked_sub(ours.at)?)
        });
        if let Some(peer_start) = peer_start {
            self.column_mut(side.other()).remove_collapse(peer_start);
        }
        self.rebuild_rows();
        true
    }

    pub(super) fn update(
        &self,
        a: &Text,
        b: &Text,
        chunks: &[Chunk],
        edited: Side,
        changes: &ChangeDesc,
    ) -> Self {
        let mut projection = Self::empty(a, b, chunks);
        for side in [Side::A, Side::B] {
            let column = projection.column_mut(side);
            column.collapsed = self
                .collapsed_ranges(side)
                .iter()
                .filter_map(|range| {
                    update_collapsed_range(
                        &column.doc,
                        chunks,
                        side,
                        range,
                        (side == edited).then_some(changes),
                    )
                })
                .collect();
        }
        projection.rebuild_rows();
        projection
    }

    fn column(&self, side: Side) -> &Column {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }

    fn column_mut(&mut self, side: Side) -> &mut Column {
        match side {
            Side::A => &mut self.a,
            Side::B => &mut self.b,
        }
    }

    fn rebuild_rows(&mut self) {
        self.a.rows.clear();
        self.b.rows.clear();
        let (mut next_a, mut next_b) = (1, 1);
        for gap in &self.gaps {
            self.a.push_lines(next_a..gap.a.lines.start);
            self.b.push_lines(next_b..gap.b.lines.start);
            if gap.align {
                align(&mut self.a.rows, &mut self.b.rows, gap.a.at, gap.b.at);
            }
            self.a.push_lines(gap.a.lines.clone());
            self.b.push_lines(gap.b.lines.clone());
            (next_a, next_b) = (gap.a.lines.end, gap.b.lines.end);
        }
        align(
            &mut self.a.rows,
            &mut self.b.rows,
            self.a.doc.len(),
            self.b.doc.len(),
        );
    }
}

pub(super) fn build_collapsed_ranges(
    doc: &Text,
    chunks: &[Chunk],
    side: Side,
    config: CollapseConfig,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for i in 0..=chunks.len() {
        let next = chunks.get(i).map(|chunk| chunk.range(side));
        let gap = GapSide::new(doc, start, next.as_ref().map(|range| range.start));
        ranges.extend(gap.collapse(doc, i == 0, next.is_none(), config));
        if let Some(next) = next {
            start = next.end;
        }
    }
    ranges
}

pub(super) fn update_collapsed_range(
    doc: &Text,
    chunks: &[Chunk],
    side: Side,
    range: &Range<usize>,
    changes: Option<&ChangeDesc>,
) -> Option<Range<usize>> {
    let range = match changes {
        Some(changes) => map_collapse(range, changes)?,
        None => range.clone(),
    };
    // Keep only complete lines and discard ranges that touch a changed chunk.
    let i = chunks.partition_point(|chunk| chunk.range(side).end < range.start);
    (chunks
        .get(i)
        .is_none_or(|chunk| chunk.range(side).start > range.end)
        && doc.line_at(range.start)?.range.start == range.start
        && doc.line_at(range.end)?.range.end == range.end)
        .then_some(range)
}

fn map_collapse(range: &Range<usize>, changes: &ChangeDesc) -> Option<Range<usize>> {
    let mode = if range.is_empty() {
        MapMode::TrackBefore
    } else {
        MapMode::Simple
    };
    let start = changes.map_pos(range.start, Assoc::Before, mode)?;
    let end = changes.map_pos(range.end, Assoc::After, MapMode::Simple)?;
    Some(start..end)
}

#[derive(Clone, Debug)]
struct Gap {
    a: GapSide,
    b: GapSide,
    align: bool,
}

#[derive(Clone, Debug)]
struct GapSide {
    at: usize,
    lines: Range<usize>,
}

impl GapSide {
    fn new(doc: &Text, at: usize, end: Option<usize>) -> Self {
        // At EOF, emit the final line even if it has no ending.
        let line_at = |pos| doc.line_at(pos).expect("invalid merge chunk offset").number;
        let lines = line_at(at)..end.map_or_else(|| doc.line_count() + 1, line_at);
        Self { at, lines }
    }

    fn collapse(
        &self,
        doc: &Text,
        first: bool,
        last: bool,
        config: CollapseConfig,
    ) -> Option<Range<usize>> {
        let from = self
            .lines
            .start
            .saturating_add(if first { 0 } else { config.margin });
        let to = self
            .lines
            .end
            .saturating_sub(if last { 0 } else { config.margin });
        (to.saturating_sub(from) >= config.min_size.max(1))
            .then(|| line_range(doc, from).start..line_range(doc, to - 1).end)
    }
}

#[derive(Clone, Debug)]
struct Column {
    doc: Text,
    collapsed: Vec<Range<usize>>,
    rows: Vec<ProjectionRow>,
}

impl Column {
    fn new(doc: &Text) -> Self {
        Self {
            doc: doc.clone(),
            collapsed: Vec::new(),
            rows: Vec::new(),
        }
    }

    fn collapse(&mut self, gap: &GapSide, first: bool, last: bool, config: CollapseConfig) {
        // With zero margin the range after a chunk touches it and is excluded.
        if !first && config.margin == 0 {
            return;
        }
        self.collapsed
            .extend(gap.collapse(&self.doc, first, last, config));
    }

    fn remove_collapse(&mut self, at: usize) -> bool {
        let Ok(i) = self
            .collapsed
            .binary_search_by_key(&at, |range| range.start)
        else {
            return false;
        };
        self.collapsed.remove(i);
        true
    }

    fn push_lines(&mut self, lines: Range<usize>) {
        if lines.is_empty() {
            return;
        }
        let bytes =
            line_range(&self.doc, lines.start).start..=line_range(&self.doc, lines.end - 1).end;
        let i = self
            .collapsed
            .partition_point(|range| range.start < *bytes.start());
        let mut next = lines.start;
        for range in self.collapsed[i..]
            .iter()
            .take_while(|range| bytes.contains(&range.start))
        {
            let from = self
                .doc
                .line_at(range.start)
                .expect("start of collapsed lines")
                .number;
            let to = self
                .doc
                .line_at(range.end)
                .expect("end of collapsed lines")
                .number
                + 1;
            self.rows.extend(visible_lines(&self.doc, next..from));
            self.rows.push(ProjectionRow::Collapsed {
                lines: from..to,
                range: range.clone(),
            });
            next = to;
        }
        self.rows.extend(visible_lines(&self.doc, next..lines.end));
    }
}

fn visible_lines(doc: &Text, lines: Range<usize>) -> impl Iterator<Item = ProjectionRow> + '_ {
    lines.map(|number| ProjectionRow::Line {
        number,
        range: line_range(doc, number),
    })
}

fn line_range(doc: &Text, number: usize) -> Range<usize> {
    doc.line(number).expect("line in merge projection").range
}

fn align(a: &mut Vec<ProjectionRow>, b: &mut Vec<ProjectionRow>, at_a: usize, at_b: usize) {
    if a.len() < b.len() {
        a.resize(b.len(), ProjectionRow::Spacer { at: at_a });
    } else if b.len() < a.len() {
        b.resize(a.len(), ProjectionRow::Spacer { at: at_b });
    }
}
