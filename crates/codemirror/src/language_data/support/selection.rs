use std::ops::Range;

use crate::{
    Assoc, EditorSelection,
    commands::logical::{ParentQuery, ParentResolution, choose_parent, resolve_parent},
};

use super::{
    NativeSyntax, SupportedDocument,
    projection::HtmlProjection,
    route::{Grammar, Site},
};

impl SupportedDocument {
    /// Expand a source selection to the next larger syntax node. A mounted
    /// parser takes precedence over its host. Do not cross a source gap when
    /// mapping a node in a projected grammar.
    pub fn parent_selection(&self, span: Range<usize>) -> Option<Range<usize>> {
        if !self.text().is_valid_range(span.clone()) {
            return None;
        }
        choose_parent(&span, |side| self.resolve_selection(&span, side))
            .or_else(|| self.template_host_parent(&span))
    }

    /// Expand each range independently and normalize overlapping results.
    /// Return `None` if the selection cannot expand, or uses invalid offsets.
    pub fn select_parent_syntax(&self, selection: &EditorSelection) -> Option<EditorSelection> {
        selection
            .check(|at| self.text().is_char_boundary(at))
            .ok()?;
        let ranges = selection
            .ranges()
            .iter()
            .map(|&range| {
                self.parent_selection(range.range())
                    .map_or(range, |parent| {
                        EditorSelection::undirectional_range(parent.start, parent.end)
                    })
            })
            .collect();
        let expanded = EditorSelection::new(ranges, selection.main_index()).ok()?;
        (&expanded != selection).then_some(expanded)
    }

    fn template_host_parent(&self, span: &Range<usize>) -> Option<Range<usize>> {
        let html = HtmlProjection::new(&self.syntax)?;
        let at = html.project_boundary(span.start);
        choose_parent(span, |side| {
            resolve_parent(
                html.document.root(),
                ParentQuery {
                    selection: span,
                    at,
                    side,
                    include_root: false,
                },
                |node| node.range(),
                |node| node.children(),
                |range| html.selection_bounds(range),
            )
        })
    }

    fn resolve_selection(&self, span: &Range<usize>, side: Assoc) -> ParentResolution {
        let mut stack = Vec::new();
        let Some(site) = self.site_with_hosts(span.start, side, |host| stack.push(host)) else {
            return ParentResolution::default();
        };
        stack.push(site);
        let mut resolution = ParentResolution::default();
        for (index, site) in stack.into_iter().enumerate().rev() {
            let inner = site_selection(&site, span, side, index != 0);
            resolution.leaf = resolution.leaf.or(inner.leaf);
            resolution.larger = inner.larger;
            if resolution.larger.is_some() {
                break;
            }
        }
        resolution
    }
}

fn site_selection(
    site: &Site<'_>,
    selection: &Range<usize>,
    side: Assoc,
    include_root: bool,
) -> ParentResolution {
    let query = ParentQuery {
        selection,
        at: site.position.at,
        side,
        include_root,
    };
    let source = |range: Range<usize>| {
        Some(site.position.source(range.start)?..site.position.source(range.end)?)
    };
    macro_rules! tree {
        ($doc:expr) => {
            resolve_parent(
                $doc.root(),
                query,
                |node| node.range(),
                |node| node.children(),
                source,
            )
        };
    }
    let stream = |span: Option<Range<usize>>| {
        let leaf = span.and_then(source);
        let larger = leaf
            .as_ref()
            .filter(|span| {
                span.start <= selection.start && selection.end <= span.end && **span != *selection
            })
            .cloned();
        ParentResolution { leaf, larger }
    };
    match site.grammar {
        Grammar::Html(doc) => tree!(doc),
        Grammar::Css(doc) => tree!(doc),
        Grammar::Javascript(doc, _) => tree!(doc),
        Grammar::Native(syntax) => match syntax {
            NativeSyntax::Angular(doc) => tree!(doc.html()),
            NativeSyntax::Asn1(doc) => stream(doc.token_range_at(query.at, side)),
            NativeSyntax::Cpp(doc) => tree!(doc),
            NativeSyntax::Css(doc) => tree!(doc),
            NativeSyntax::Go(doc) => tree!(doc),
            NativeSyntax::Html(doc) => tree!(doc),
            NativeSyntax::Java(doc) => tree!(doc),
            NativeSyntax::Javascript(doc) => tree!(doc),
            NativeSyntax::Jinja(doc) => tree!(doc),
            NativeSyntax::Json(doc) => tree!(doc),
            NativeSyntax::Legacy(doc) => stream(doc.token_range_at(query.at, side)),
            NativeSyntax::Less(doc) => tree!(doc),
            NativeSyntax::LezerGrammar(doc) => tree!(doc),
            NativeSyntax::Liquid(doc) => tree!(doc),
            NativeSyntax::Markdown(doc) => tree!(doc),
            NativeSyntax::Php(doc) => tree!(doc),
            NativeSyntax::Python(doc) => tree!(doc),
            NativeSyntax::Rust(doc) => tree!(doc),
            NativeSyntax::Sass(doc) => tree!(doc),
            NativeSyntax::Sql(doc) => tree!(doc),
            NativeSyntax::Vue(doc) => tree!(doc.host()),
            NativeSyntax::Wast(doc) => tree!(doc),
            NativeSyntax::Xml(doc) => tree!(doc),
            NativeSyntax::Yaml(doc) => resolve_parent(
                doc.root(),
                query,
                |node| node.range(),
                |node| node.children(),
                source,
            ),
        },
    }
}
