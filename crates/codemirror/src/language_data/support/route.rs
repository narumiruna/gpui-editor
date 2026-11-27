use std::ops::Range;

use crate::{
    Assoc,
    language::{
        css::CssDocument,
        html::{HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage},
        javascript::{JavascriptDocument, JavascriptOptions},
        line_projection::splits_crlf,
        liquid,
        wast::WastDocument,
    },
};

use super::{NativeLanguage, NativeSyntax, SupportedDocument};

pub(super) enum Grammar<'a> {
    Native(&'a NativeSyntax),
    Html(&'a HtmlDocument),
    Css(&'a CssDocument),
    Javascript(&'a JavascriptDocument, bool),
}

pub(super) struct Site<'a> {
    pub language: NativeLanguage,
    pub grammar: Grammar<'a>,
    pub position: Query,
    pub html_host: HtmlHost,
}

impl<'a> Site<'a> {
    pub fn wast(&self) -> Option<&'a WastDocument> {
        match self.grammar {
            Grammar::Native(NativeSyntax::Wast(doc)) => Some(doc),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum HtmlHost {
    Plain,
    Angular,
    Vue,
    Template,
}

/// The part of the active grammar that maps continuously to the original
/// document. A completion edit must not cross an excluded source gap.
#[derive(Clone)]
pub(super) struct Query {
    pub at: usize,
    side: Assoc,
    local: Range<usize>,
    original: usize,
}

impl Query {
    fn new(at: usize, side: Assoc, length: usize) -> Self {
        Self {
            at,
            side,
            local: 0..length,
            original: 0,
        }
    }

    pub fn source(&self, at: usize) -> Option<usize> {
        (self.local.start <= at && at <= self.local.end)
            .then(|| self.original + (at - self.local.start))
    }

    fn inside(&self, region: &Range<usize>) -> bool {
        match self.side {
            Assoc::Before => region.start < self.at && self.at <= region.end,
            Assoc::After => region.start <= self.at && self.at < region.end,
        }
    }

    fn enter(&self, region: Range<usize>, parsed_start: usize) -> Option<Self> {
        if !self.inside(&region) {
            return None;
        }
        let start = self.local.start.max(region.start);
        let end = self.local.end.min(region.end);
        let original = self.source(start)?;
        let into = |pos| parsed_start + (pos - region.start);
        Some(Self {
            at: into(self.at),
            side: self.side,
            local: into(start)..into(end),
            original,
        })
    }

    fn projected(&self, regions: impl IntoIterator<Item = Range<usize>>) -> Option<Self> {
        let mut parsed_start = 0;
        for region in regions {
            let length = region.len();
            if let Some(result) = self.enter(region, parsed_start) {
                return Some(result);
            }
            parsed_start += length;
        }
        None
    }
}

impl SupportedDocument {
    pub(super) fn markdown_presence_site(&self, position: usize, side: Assoc) -> Option<Site<'_>> {
        let site = self.site(position, side)?;
        let NativeSyntax::Markdown(doc) = &self.syntax else {
            return Some(site);
        };
        if splits_crlf(doc.text().as_str(), position) {
            return None;
        }
        // At a widened endpoint, report the fence root, not any of its children.
        if site.language == NativeLanguage::Markdown
            && let Some(segment) = doc.fenced_segment_endpoint(position, side)
            && let Some(mount) = self
                .markdown_mounts
                .iter()
                .find(|mount| mount.source_ranges().any(|range| range == segment))
        {
            let inward = match side {
                Assoc::Before => Assoc::After,
                Assoc::After => Assoc::Before,
            };
            let mut query =
                Query::new(position, inward, self.text().len()).projected(mount.source_ranges())?;
            query.side = side;
            return Some(native(mount.language(), mount.syntax(), query));
        }
        Some(site)
    }

    pub(super) fn site(&self, position: usize, side: Assoc) -> Option<Site<'_>> {
        self.site_with_hosts(position, side, |_| {})
    }

    pub(super) fn site_with_hosts<'a>(
        &'a self,
        position: usize,
        side: Assoc,
        mut host: impl FnMut(Site<'a>),
    ) -> Option<Site<'a>> {
        let text = self.syntax.text();
        if !text.is_char_boundary(position) {
            return None;
        }
        let query = Query::new(position, side, text.len());
        for mount in &self.markdown_mounts {
            if let Some(nested) = query.projected(mount.source_ranges()) {
                host(native(self.language, &self.syntax, query));
                return Some(locate(mount.language(), mount.syntax(), nested, &mut host));
            }
        }
        Some(locate(self.language, &self.syntax, query, &mut host))
    }
}

fn locate<'a>(
    language: NativeLanguage,
    syntax: &'a NativeSyntax,
    query: Query,
    host: &mut impl FnMut(Site<'a>),
) -> Site<'a> {
    match syntax {
        NativeSyntax::Html(doc) => return html(doc, query, HtmlHost::Plain, host),
        NativeSyntax::Javascript(doc) => {
            let typed = matches!(
                language,
                NativeLanguage::Javascript(JavascriptOptions {
                    typescript: true,
                    ..
                })
            );
            return javascript(doc, language, query, typed);
        }
        NativeSyntax::Vue(doc) => {
            for expression in doc.expressions() {
                if let Some(nested) = query.enter(expression.range(), 0) {
                    host(html_site(doc.host(), query, HtmlHost::Vue));
                    return javascript(expression.document(), js_plain(), nested, false);
                }
            }
            return html(doc.host(), query, HtmlHost::Vue, host);
        }
        NativeSyntax::Angular(doc) => {
            for expression in doc.regions().iter().flat_map(|region| region.scripts()) {
                if let Some(nested) = query.enter(expression.range(), 0) {
                    host(html_site(doc.html(), query, HtmlHost::Angular));
                    return javascript(expression.document(), js_plain(), nested, false);
                }
            }
            return html(doc.html(), query, HtmlHost::Angular, host);
        }
        NativeSyntax::Jinja(doc) => {
            if let Some(outer) = doc.html()
                && let Some(ranges) = outer.source_ranges(0..outer.document().text().len())
                && let Some(nested) = query.projected(ranges)
            {
                host(native(language, syntax, query));
                return html(outer.document(), nested, HtmlHost::Template, host);
            }
        }
        NativeSyntax::Liquid(doc) => {
            if !(query.side == Assoc::Before && liquid::has_recovered_end_tag_name(doc, query.at))
                && let Some(outer) = doc.html()
                && let Some(nested) =
                    query.projected(outer.ranges().iter().map(|range| range.source.clone()))
            {
                host(native(language, syntax, query));
                return html(outer.document(), nested, HtmlHost::Template, host);
            }
        }
        NativeSyntax::Php(doc) => {
            if let Some(outer) = doc.html()
                && let Some(ranges) = outer.source_ranges(0..outer.document().text().len())
                && let Some(nested) = query.projected(ranges)
            {
                host(native(language, syntax, query));
                return html(outer.document(), nested, HtmlHost::Plain, host);
            }
        }
        _ => {}
    }
    native(language, syntax, query)
}

