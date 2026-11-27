use std::ops::Range;

use crate::{Assoc, Text};

use super::{
    super::{
        IndentContext,
        html::{HtmlDocument, HtmlNodeKind, HtmlOptions},
    },
    HighlightSpan, IndentOptions, Node, StreamSettings,
};

#[derive(Debug)]
struct Segment {
    source: Range<usize>,
    projected: Range<usize>,
}

/// The native HTML projection of the Jinja `Text` and `RawText` nodes.
/// HTML can span template gaps. Its nodes use offsets in the projected text.
#[derive(Debug)]
pub struct JinjaHtml {
    document: HtmlDocument,
    segments: Vec<Segment>,
}

impl JinjaHtml {
    pub(super) fn parse(text: &Text, nodes: &[Node], options: HtmlOptions) -> Option<Self> {
        let mut projected = String::new();
        let mut segments = Vec::<Segment>::new();
        for node in nodes
            .iter()
            .filter(|n| matches!(n.name(), "Text" | "RawText"))
        {
            let source = node.range.clone();
            if source.is_empty() {
                continue;
            }
            let start = projected.len();
            projected.push_str(text.slice(source.clone())?);
            if let Some(last) = segments
                .last_mut()
                .filter(|last| last.source.end == source.start)
            {
                last.source.end = source.end;
                last.projected.end = projected.len();
            } else {
                segments.push(Segment {
                    source,
                    projected: start..projected.len(),
                });
            }
        }
        if segments.is_empty() {
            return None;
        }
        let document = HtmlDocument::parse_with_options(Text::new(projected), options);
        Some(Self { document, segments })
    }

    pub fn document(&self) -> &HtmlDocument {
        &self.document
    }

    /// Map an original byte position inside HTML to the projected text.
    /// Positions in Jinja gaps or at the end of the projection do not map.
    pub fn projected_offset(&self, source: usize) -> Option<usize> {
        let segment = self
            .segments
            .get(self.segments.partition_point(|s| s.source.end <= source))?;
        let offset = segment.projected.start + source.checked_sub(segment.source.start)?;
        self.document
            .text()
            .as_str()
            .is_char_boundary(offset)
            .then_some(offset)
    }

    /// Map a valid projected range into the original ranges it covers.
    pub fn source_ranges(&self, projected: Range<usize>) -> Option<Vec<Range<usize>>> {
        self.document.text().slice(projected.clone())?;
        let start = self
            .segments
            .partition_point(|s| s.projected.end <= projected.start);
        Some(
            self.segments[start..]
                .iter()
                .take_while(|s| !projected.is_empty() && s.projected.start < projected.end)
                .map(|s| {
                    let base = s.projected.start;
                    let from = s.source.start + (projected.start.max(base) - base);
                    let to = s.source.start + (projected.end.min(s.projected.end) - base);
                    from..to
                })
                .collect(),
        )
    }

    fn project_boundary(&self, source: usize) -> usize {
        let index = self.segments.partition_point(|s| s.source.end <= source);
        self.segments
            .get(index)
            .map_or(self.document.text().len(), |s| {
                s.projected.start + source.saturating_sub(s.source.start)
            })
    }

    fn source_boundary(&self, projected: usize, bias: Assoc) -> usize {
        let index = self.segments.partition_point(|s| {
            s.projected.end < projected || (s.projected.end == projected && bias == Assoc::After)
        });
        self.segments.get(index).map_or_else(
            || {
                self.segments
                    .last()
                    .expect("nonempty projection")
                    .source
                    .end
            },
            |s| s.source.start + (projected - s.projected.start),
        )
    }

    pub(super) fn highlights<'a>(
        &'a self,
        range: &Range<usize>,
        spans: &mut Vec<HighlightSpan<'a>>,
    ) {
        let projected = self.project_boundary(range.start)..self.project_boundary(range.end);
        if let Some(outer) = self.document.highlights(projected) {
            for span in outer {
                if let Some(ranges) = self.source_ranges(span.range) {
                    spans.extend(ranges.into_iter().map(|range| HighlightSpan {
                        range,
                        tags: span.tags,
                    }));
                }
            }
        }
    }

    pub(super) fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        let range = self.document.foldable(
            self.project_boundary(line_start),
            self.project_boundary(line_end),
        )?;
        let start = self.source_boundary(range.start, Assoc::Before);
        let end = self.source_boundary(range.end, Assoc::After);
        ((line_start..=line_end).contains(&start) && line_end < end).then_some(start..end)
    }

    pub(super) fn indentation(
        &self,
        original: &IndentContext<'_>,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<(usize, Option<usize>)> {
        let end = self.segments.last()?.source.end;
        let projected = if original.position() == end {
            self.document.text().len()
        } else {
            self.projected_offset(original.position())?
        };
        let mut node = self.document.root();
        let mut start = None;
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
                start = Some(self.source_boundary(node.range().start, Assoc::After));
            }
        }
        let override_indent =
            |at| original.line_indent(self.source_boundary(at, Assoc::After), Assoc::After);
        let indent = self.document.indentation(
            projected,
            settings,
            IndentOptions {
                override_indentation: Some(&override_indent),
                simulate_break: options
                    .simulate_break
                    .and_then(|pos| self.projected_offset(pos)),
                simulate_double_break: options.simulate_double_break,
            },
        );
        // At the end of the final overlay range, the HTML root can still
        // supply indentation or preserve it on an otherwise empty line.
        let at_end = original.position() == end || indent.is_none();
        Some((
            if at_end {
                original.position()
            } else {
                start.unwrap_or(0)
            },
            indent,
        ))
    }
}
