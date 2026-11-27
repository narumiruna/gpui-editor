use std::{ops::Range, sync::LazyLock};

use super::{
    super::{
        HighlightTag,
        html::{HtmlNode, HtmlNodeKind},
        lezer::Parser,
    },
    HighlightSpan, HtmlDocument, JavascriptDocument, JavascriptOptions, Text, VueExpression,
    VueExpressionKind, tables,
};

mod recovery;

#[derive(Debug, Default)]
pub(super) struct Template {
    overlays: Vec<Range<usize>>,
    markup: Vec<HighlightSpan<'static>>,
    pub(super) expressions: Vec<VueExpression>,
}

impl Template {
    pub(super) fn parse(host: &HtmlDocument) -> Self {
        let mut template = Self::default();
        let source = host.text().as_str();
        for node in host.nodes() {
            let range = node.range();
            let fragment = &source[range.clone()];
            let directive = match node.kind() {
                HtmlNodeKind::Text if fragment.contains("{{") => false,
                HtmlNodeKind::Attribute
                    if fragment.starts_with(['@', ':']) || fragment.starts_with("v-") =>
                {
                    true
                }
                _ => continue,
            };
            template.overlays.push(range.clone());
            let parser = if directive {
                tables::attribute()
            } else {
                tables::content()
            };
            if !template.parse_fragment(parser, fragment, range.start) {
                if directive {
                    recovery::attribute(&mut template, source, node);
                } else {
                    recovery::content(&mut template, fragment, range.start);
                }
            }
        }
        template
    }

    fn parse_fragment(&mut self, parser: &Parser, source: &str, offset: usize) -> bool {
        let Ok(tree) = parser.parse(source) else {
            return false;
        };
        let mut pending = vec![tree.root()];
        while let Some(node) = pending.pop() {
            let kind = node.name();
            let local = node.range();
            let range = offset + local.start..offset + local.end;
            match kind {
                "InterpolationContent" | "AttributeScript" => {
                    let context = if kind == "InterpolationContent" {
                        VueExpressionKind::Interpolation
                    } else {
                        VueExpressionKind::Directive
                    };
                    self.expression(&source[local], range.start, context);
                }
                "ScriptAttributeValue" => {
                    if node.children().len() == 0 {
                        self.push(range, Tag::Value);
                    } else {
                        self.push(range.start..range.start + 1, Tag::Value);
                        pending.extend(node.children().rev());
                        self.push(range.end - 1..range.end, Tag::Value);
                    }
                }
                "Text" => self.push(range, Tag::Content),
                "}}" => self.push(range, Tag::Brace),
                "VueAttributeName" => self.push(range, Tag::Keyword),
                ":" | "@" => self.push(range, Tag::Punctuation),
                "Identifier" => self.push(range, Tag::Variable),
                "AttributeName" => self.push(range, Tag::Name),
                "Is" => self.push(range, Tag::Operator),
                "AttributeValue" => {
                    let mut start = range.start;
                    for entity in node.children().filter(|child| child.name() == "Entity") {
                        let local = entity.range();
                        self.push(start..offset + local.start, Tag::Value);
                        self.push(offset + local.start..offset + local.end, Tag::Character);
                        start = offset + local.end;
                    }
                    self.push(start..range.end, Tag::Value);
                }
                "Entity" => self.push(range, Tag::Character),
                _ => pending.extend(node.children().rev()),
            }
        }
        true
    }

    pub(super) fn expression_at(&self, position: usize) -> Option<&VueExpression> {
        let index = self
            .expressions
            .partition_point(|expression| expression.range.end <= position);
        self.expressions
            .get(index)
            .filter(|expression| expression.range.start < position)
    }

    fn expression(&mut self, source: &str, start: usize, kind: VueExpressionKind) {
        if !source.is_empty() {
            self.expressions.push(VueExpression {
                range: start..start + source.len(),
                kind,
                document: JavascriptDocument::parse_expression(
                    Text::new(source),
                    JavascriptOptions::default(),
                ),
            });
        }
    }

    fn push(&mut self, range: Range<usize>, tag: Tag) {
        if !range.is_empty() {
            static TAGS: LazyLock<[HighlightTag; 9]> = LazyLock::new(|| {
                [
                    "content",
                    "brace",
                    "keyword",
                    "punctuation",
                    "variableName",
                    "attributeName",
                    "definitionOperator",
                    "attributeValue",
                    "character",
                ]
                .map(HighlightTag::new)
            });
            self.markup.push(HighlightSpan {
                range,
                tags: std::slice::from_ref(&TAGS[tag as usize]),
            });
        }
    }

    pub(super) fn highlights<'a>(
        &'a self,
        range: Range<usize>,
        host: impl Iterator<Item = HighlightSpan<'a>>,
    ) -> Vec<HighlightSpan<'a>> {
        if range.is_empty() {
            return Vec::new();
        }
        let mut spans = Vec::new();
        let index = self
            .overlays
            .partition_point(|span| span.end <= range.start);
        let mut overlays = self.overlays[index..].iter().peekable();
        for span in host {
            let mut start = span.range.start;
            let end = span.range.end;
            while let Some(overlay) = overlays.peek().filter(|overlay| overlay.start < end) {
                if start < overlay.start {
                    spans.push(HighlightSpan {
                        range: start..overlay.start,
                        tags: span.tags,
                    });
                }
                start = start.max(overlay.end);
                if overlay.end > end {
                    break;
                }
                overlays.next();
            }
            if start < end {
                spans.push(HighlightSpan {
                    range: start..end,
                    tags: span.tags,
                });
            }
        }
        let index = self
            .markup
            .partition_point(|span| span.range.end <= range.start);
        spans.extend(
            self.markup[index..]
                .iter()
                .take_while(|span| span.range.start < range.end)
                .map(|span| HighlightSpan {
                    range: span.range.start.max(range.start)..span.range.end.min(range.end),
                    tags: span.tags,
                }),
        );
        let index = self
            .expressions
            .partition_point(|expression| expression.range.end <= range.start);
        for expr in &self.expressions[index..] {
            let offset = expr.range.start;
            if offset >= range.end {
                break;
            }
            let query = range.start.saturating_sub(offset)..range.end.min(expr.range.end) - offset;
            spans.extend(
                expr.document
                    .highlights(query)
                    .into_iter()
                    .flatten()
                    .map(|span| HighlightSpan {
                        range: offset + span.range.start..offset + span.range.end,
                        tags: span.tags,
                    }),
            );
        }
        spans.sort_unstable_by_key(|span| span.range.start);
        spans
    }
}

#[derive(Clone, Copy)]
enum Tag {
    Content,
    Brace,
    Keyword,
    Punctuation,
    Variable,
    Name,
    Operator,
    Value,
    Character,
}
