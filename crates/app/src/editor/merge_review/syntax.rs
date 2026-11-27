use std::{ops::Range, sync::Arc};

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, StyledText, TextAlign, Window, point, px, rgba, size,
};
use gpui_editor_codemirror::{
    EditorState, Text,
    language::{
        HighlightContext, HighlightTag, StreamSettings, highlighting_for_context,
        markdown::{
            MarkdownDocument, MarkdownNodeKind, MarkdownParserConfig, configured_markdown_parser,
        },
    },
    theme_one_dark::highlight_configuration,
};

use crate::{
    document::{BlockPreview, Markdown, SectionKind},
    editor::source_fence::fence_parser_in,
};

struct SyntaxSpan {
    range: Range<usize>,
    tags: Vec<HighlightTag>,
    scope: Option<HighlightContext>,
}

#[derive(Default)]
pub(super) struct ReviewSyntax {
    snapshot: Option<SyntaxSnapshot>,
}

struct SyntaxSnapshot {
    parser: Option<Arc<MarkdownParserConfig>>,
    settings: StreamSettings,
    spans: Vec<SyntaxSpan>,
}

type ColorRun = (Range<usize>, Hsla);

impl ReviewSyntax {
    /// The review owns one cache per immutable side. Theme-only changes do not change syntax.
    pub(super) fn prepare(&mut self, source: &Text, state: &EditorState) {
        let parser = configured_markdown_parser(state);
        let settings = StreamSettings {
            tab_size: state.tab_size(),
            ..Default::default()
        };
        if self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.settings == settings
                && match (&snapshot.parser, &parser) {
                    (Some(before), Some(after)) => Arc::ptr_eq(before, after),
                    (None, None) => true,
                    _ => false,
                }
        }) {
            return;
        }
        let spans = fence_spans(source, state, parser.as_deref(), settings);
        self.snapshot = Some(SyntaxSnapshot {
            parser,
            settings,
            spans,
        });
    }

    pub(super) fn colors(&self, line: &Range<usize>, state: &EditorState) -> Vec<ColorRun> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let spans = &snapshot.spans;
        let (layers, theme) = highlight_configuration(state);
        let first = spans.partition_point(|span| span.range.end <= line.start);
        spans[first..]
            .iter()
            .take_while(|span| span.range.start < line.end)
            .filter_map(|span| {
                let color =
                    highlighting_for_context(&layers, &span.tags, span.scope, theme).foreground?;
                let start = span.range.start.max(line.start) - line.start;
                let end = span.range.end.min(line.end) - line.start;
                (start < end).then_some((start..end, rgba(color.rgba32()).into()))
            })
            .collect()
    }
}

fn fence_spans(
    source: &Text,
    state: &EditorState,
    parser: Option<&MarkdownParserConfig>,
    settings: StreamSettings,
) -> Vec<SyntaxSpan> {
    use MarkdownNodeKind::{CodeInfo, CodeText, FencedCode};

    // The app recognizes other blocks such as YAML front matter. Use its fence
    // eligibility, then use native Markdown ranges to exclude container prefixes.
    let fences: Vec<_> = Markdown::new(source.as_str())
        .sections()
        .into_iter()
        .filter(|section| {
            section.kind == SectionKind::DraftCode
                || matches!(section.preview, Some(BlockPreview::Code { .. }))
        })
        .map(|section| section.source)
        .collect();
    if fences.is_empty() {
        return Vec::new();
    }
    let markdown = MarkdownDocument::parse_with_dialect(
        source.clone(),
        parser.map_or_else(Default::default, |parser| parser.base),
    );
    let mut spans = Vec::new();
    for node in markdown.nodes().filter(|node| node.kind() == FencedCode) {
        let Ok(fence) = fences.binary_search_by_key(&node.range().start, |range| range.start)
        else {
            continue;
        };
        let info = node
            .children()
            .find(|child| child.kind() == CodeInfo)
            .and_then(|child| source.slice(child.range()))
            .unwrap_or("");
        let Some(parser) = fence_parser_in(state, info) else {
            continue;
        };
        let mut body = String::new();
        let mut segments = Vec::new();
        for range in node
            .children()
            .filter(|child| child.kind() == CodeText)
            .map(|child| child.range())
        {
            if range.is_empty() || range.end > fences[fence].end {
                continue;
            }
            let start = body.len();
            body.push_str(
                source
                    .slice(range.clone())
                    .expect("valid native code range"),
            );
            segments.push((start..body.len(), range.start));
        }
        if body.is_empty() {
            continue;
        }
        let Ok(document) = parser.parse(Text::new(body), settings) else {
            continue;
        };
        for span in document
            .highlights(0..document.text().len())
            .into_iter()
            .flatten()
        {
            let first = segments.partition_point(|(local, _)| local.end <= span.range.start);
            let scope = document.highlight_context(&span.range);
            for (local, offset) in segments[first..]
                .iter()
                .take_while(|(local, _)| local.start < span.range.end)
            {
                let start = offset + span.range.start.saturating_sub(local.start);
                let end = offset + span.range.end.min(local.end) - local.start;
                spans.push(SyntaxSpan {
                    range: start..end,
                    tags: span.tags.to_vec(),
                    scope,
                });
            }
        }
    }
    spans
}

