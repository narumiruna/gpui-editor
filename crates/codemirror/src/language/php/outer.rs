use std::ops::Range;

use crate::Text;

use super::{
    super::{
        HighlightSpan, IndentOptions, StreamSettings,
        html::{HtmlDocument, HtmlNodeKind, HtmlOptions},
    },
    PhpDocument,
};

#[derive(Debug)]
struct Segment {
    source: Range<usize>,
    projected: Range<usize>,
}

/// A native HTML document parsed from the PHP tree's `Text` nodes in source
/// order. It can join markup across PHP gaps, but does not mount any nodes in
/// the PHP tree. HTML offsets refer to the projected text.
#[derive(Debug)]
pub struct PhpHtml {
    document: HtmlDocument,
    segments: Vec<Segment>,
}

impl PhpHtml {
    pub(super) fn parse(php: &PhpDocument) -> Option<Self> {
        let mut text = String::new();
        let mut segments = Vec::new();
        for node in php.nodes().filter(|node| node.name() == "Text") {
            let source = node.range();
            if source.is_empty() {
                continue;
            }
            let start = text.len();
            text.push_str(php.text().slice(source.clone())?);
            segments.push(Segment {
                source,
                projected: start..text.len(),
            });
        }
        if segments.is_empty() {
            return None;
        }
        let document = HtmlDocument::parse_with_options(
            Text::new(text),
            HtmlOptions {
                match_closing_tags: false,
                ..HtmlOptions::default()
            },
        );
        Some(Self { document, segments })
    }

    pub fn document(&self) -> &HtmlDocument {
        &self.document
    }

    /// Map a source byte position inside template text into the HTML document.
    /// A position at the end of a segment or in PHP code does not map.
    pub fn projected_offset(&self, source: usize) -> Option<usize> {
        let index = self
            .segments
            .partition_point(|segment| segment.source.end <= source);
        let segment = self.segments.get(index)?;
        let offset = segment.projected.start + source.checked_sub(segment.source.start)?;
        self.document
            .text()
            .as_str()
            .is_char_boundary(offset)
            .then_some(offset)
    }

    /// Map a valid HTML byte range into its source ranges. A node can span one
    /// or more PHP gaps. The returned ranges include no PHP bytes.
    pub fn source_ranges(&self, projected: Range<usize>) -> Option<Vec<Range<usize>>> {
        self.document.text().slice(projected.clone())?;
        if projected.is_empty() {
            return Some(Vec::new());
        }
        let first = self
            .segments
            .partition_point(|s| s.projected.end <= projected.start);
        let ranges = self.segments[first..]
            .iter()
            .take_while(|s| s.projected.start < projected.end)
            .map(|s| {
                let base = s.projected.start;
                let start = s.source.start + (projected.start.max(base) - base);
                let end = s.source.start + (projected.end.min(s.projected.end) - base);
                start..end
            });
        Some(ranges.collect())
    }

    pub(super) fn highlights<'a>(
        &'a self,
        range: &Range<usize>,
        spans: &mut Vec<HighlightSpan<'a>>,
    ) {
        let first = self
            .segments
            .partition_point(|s| s.source.end <= range.start);
        let end = self
            .segments
            .partition_point(|s| s.source.start < range.end);
        if first == end {
            return;
        }
        let segments = &self.segments[first..end];
        let start =
            segments[0].projected.start + range.start.saturating_sub(segments[0].source.start);
        let last = &segments[segments.len() - 1];
        let end = last.projected.end - last.source.end.saturating_sub(range.end);
        let Some(outer) = self.document.highlights(start..end) else {
            return;
        };
        for span in outer {
            if let Some(ranges) = self.source_ranges(span.range) {
                spans.extend(ranges.into_iter().map(|range| HighlightSpan {
                    range,
                    tags: span.tags,
                }));
            }
        }
    }

    pub(super) fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        let first = self.segments.get(
            self.segments
                .partition_point(|s| s.source.end <= line_start),
        )?;
        let end = self
            .segments
            .partition_point(|s| s.source.start <= line_end);
        let last = self.segments.get(end.checked_sub(1)?)?;
        let start = first.projected.start + line_start.saturating_sub(first.source.start);
        let end = last.projected.end - last.source.end.saturating_sub(line_end);
        let ranges = self.source_ranges(self.document.foldable(start, end)?)?;
        let range = ranges.first()?.start..ranges.last()?.end;
        ((line_start..=line_end).contains(&range.start) && range.end > line_end).then_some(range)
    }

    pub(super) fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<(usize, Option<usize>)> {
        let projected = self.projected_offset(position)?;
        let mut node = self.document.root();
        let mut start = 0;
        while let Some(child) = node.children().find(|n| {
            let range = n.range();
            range.start < projected && projected < range.end
        }) {
            node = child;
            if matches!(
                node.kind(),
                HtmlNodeKind::Element
                    | HtmlNodeKind::OpenTag
                    | HtmlNodeKind::CloseTag
                    | HtmlNodeKind::SelfClosingTag
            ) {
                start = node.range().start;
            }
        }
        let override_indent = |at| options.override_indentation?(self.source_offset(at)?);
        let columns = self.document.indentation(
            projected,
            settings,
            IndentOptions {
                override_indentation: options
                    .override_indentation
                    .map(|_| &override_indent as &dyn Fn(usize) -> Option<usize>),
                simulate_break: options
                    .simulate_break
                    .and_then(|pos| self.projected_offset(pos)),
                simulate_double_break: options.simulate_double_break,
            },
        );
        Some((self.source_offset(start)?, columns))
    }

    fn source_offset(&self, projected: usize) -> Option<usize> {
        let index = self
            .segments
            .partition_point(|s| s.projected.end <= projected);
        let segment = self.segments.get(index)?;
        Some(segment.source.start + projected - segment.projected.start)
    }
}