fn native(language: NativeLanguage, syntax: &NativeSyntax, query: Query) -> Site<'_> {
    Site {
        language,
        grammar: Grammar::Native(syntax),
        position: query,
        html_host: HtmlHost::Plain,
    }
}

fn html<'a>(
    document: &'a HtmlDocument,
    query: Query,
    html_host: HtmlHost,
    host: &mut impl FnMut(Site<'a>),
) -> Site<'a> {
    for mount in document.mounts() {
        if let Some(nested) = query.enter(mount.range(), 0) {
            let site = match mount.document() {
                HtmlEmbeddedDocument::Css(doc) => Site {
                    language: NativeLanguage::Css,
                    grammar: Grammar::Css(doc),
                    position: nested,
                    html_host: HtmlHost::Plain,
                },
                HtmlEmbeddedDocument::Javascript(doc) => {
                    let kind = mount.language();
                    let options = JavascriptOptions {
                        jsx: matches!(kind, HtmlEmbeddedLanguage::Jsx | HtmlEmbeddedLanguage::Tsx),
                        typescript: matches!(
                            kind,
                            HtmlEmbeddedLanguage::TypeScript | HtmlEmbeddedLanguage::Tsx
                        ),
                    };
                    javascript(doc, NativeLanguage::Javascript(options), nested, false)
                }
                HtmlEmbeddedDocument::Custom(_) => break,
            };
            host(html_site(document, query, html_host));
            return site;
        }
    }
    html_site(document, query, html_host)
}

fn html_site(document: &HtmlDocument, query: Query, html_host: HtmlHost) -> Site<'_> {
    Site {
        language: NativeLanguage::Html,
        grammar: Grammar::Html(document),
        position: query,
        html_host,
    }
}

fn javascript(
    doc: &JavascriptDocument,
    language: NativeLanguage,
    position: Query,
    typescript_defaults: bool,
) -> Site<'_> {
    Site {
        language,
        grammar: Grammar::Javascript(doc, typescript_defaults),
        position,
        html_host: HtmlHost::Plain,
    }
}

fn js_plain() -> NativeLanguage {
    NativeLanguage::Javascript(JavascriptOptions::default())
}

pub(super) fn in_jsx(site: &Site<'_>) -> bool {
    let Grammar::Javascript(doc, _) = site.grammar else {
        return false;
    };
    let NativeLanguage::Javascript(JavascriptOptions { jsx: true, .. }) = site.language else {
        return false;
    };
    let mut node = doc.root();
    while let Some(child) = node
        .children()
        .find(|child| site.position.inside(&child.range()))
    {
        node = child;
    }
    node.kind().name().starts_with("JSX")
}