/// Use the normal unstyled text layout. GPUI can split font runs at color
/// boundaries, and macOS shaping can change ligatures or joining at those splits.
/// The syntax ranges only select a foreground for each already shaped glyph.
pub(super) struct ReviewText {
    text: StyledText,
    colors: Vec<ColorRun>,
    #[cfg(test)]
    probe: Option<(gpui_editor_codemirror::merge::Side, usize)>,
}

impl ReviewText {
    pub(super) fn new(text: &str, colors: Vec<ColorRun>) -> Self {
        Self {
            text: StyledText::new(text.to_owned()),
            colors,
            #[cfg(test)]
            probe: None,
        }
    }

    #[cfg(test)]
    pub(super) fn probe(mut self, side: gpui_editor_codemirror::merge::Side, line: usize) -> Self {
        self.probe = Some((side, line));
        self
    }

    fn color_at(&self, byte: usize, base: Hsla) -> Hsla {
        let index = self.colors.partition_point(|(range, _)| range.end <= byte);
        self.colors
            .get(index)
            .filter(|(range, _)| range.contains(&byte))
            .map_or(base, |(_, color)| *color)
    }
}

impl IntoElement for ReviewText {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ReviewText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.prepaint(id, inspector, bounds, state, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.text.layout();
        let style = window.text_style();
        #[cfg(test)]
        if let Some((side, number)) = self.probe {
            super::tests::record_paint(side, number, layout, |byte| {
                self.color_at(byte, style.color)
            });
        }
        if self.colors.is_empty() {
            self.text
                .paint(id, inspector, bounds, state, prepaint, window, cx);
            return;
        }
        let Some(shaped) = layout.line_layout_for_index(0) else {
            return;
        };
        debug_assert!(shaped.wrap_boundaries().is_empty());
        let line = &shaped.unwrapped_layout;
        let line_height = layout.line_height();
        let x = bounds.left()
            + match style.text_align {
                TextAlign::Left => px(0.),
                TextAlign::Center => (bounds.size.width - line.width) / 2.,
                TextAlign::Right => bounds.size.width - line.width,
            };
        let baseline = bounds.top() + (line_height - line.ascent - line.descent) / 2. + line.ascent;
        let clip = window.content_mask().bounds;
        window.paint_layer(
            Bounds::new(bounds.origin, size(line.width, line_height)),
            |window| {
                for run in &line.runs {
                    let glyph_size = cx
                        .text_system()
                        .bounding_box(run.font_id, line.font_size)
                        .size;
                    for glyph in &run.glyphs {
                        let x = x + glyph.position.x;
                        if !Bounds::new(point(x, bounds.top()), glyph_size).intersects(&clip) {
                            continue;
                        }
                        let at = point(x, baseline);
                        let result = if glyph.is_emoji {
                            window.paint_emoji(at, run.font_id, glyph.id, line.font_size)
                        } else {
                            window.paint_glyph(
                                at,
                                run.font_id,
                                glyph.id,
                                line.font_size,
                                self.color_at(glyph.index, style.color),
                            )
                        };
                        if result.is_err() {
                            return;
                        }
                    }
                }
            },
        );
    }
}
