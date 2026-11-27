use std::ops::Range;

use gpui::{Pixels, ShapedLine, px};
use gpui_editor_codemirror::{
    Assoc, EditorSelection, SelectionRange,
    view::bidi::{BidiSpan, Direction, bidi_spans},
};
use unicode_segmentation::UnicodeSegmentation;

/// Caret geometry for one complete, unwrapped LTR source line with RTL text.
/// Byte positions are relative to the line, not to the document.
pub(super) struct BidiRow {
    len: usize,
    spans: Vec<BidiSpan>,
    clusters: Vec<Cluster>,
}

struct Cluster {
    bytes: Range<usize>,
    x: Range<Pixels>,
    span: usize,
}

impl BidiRow {
    /// Use the positions of the glyphs that GPUI paints. GPUI 0.2.2 does not
    /// expose both caret positions at a bidi boundary or stops inside ligatures.
    /// Reject a row if its shaped clusters do not match CodeMirror visual order.
    pub(super) fn new(line: &ShapedLine) -> Option<Self> {
        if line.len() != line.text.len() {
            return None;
        }
        let glyphs = line
            .runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| (glyph.index, glyph.position.x));
        Self::measure(&line.text, line.width, glyphs)
    }

    pub(super) fn measure(
        text: &str,
        width: Pixels,
        glyphs: impl IntoIterator<Item = (usize, Pixels)>,
    ) -> Option<Self> {
        if text.is_ascii() {
            return None;
        }
        if text.is_ascii() {
            return None;
        }
        let spans = bidi_spans(text, Direction::Ltr, &[]).ok()?;
        if !f32::from(width).is_finite()
            || !spans.iter().any(|span| span.direction() == Direction::Rtl)
        {
            return None;
        }
        let boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain([text.len()])
            .collect();
        let mut glyphs = glyphs.into_iter().peekable();
        let mut clusters: Vec<Cluster> = Vec::with_capacity(boundaries.len() - 1);
        for (span_index, span) in spans.iter().enumerate() {
            let from = boundaries.binary_search(&span.from).ok()?;
            let to = boundaries.binary_search(&span.to).ok()?;
            for i in 0..to - from {
                let i = if span.direction() == Direction::Rtl {
                    to - 1 - i
                } else {
                    from + i
                };
                let bytes = boundaries[i]..boundaries[i + 1];
                let (index, x) = glyphs.next()?;
                if !bytes.contains(&index) || !text.is_char_boundary(index) {
                    return None;
                }
                while glyphs
                    .peek()
                    .is_some_and(|(index, _)| bytes.contains(index))
                {
                    let (index, next_x) = glyphs.next()?;
                    if next_x != x || !text.is_char_boundary(index) {
                        return None;
                    }
                }
                if !f32::from(x).is_finite() || x >= width {
                    return None;
                }
                if let Some(previous) = clusters.last_mut() {
                    if x <= previous.x.start {
                        return None;
                    }
                    previous.x.end = x;
                } else if x != px(0.) {
                    return None;
                }
                clusters.push(Cluster {
                    bytes,
                    x: x..width,
                    span: span_index,
                });
            }
        }
        if glyphs.next().is_some() || clusters.is_empty() {
            return None;
        }
        Some(Self {
            len: text.len(),
            spans,
            clusters,
        })
    }

    pub(super) fn spans(&self) -> &[BidiSpan] {
        &self.spans
    }

    pub(super) fn x_at(
        &self,
        byte: usize,
        assoc: Option<Assoc>,
        level: Option<u8>,
    ) -> Option<Pixels> {
        // CodeMirror uses these associations for the exterior of the logical line.
        if byte == 0 && assoc == Some(Assoc::Before) {
            return Some(px(0.));
        }
        if byte == self.len && assoc == Some(Assoc::After) {
            return Some(self.clusters.last()?.x.end);
        }
        let span_index = BidiSpan::find(&self.spans, byte, level.map(usize::from), assoc)?;
        let span = self.spans[span_index];
        self.clusters
            .iter()
            .filter(|cluster| cluster.span == span_index)
            .find_map(|cluster| {
                if byte == cluster.bytes.start {
                    Some(if span.direction() == Direction::Ltr {
                        cluster.x.start
                    } else {
                        cluster.x.end
                    })
                } else if byte == cluster.bytes.end {
                    Some(if span.direction() == Direction::Ltr {
                        cluster.x.end
                    } else {
                        cluster.x.start
                    })
                } else {
                    None
                }
            })
    }

    pub(super) fn hit(&self, x: Pixels) -> SelectionRange {
        let index = self.clusters.partition_point(|cluster| cluster.x.end <= x);
        let cluster = &self.clusters[index.min(self.clusters.len() - 1)];
        let right = x - cluster.x.start > cluster.x.end - x;
        let span = self.spans[cluster.span];
        let logical_end = right == (span.direction() == Direction::Ltr);
        let (byte, assoc) = if logical_end {
            (cluster.bytes.end, Assoc::Before)
        } else {
            (cluster.bytes.start, Assoc::After)
        };
        EditorSelection::cursor(byte, Some(assoc), Some(span.level.min(6) as u8), None)
    }

    /// Return the visual intervals for a logical range. Include the full
    /// cluster if a range endpoint falls inside it.
    pub(super) fn selection_spans(&self, bytes: Range<usize>) -> Vec<Range<Pixels>> {
        let mut result: Vec<Range<Pixels>> = Vec::new();
        if !bytes.is_empty() {
            for cluster in &self.clusters {
                if bytes.start < cluster.bytes.end && cluster.bytes.start < bytes.end {
                    if let Some(last) = result.last_mut()
                        && last.end == cluster.x.start
                    {
                        last.end = cluster.x.end;
                    } else {
                        result.push(cluster.x.clone());
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use gpui_editor_codemirror::{Text, view::bidi::move_visually};

    use super::*;

    #[test]
    fn bidi_boundaries_follow_affinity_and_hit_testing() {
        let row = BidiRow::measure(
            "ab אב cd",
            px(80.),
            [0, 1, 2, 5, 3, 7, 8, 9]
                .into_iter()
                .enumerate()
                .map(|(x, byte)| (byte, px(x as f32 * 10.))),
        )
        .unwrap();
        assert_eq!(row.x_at(3, Some(Assoc::Before), None), Some(px(30.)));
        assert_eq!(row.x_at(3, Some(Assoc::After), None), Some(px(50.)));
        assert_eq!(row.x_at(3, Some(Assoc::Before), Some(1)), Some(px(50.)));
        for (x, head, assoc, level, caret_x) in [
            (29., 3, Assoc::Before, 0, 30.),
            (31., 7, Assoc::Before, 1, 30.),
            (41., 5, Assoc::Before, 1, 40.),
            (49., 3, Assoc::After, 1, 50.),
            (51., 7, Assoc::After, 0, 50.),
        ] {
            let hit = row.hit(px(x));
            assert_eq!(
                (hit.head(), hit.assoc(), hit.bidi_level()),
                (head, Some(assoc), Some(level))
            );
            assert_eq!(
                row.x_at(hit.head(), hit.assoc(), hit.bidi_level()),
                Some(px(caret_x))
            );
        }
        assert_eq!(
            row.selection_spans(1..5),
            vec![px(10.)..px(30.), px(40.)..px(50.)]
        );
    }

    #[test]
    fn native_motion_keeps_document_bytes_after_crlf() {
        let text = Text::new("```\r\nab אב cd\r\n```\r\n");
        let line = text.line(2).unwrap();
        let row = BidiRow::measure(
            line.text,
            px(80.),
            [0, 1, 2, 5, 3, 7, 8, 9]
                .into_iter()
                .enumerate()
                .map(|(x, byte)| (byte, px(x as f32 * 10.))),
        )
        .unwrap();
        assert_eq!(line.range, 5..15);
        let mut selection = EditorSelection::cursor(8, Some(Assoc::Before), Some(0), None);
        for (head, x, crossed) in [(10, 40., "ב"), (12, 50., "א")] {
            let step = move_visually(&line, row.spans(), Direction::Ltr, selection, true).unwrap();
            selection = step.selection;
            assert_eq!((selection.head(), step.moved_over), (head, crossed));
            assert_eq!(
                row.x_at(
                    head - line.range.start,
                    selection.assoc(),
                    selection.bidi_level()
                ),
                Some(px(x))
            );
        }
        assert_eq!(&text.as_str()[line.range.end..line.range.end + 2], "\r\n");
    }

    #[test]
    fn incomplete_cluster_geometry_is_not_measurable() {
        assert!(
            BidiRow::measure("x لا", px(30.), [(0, px(0.)), (1, px(10.)), (2, px(20.))]).is_none()
        );
        assert!(
            BidiRow::measure(
                "אב x",
                px(40.),
                [(5, px(0.)), (4, px(10.)), (2, px(20.)), (0, px(30.))]
            )
            .is_none()
        );
    }
}
