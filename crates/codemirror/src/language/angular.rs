//! Native Angular template support from `@codemirror/lang-angular` 0.1.4.
//! See `docs/licenses/codemirror-lang-angular.txt`. Ranges use UTF-8 bytes.
//!
//! Angular regions use the published grammar and mount JavaScript expressions
//! and statements. The outer HTML parser supplies existing HTML services.
//! Malformed regions use local repair or bounded LR recovery for a missing
//! binding name closer.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentContext, IndentOptions, LanguageData, StreamSettings,
    html::{self, HtmlDocument, HtmlMount, HtmlNesting, HtmlNodeKind, HtmlOptions},
    javascript::{JavascriptDocument, JavascriptOptions},
    lezer::ReuseStats,
};

mod highlight;
mod parser;
mod tables;

pub const NAME: &str = "angular";
pub const CLOSE_BRACKETS: [char; 3] = ['[', '{', '"'];

pub fn language_data() -> LanguageData {
    html::language_data()
        .with("closeBrackets", CLOSE_BRACKETS)
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

/// Whether the Angular or base HTML indentation rule applies to a line prefix.
pub fn indent_on_input(prefix: &str) -> bool {
    template_indent_on_input(prefix) || html::indent_on_input(prefix)
}

pub(crate) fn template_indent_on_input(prefix: &str) -> bool {
    matches!(trim_space(prefix), "}" | "]")
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// The two roots used by Angular inside an HTML document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AngularRegionKind {
    Content,
    Attribute,
}

#[derive(Debug)]
struct Node {
    name: &'static str,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed Angular node at an absolute template byte range.
#[derive(Clone, Copy, Debug)]
pub struct AngularNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> AngularNode<'a> {
    pub fn name(self) -> &'static str {
        self.nodes[self.id].name
    }

    pub fn range(self) -> Range<usize> {
        self.nodes[self.id].range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.nodes[self.id].parent.map(|id| Self { id, ..self })
    }

    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.nodes[self.id]
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }
}

#[derive(Debug)]
pub struct AngularScript {
    range: Range<usize>,
    document: JavascriptDocument,
}

impl AngularScript {
    /// The embedded source's range in the template, excluding braces or quotes.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// The script's syntax offsets start at zero in the embedded source.
    pub fn document(&self) -> &JavascriptDocument {
        &self.document
    }
}

#[derive(Debug)]
pub struct AngularRegion {
    kind: AngularRegionKind,
    nodes: Vec<Node>,
    scripts: Vec<AngularScript>,
    recovered: bool,
}

impl AngularRegion {
    pub fn kind(&self) -> AngularRegionKind {
        self.kind
    }

    pub fn root(&self) -> AngularNode<'_> {
        AngularNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    pub fn nodes(&self) -> impl ExactSizeIterator<Item = AngularNode<'_>> {
        (0..self.nodes.len()).map(|id| AngularNode { id, ..self.root() })
    }

    pub fn scripts(&self) -> &[AngularScript] {
        &self.scripts
    }

    /// Whether the template grammar needed local recovery in this region.
    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    fn replaces_html(&self) -> bool {
        self.nodes.get(1).is_some_and(|node| node.name != "⚠")
    }
}

/// A template with an HTML host and sorted Angular regions.
#[derive(Debug)]
pub struct AngularDocument {
    html: HtmlDocument,
    regions: Vec<AngularRegion>,
}

