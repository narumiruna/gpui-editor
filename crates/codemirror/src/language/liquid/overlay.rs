use std::ops::Range;

use crate::{Assoc, Text};

use super::{
    super::html::{HtmlDocument, HtmlNesting, HtmlOptions},
    HighlightSpan, IndentOptions,
    LiquidNodeKind::*,
    Node, StreamSettings,
};

/// Same bytes in the Liquid source and in the compact HTML document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidHtmlRange {
    pub source: Range<usize>,
    pub html: Range<usize>,
}

/// The existing native HTML parser mounted over the template's plain and raw text.
#[derive(Debug)]
pub struct LiquidHtmlMount {
    document: HtmlDocument,
    ranges: Vec<LiquidHtmlRange>,
}

impl LiquidHtmlMount {
    pub(super) fn new(
        source: &str,
        nodes: &[Node],
        options: HtmlOptions,
        nesting: HtmlNesting<'_>,
    ) -> Option<Self> {
        let mut compact = String::new();
        let mut ranges: Vec<LiquidHtmlRange> = Vec::new();
        for node in nodes
            .iter()
            .filter(|node| matches!(node.kind, Text | RawText) && !node.range.is_empty())
        {
            let start = compact.len();
            compact.push_str(&source[node.range.clone()]);
            if let Some(last) = ranges.last_mut()
                && last.source.end == node.range.start
            {
                last.source.end = node.range.end;
                last.html.end = compact.len();
            } else {
                ranges.push(LiquidHtmlRange {
                    source: node.range.clone(),
                    html: start..compact.len(),
                });
            }
        }
        if ranges.is_empty() {
            return None;
        }
        let original = |range: Range<usize>| {
            let first = ranges.get(ranges.partition_point(|part| part.html.end <= range.start))?;
            let last = ranges.get(ranges.partition_point(|part| part.html.end < range.end))?;
            Some(
                first.source.start + range.start - first.html.start
                    ..last.source.start + range.end - last.html.start,
            )
        };
        let document = HtmlDocument::parse_projected(
            Text::new(compact),
            options,
            nesting,
            (source, &original),
        );
        Some(Self { document, ranges })
    }

    /// The HTML tree and its native child mounts use compact byte offsets.
    pub fn document(&self) -> &HtmlDocument {
        &self.document
    }

    pub fn ranges(&self) -> &[LiquidHtmlRange] {
        &self.ranges
    }

    /// Map an original position. Return `None` for a gap or a non-boundary byte.
    pub fn to_html(&self, position: usize) -> Option<usize> {
        let index = self.ranges.partition_point(|r| r.source.end < position);
        let r = self.ranges.get(index)?;
        let html = r.html.start + position.checked_sub(r.source.start)?;
        self.document
            .text()
            .as_str()
            .is_char_boundary(html)
            .then_some(html)
    }

    /// Map an HTML boundary. At a gap, select the preceding or following source range.
    pub fn to_source(&self, position: usize, side: Assoc) -> Option<usize> {
        if !self.document.text().as_str().is_char_boundary(position) {
            return None;
        }
        let index = self.ranges.partition_point(|r| {
            r.html.end < position || (side == Assoc::After && r.html.end == position)
        });
        let r = self.ranges.get(index).or_else(|| self.ranges.last())?;
        Some(r.source.start + position - r.html.start)
    }

    /// Return the original source pieces for an HTML range. A node can cross a gap.
    pub fn source_ranges(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = Range<usize>> + '_> {
        self.document.text().slice(range.clone())?;
        let Range { start, end } = range;
        let first = self.ranges.partition_point(|r| r.html.end <= start);
        Some(
            self.ranges[first..]
                .iter()
                .take_while(move |r| start < end && r.html.start < end)
                .map(move |r| {
                    let start = r.html.start.max(start);
                    let end = r.html.end.min(end);
                    (r.source.start + start - r.html.start)..(r.source.start + end - r.html.start)
                }),
        )
    }

    // Collapse gaps to their shared boundary. Used for covering or clipping queries,
    // not for projections of individual source positions.
    fn compact_offset(&self, position: usize) -> usize {
        let index = self.ranges.partition_point(|r| r.source.end <= position);
        self.ranges
            .get(index)
            .map_or(self.document.text().len(), |r| {
                r.html.start + position.saturating_sub(r.source.start)
            })
    }

    pub(super) fn highlights<'a>(
        &'a self,
        range: &Range<usize>,
        spans: &mut Vec<HighlightSpan<'a>>,
    ) {
        let query = self.compact_offset(range.start)..self.compact_offset(range.end);
        if query.is_empty() {
            return;
        }
        for span in self.document.highlights(query).into_iter().flatten() {
            spans.extend(
                self.source_ranges(span.range)
                    .into_iter()
                    .flatten()
                    .map(|range| HighlightSpan {
                        range,
                        tags: span.tags,
                    }),
            );
        }
    }

    pub(super) fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        let end = self.to_html(line_end)?;
        let fold = self
            .document
            .foldable(self.compact_offset(line_start), end)?;
        let start = self.to_source(fold.start, Assoc::Before)?;
        let end = self.to_source(fold.end, Assoc::After)?;
        (line_start <= start && start <= line_end && line_end < end).then_some(start..end)
    }

    pub(super) fn indentation(
        &self,
        text: &Text,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let html = self.to_html(position)?;
        let context = super::super::IndentContext::new(text, position, settings, options)?;
        let over = |at| {
            let at = self.to_source(at, Assoc::After)?;
            context.line_indent(at, Assoc::After)
        };
        self.document.indentation(
            html,
            settings,
            IndentOptions {
                override_indentation: Some(&over),
                simulate_break: options.simulate_break.and_then(|point| self.to_html(point)),
                simulate_double_break: options.simulate_double_break,
            },
        )
    }
}
