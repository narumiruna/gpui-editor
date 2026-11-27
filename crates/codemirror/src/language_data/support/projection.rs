use std::ops::Range;

use crate::{Assoc, language::html::HtmlDocument};

use super::NativeSyntax;

/// Map the compact HTML tree in a template back to original byte positions.
pub(super) struct HtmlProjection<'a> {
    pub document: &'a HtmlDocument,
    segments: Vec<Segment>,
}

pub(super) struct Segment {
    pub source: Range<usize>,
    html: Range<usize>,
}

impl<'a> HtmlProjection<'a> {
    pub fn new(syntax: &'a NativeSyntax) -> Option<Self> {
        let (document, ranges) = match syntax {
            NativeSyntax::Jinja(doc) => {
                let html = doc.html()?;
                (
                    html.document(),
                    html.source_ranges(0..html.document().text().len())?,
                )
            }
            NativeSyntax::Liquid(doc) => {
                let html = doc.html()?;
                (
                    html.document(),
                    html.ranges()
                        .iter()
                        .map(|range| range.source.clone())
                        .collect(),
                )
            }
            NativeSyntax::Php(doc) => {
                let html = doc.html()?;
                (
                    html.document(),
                    html.source_ranges(0..html.document().text().len())?,
                )
            }
            _ => return None,
        };
        let mut segments: Vec<Segment> = Vec::new();
        let mut end = 0;
        for source in ranges {
            let start = end;
            end += source.len();
            if let Some(last) = segments
                .last_mut()
                .filter(|seg| seg.source.end == source.start)
            {
                last.source.end = source.end;
                last.html.end = end;
            } else {
                segments.push(Segment {
                    source,
                    html: start..end,
                });
            }
        }
        Some(Self { document, segments })
    }

    pub fn at(&self, position: usize, side: Assoc) -> Option<(&Segment, usize)> {
        let index = self.segments.partition_point(|seg| {
            seg.source.end < position || (side == Assoc::After && seg.source.end == position)
        });
        let segment = self.segments.get(index)?;
        let delta = position.checked_sub(segment.source.start)?;
        if delta == 0 && side == Assoc::Before {
            return None;
        }
        let projected = segment.html.start + delta;
        self.document
            .text()
            .is_char_boundary(projected)
            .then_some((segment, projected))
    }

    /// Map only one contiguous span. Use this for text copied or replaced by edits.
    pub fn source(&self, range: Range<usize>) -> Option<Range<usize>> {
        let index = self
            .segments
            .partition_point(|seg| seg.html.end <= range.start);
        let segment = self.segments.get(index)?;
        (range.end <= segment.html.end).then(|| {
            let from = segment.source.start + (range.start - segment.html.start);
            from..from + range.len()
        })
    }

    /// Project a source position to HTML, collapsing gaps to their boundaries.
    pub fn project_boundary(&self, position: usize) -> usize {
        let index = self
            .segments
            .partition_point(|seg| seg.source.end <= position);
        self.segments
            .get(index)
            .map_or(self.document.text().len(), |segment| {
                segment.html.start + position.saturating_sub(segment.source.start)
            })
    }

    /// Map a nonempty HTML node to its enclosing source interval. This can
    /// span a template gap. Use it for selection, not text edits.
    pub fn selection_bounds(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.is_empty() {
            return None;
        }
        let first = self.segments.get(
            self.segments
                .partition_point(|seg| seg.html.end <= range.start),
        )?;
        let last = self.segments.get(
            self.segments
                .partition_point(|seg| seg.html.end < range.end),
        )?;
        let start = first.source.start + (range.start - first.html.start);
        let end = last.source.start + (range.end - last.html.start);
        Some(start..end)
    }
}