impl AngularDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(
            text,
            HtmlOptions {
                self_closing_tags: true,
                ..HtmlOptions::default()
            },
        )
    }

    /// Configure the base HTML provider. Without options, Angular allows XML-style self-closing tags.
    pub fn parse_with_options(text: Text, options: HtmlOptions) -> Self {
        Self::parse_with_nesting(text, options, HtmlNesting::default())
    }

    /// Parse with extra HTML body and attribute routes. An HTML body mount
    /// hides Angular syntax inside it. An Angular attribute that replaces the
    /// HTML attribute takes precedence over its HTML value mount.
    pub fn parse_with_nesting(text: Text, options: HtmlOptions, nesting: HtmlNesting<'_>) -> Self {
        Self::from_html(HtmlDocument::parse_with_nesting(text, options, nesting))
    }

    /// Apply byte edits with the same HTML dialect and stock routes. To keep
    /// custom routes, use [`Self::reparse_with_nesting`].
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        self.reparse_with_nesting(changes, HtmlNesting::default())
    }

    /// Apply byte edits and rebuild mounts with the supplied routes. Supply
    /// custom routes on each call. Reuse strict host checkpoints where possible.
    pub fn reparse_with_nesting(
        &self,
        changes: &ChangeSet,
        nesting: HtmlNesting<'_>,
    ) -> Result<Self, EditError> {
        Ok(Self::from_html(
            self.html.reparse_with_nesting(changes, nesting)?,
        ))
    }

    /// Strict HTML host checkpoint reuse. This does not count mounted trees.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.html.reuse_stats()
    }

    fn from_html(html: HtmlDocument) -> Self {
        let source = html.text().as_str();
        let regions = html
            .nodes()
            .filter_map(|node| {
                let range = node.range();
                let body = source.get(range.clone()).filter(|body| !body.is_empty())?;
                let kind = match node.kind() {
                    HtmlNodeKind::Text => AngularRegionKind::Content,
                    HtmlNodeKind::Attribute
                        if body.starts_with(['*', '#', '(', '[']) || body.contains("{{") =>
                    {
                        AngularRegionKind::Attribute
                    }
                    _ => return None,
                };
                // A body mount covers the start of the node. A value mount
                // starts after the attribute name, so it cannot hide the attribute.
                if mount_at_or_after(&html, range.start)
                    .is_some_and(|mount| mount.range().contains(&range.start))
                {
                    return None;
                }
                let (nodes, recovered) = parser::parse(source, range, kind);
                let scripts = nodes
                    .iter()
                    .filter_map(|node| {
                        let statements = match node.name {
                            "InterpolationContent" => false,
                            "AttributeInterpolation" => node
                                .parent
                                .is_some_and(|id| nodes[id].name == "StatementAttributeValue"),
                            _ => return None,
                        };
                        let range = node.range.clone();
                        let input = html.text().slice_owned(range.clone())?;
                        let document = if statements {
                            JavascriptDocument::parse(input)
                        } else {
                            JavascriptDocument::parse_expression(
                                input,
                                JavascriptOptions::default(),
                            )
                        };
                        Some(AngularScript { range, document })
                    })
                    .collect();
                Some(AngularRegion {
                    kind,
                    nodes,
                    scripts,
                    recovered,
                })
            })
            .collect();
        Self { html, regions }
    }

    pub fn text(&self) -> &Text {
        self.html.text()
    }

    /// The base HTML document, before Angular replaces its text and attributes.
    /// Use [`Self::mounts`] for the HTML mounts that remain active.
    pub fn html(&self) -> &HtmlDocument {
        &self.html
    }

    /// The active HTML mounts in source order. Angular JavaScript mounts are
    /// available on their respective [`AngularRegion`]s.
    pub fn mounts(&self) -> impl Iterator<Item = &HtmlMount> {
        self.html
            .mounts()
            .iter()
            .filter(|mount| !self.replaces_mount(mount))
    }

    pub fn regions(&self) -> &[AngularRegion] {
        &self.regions
    }

    /// Return clipped HTML, Angular and JavaScript syntax highlights.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text().slice(range.clone())?;
        Some(highlight::collect(self, range).into_iter())
    }

    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text().slice(line_start..line_end)?;
        // Still allow an outer HTML fold, but do not ask the replaced mount.
        let host_end = self.replaced_value_start(line_end).unwrap_or(line_end);
        self.html
            .foldable(line_start, host_end)
            .filter(|fold| fold.end > line_end)
            .or_else(|| {
                let script = self.script_at(line_end)?;
                let offset = script.range.start;
                let fold = script
                    .document
                    .foldable(line_start.saturating_sub(offset), line_end - offset)?;
                Some(offset + fold.start..offset + fold.end)
            })
    }

    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let cx = IndentContext::new(self.text(), position, settings, options)?;
        let fallback = || {
            self.interpolation_indent(position, &cx, options)
                .or_else(|| {
                    // At the value start, HTML uses the opening tag's rule,
                    // not the replaced attribute mount.
                    let at = self.replaced_value_start(position).unwrap_or(position);
                    self.html.indentation(at, settings, options)
                })
        };
        let Some(script) = self.script_at(position) else {
            return fallback();
        };
        let offset = script.range.start;
        let from_host = |local| {
            if local == 0 {
                cx.line_indent(offset, Assoc::After)
            } else {
                options
                    .override_indentation
                    .and_then(|get| get(offset + local))
            }
        };
        let local = IndentOptions {
            override_indentation: Some(&from_host),
            simulate_break: options
                .simulate_break
                .filter(|at| script.range.contains(at))
                .map(|at| at - offset),
            simulate_double_break: options.simulate_double_break,
        };
        script
            .document
            .embedded_indentation(position - offset, settings, local, fallback)
    }

    fn interpolation_indent(
        &self,
        position: usize,
        cx: &IndentContext<'_>,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let node = self.region_at(position)?.nodes().find(|node| {
            let range = node.range();
            node.name() == "Interpolation" && range.start < position && range.end > position
        })?;
        let mut children = node.children();
        let open = children.next()?.range();
        let close = children.next_back().filter(|node| node.name() == "}}");
        let after = cx.text_after_pos(position, Assoc::After)?;
        let closed = close.is_some_and(|node| {
            node.range().start == position + after.len() - trim_space(after).len()
        }) && !(options.simulate_double_break
            && cx.simulated_break() == Some(position));
        let line = self.text().line_at(open.start)?.range;
        let end = cx
            .simulated_break()
            .filter(|&at| at > line.start)
            .map_or(line.end, |at| line.end.min(at));
        if children
            .next()
            .is_some_and(|child| child.range().start < end)
        {
            let spaces = self
                .text()
                .slice(open.end..line.end)?
                .bytes()
                .take_while(|&ch| ch == b' ')
                .count();
            let column = if closed {
                open.start
            } else {
                open.end + spaces
            };
            return cx
                .column(column, Assoc::After)
                .map(|column| column.max(0) as usize);
        }
        let base = cx.line_indent(open.start, Assoc::After)?;
        Some(base.saturating_add(if closed { 0 } else { cx.unit() }))
    }

    fn region_at(&self, position: usize) -> Option<&AngularRegion> {
        let i = self
            .regions
            .partition_point(|region| region.root().range().end <= position);
        self.regions
            .get(i)
            .filter(|region| region.root().range().contains(&position))
    }

    fn replaces_mount(&self, mount: &HtmlMount) -> bool {
        self.region_at(mount.range().start).is_some_and(|region| {
            region.kind() == AngularRegionKind::Attribute && region.replaces_html()
        })
    }

    fn replaced_value_start(&self, position: usize) -> Option<usize> {
        let mount = mount_at_or_after(&self.html, position)?;
        (mount.range().start < position && self.replaces_mount(mount)).then(|| mount.range().start)
    }

    fn script_at(&self, position: usize) -> Option<&AngularScript> {
        let region = self.region_at(position)?;
        let at = region
            .scripts
            .partition_point(|script| script.range.end <= position);
        region
            .scripts
            .get(at)
            .filter(|script| script.range.start < position)
    }
}

fn mount_at_or_after(html: &HtmlDocument, position: usize) -> Option<&HtmlMount> {
    let mounts = html.mounts();
    let i = mounts.partition_point(|mount| mount.range().end <= position);
    mounts.get(i)
}
