use std::sync::LazyLock;

use crate::{
    Assoc,
    autocomplete::{
        Snippet,
        session::{CompletionItem, SourceResult},
    },
    language::{
        css::{CssPropertySnapshot, css_completion_source_with_properties},
        go::{go_completion_snippet, go_local_completion, go_snippet_completion},
        html::html_completion_source,
        javascript::{
            javascript_completion_snippet, javascript_completion_source, local_completion_source,
        },
        jinja::jinja_completion_source,
        less::less_completion_source_with_properties,
        liquid::liquid_completion_source,
        markdown::markdown_completion_source,
        python::{
            global_completion_source as python_globals, local_completion_source as python_locals,
            python_completion_snippet,
        },
        sass::sass_completion_source_with_properties,
        sql::{SqlDialect, SqlKeywordCompletionSource, keyword_completion_source},
        xml::{XmlSchema, xml_completion_source},
    },
};

use super::{
    NativeLanguage, NativeSyntax, SupportedDocument,
    route::{Grammar, Query},
};

impl SupportedDocument {
    /// Run the default sources in registration order. Each result uses
    /// original byte offsets. Skip edits that cross a mount's excluded gap.
    pub fn completions(&self, position: usize, explicit: bool) -> Vec<SourceResult> {
        self.indexed_completions(position, explicit)
            .map(|(_, result)| result)
            .collect()
    }

    /// Stock results keyed by their zero-based source slot. A source that
    /// returns no result leaves its slot empty; later sources keep their numbers.
    pub fn indexed_completions(
        &self,
        position: usize,
        explicit: bool,
    ) -> impl Iterator<Item = (usize, SourceResult)> + '_ {
        self.indexed_completions_with_css_properties(position, explicit, None)
    }

    /// Run the same sources with a property snapshot for CSS, Less, and
    /// Sass, including mounted CSS. `None` selects the pinned stock names.
    pub fn indexed_completions_with_css_properties<'a>(
        &'a self,
        position: usize,
        explicit: bool,
        properties: Option<&CssPropertySnapshot>,
    ) -> impl Iterator<Item = (usize, SourceResult)> + use<'a> {
        let site = self.site(position, Assoc::Before);
        let results = site.as_ref().map_or([None, None], |site| {
            source_results(site, explicit, properties)
        });
        results
            .into_iter()
            .enumerate()
            .filter_map(move |(index, result)| {
                let result = self.project_completion(&site.as_ref()?.position, result?)?;
                Some((index, result))
            })
    }

    pub(super) fn project_completion(
        &self,
        query: &Query,
        mut result: SourceResult,
    ) -> Option<SourceResult> {
        let from = query.source(result.from)?;
        let to = query.source(result.to.unwrap_or(query.at))?;
        if !self.text().is_valid_range(from..to) {
            return None;
        }
        result.from = from;
        if result.to.is_some() {
            result.to = Some(to);
        }
        Some(result)
    }

    /// Return the snippet action for a native default completion option.
    /// This does not edit the document; apply it to the accepted result range.
    pub fn completion_snippet(&self, position: usize, item: &CompletionItem) -> Option<Snippet> {
        match self.language_at(position, Assoc::Before)? {
            NativeLanguage::Javascript(_) => Some(javascript_completion_snippet(item)?.snippet()),
            NativeLanguage::Go => Some(go_completion_snippet(item)?.snippet()),
            NativeLanguage::Python => Some(python_completion_snippet(item)?.snippet()),
            _ => None,
        }
    }
}

fn source_results(
    site: &super::route::Site<'_>,
    explicit: bool,
    properties: Option<&CssPropertySnapshot>,
) -> [Option<SourceResult>; 2] {
    let at = site.position.at;
    let properties = || properties.unwrap_or_else(|| CssPropertySnapshot::stock());
    let css = |doc| css_completion_source_with_properties(doc, at, explicit, properties());
    match site.grammar {
        Grammar::Html(doc) => [html_completion_source(doc, at, explicit), None],
        Grammar::Css(doc) => [css(doc), None],
        Grammar::Javascript(doc, typescript_defaults) => [
            javascript_completion_source(doc, at, explicit, typescript_defaults),
            local_completion_source(doc, at, explicit),
        ],
        Grammar::Native(syntax) => match syntax {
            NativeSyntax::Css(doc) => [css(doc), None],
            NativeSyntax::Go(doc) => [
                go_snippet_completion(doc, at, explicit),
                go_local_completion(doc, at, explicit),
            ],
            NativeSyntax::Jinja(doc) => [jinja_completion_source(doc, at, explicit), None],
            NativeSyntax::Less(doc) => [
                less_completion_source_with_properties(doc, at, explicit, properties()),
                None,
            ],
            NativeSyntax::Liquid(doc) => [liquid_completion_source(doc, at, explicit), None],
            NativeSyntax::Markdown(doc) => [markdown_completion_source(doc, at, explicit), None],
            NativeSyntax::Python(doc) => [
                python_locals(doc, at, explicit),
                python_globals(doc, at, explicit),
            ],
            NativeSyntax::Sass(doc) => [
                sass_completion_source_with_properties(doc, at, explicit, properties()),
                None,
            ],
            NativeSyntax::Sql(doc) => {
                let NativeLanguage::Sql(dialect) = site.language else {
                    return [None, None];
                };
                [sql_keywords(dialect).complete(doc, at, explicit), None]
            }
            NativeSyntax::Xml(doc) => [
                xml_completion_source(doc, at, explicit, XmlSchema::default()),
                None,
            ],
            _ => [None, None],
        },
    }
}

pub(super) fn sql_keywords(dialect: SqlDialect) -> &'static SqlKeywordCompletionSource {
    use SqlDialect::*;
    const DIALECTS: [SqlDialect; 8] = [
        StandardSql,
        PostgreSql,
        MySql,
        MariaSql,
        MsSql,
        Sqlite,
        Cassandra,
        PlSql,
    ];
    static SOURCES: LazyLock<[SqlKeywordCompletionSource; 8]> = LazyLock::new(|| {
        DIALECTS.map(|variant| keyword_completion_source(variant.language(), false))
    });
    &SOURCES[DIALECTS
        .iter()
        .position(|variant| *variant == dialect)
        .expect("bundled dialect")]
}
