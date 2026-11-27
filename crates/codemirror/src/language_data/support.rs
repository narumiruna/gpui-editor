//! Composition of the native services installed by catalog entries.

use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, EditorState, Extension, Text,
    commands::CommentTokens,
    language::{
        HighlightContext, HighlightSpan, IndentContext, LanguageData, StreamSettings, angular, cpp,
        css, go, html,
        html::{HtmlEmbeddedDocument, HtmlEmbeddedLanguage},
        java, javascript, jinja, json, language_data_provider, less,
        lezer::ParseError,
        lezer_grammar,
        line_projection::splits_crlf,
        liquid,
        markdown::{
            self, MarkdownCodeLanguages, MarkdownDocument, MarkdownMount, MarkdownParserConfig,
        },
        php, python, rust, sass,
        wast::WastDataFacet,
        xml, yaml,
    },
    legacy_modes::asn1,
    stream_parser::TokenizeError,
};

use super::{LanguageDescription, NativeLanguage, NativeSyntax};
use route::HtmlHost;

mod completion;
mod go_install;
mod indent_input;
mod input;
mod javascript_regions;
mod lr_data;
mod projection;
mod route;
mod selection;
mod tracked;
mod update;

pub use go_install::{GoCompletionSource, go_extension, go_language_extension};
pub use lr_data::LrDataFacet;
pub use tracked::{
    JsonLanguage, LezerLanguage, WastLanguage, attached_language_snapshot, json_language,
    lezer_language, wast_language,
};
pub(crate) use tracked::{WastConfig, active_wast_viewport};
pub use update::SyntaxUpdateError;

/// The bundled parser configuration and the native default language services.
/// Embedded syntax reuses the same defaults. This does not install a UI or
/// require a JavaScript module loader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeLanguageSupport {
    language: NativeLanguage,
}

impl NativeLanguageSupport {
    pub(super) fn new(language: NativeLanguage) -> Self {
        Self { language }
    }

    pub fn language(self) -> NativeLanguage {
        self.language
    }

    /// The maximum number of default source slots at any document position.
    /// A mounted language can have fewer active sources than the host limit.
    pub fn completion_source_capacity(self) -> usize {
        use NativeLanguage::*;
        match self.language {
            Angular | Go | Html | Javascript(_) | Jinja | Liquid | Markdown | Php | Python
            | Vue => 2,
            Css | Less | Sass(_) | Sql(_) | Xml => 1,
            _ => 0,
        }
    }

    /// Parse with the default services. Markdown mounts HTML, but by default
    /// does not select a language for code fences.
    pub fn parse(self, text: Text) -> Result<SupportedDocument, TokenizeError> {
        self.parse_with_code_languages(text, StreamSettings::default(), &[])
    }

    /// Parse a native snapshot and optionally find Markdown fence languages
    /// by name or alias in the given catalog. Other parsers ignore `catalog`.
    pub fn parse_with_code_languages(
        self,
        text: Text,
        settings: StreamSettings,
        catalog: &[LanguageDescription],
    ) -> Result<SupportedDocument, TokenizeError> {
        if self.language == NativeLanguage::Markdown {
            return Ok(SupportedDocument::parse_markdown(
                text,
                settings,
                Arc::new(MarkdownParserConfig {
                    code_languages: (!catalog.is_empty())
                        .then(|| MarkdownCodeLanguages::Catalog(catalog.into())),
                    ..MarkdownParserConfig::default()
                }),
            )
            .expect("catalog mounts do not use strict parsers"));
        }
        Ok(SupportedDocument {
            language: self.language,
            syntax: self.language.parse_with_stream_settings(text, settings)?,
            settings,
            markdown_parser: None,
            markdown_mounts: Vec::new(),
            strict_parser: None,
        })
    }
}

/// Language-specific close-bracket settings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CloseBracketData {
    /// Opening tokens. `None` uses the default list.
    pub brackets: Option<&'static [&'static str]>,
    /// Characters that allow a pair before them. `None` or empty uses the default.
    pub before: Option<&'static str>,
    pub string_prefixes: &'static [&'static str],
}

/// Native parser output and the installed services for one immutable document.
pub struct SupportedDocument {
    language: NativeLanguage,
    syntax: NativeSyntax,
    settings: StreamSettings,
    markdown_parser: Option<Arc<MarkdownParserConfig>>,
    markdown_mounts: Vec<MarkdownMount>,
    strict_parser: Option<StrictParser>,
}

#[derive(Clone)]
enum StrictParser {
    Lr(LrGrammar),
    Wast(Arc<WastConfig>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LrGrammar {
    Json,
    Lezer,
}

impl SupportedDocument {
    pub(crate) fn parse_markdown(
        text: Text,
        settings: StreamSettings,
        parser: Arc<MarkdownParserConfig>,
    ) -> Result<Self, ParseError> {
        let syntax =
            NativeSyntax::Markdown(MarkdownDocument::parse_with_dialect(text, parser.base));
        let markdown_mounts = markdown_mounts(&syntax, settings, Some(&parser))?;
        Ok(Self {
            language: NativeLanguage::Markdown,
            syntax,
            settings,
            markdown_parser: Some(parser),
            markdown_mounts,
            strict_parser: None,
        })
    }

    pub fn text(&self) -> &Text {
        self.syntax.text()
    }

    pub fn syntax(&self) -> &NativeSyntax {
        &self.syntax
    }

    pub(crate) fn into_syntax(self) -> NativeSyntax {
        self.syntax
    }

    /// The language at a byte offset, including native mounts. Boundary
    /// association determines which region to use at a shared endpoint.
    pub fn language_at(&self, position: usize, side: Assoc) -> Option<NativeLanguage> {
        self.site(position, side).map(|site| site.language)
    }

    /// Like [`Self::language_at`], but in Markdown also report a mounted fence
    /// language at a nonempty segment endpoint when `side` reaches its fence.
    /// UTF-8 interiors and, in Markdown, CRLF midpoints return `None`.
    /// This reports a grammar, not facet identity or edit ownership. Nested
    /// mount routing is unchanged; a newly included endpoint reports the fence
    /// language.
    pub fn markdown_language_presence_at(
        &self,
        position: usize,
        side: Assoc,
    ) -> Option<NativeLanguage> {
        self.markdown_presence_site(position, side)
            .map(|site| site.language)
    }

    pub(crate) fn markdown_mounted_data_at(
        &self,
        state: &EditorState,
        position: usize,
        side: Assoc,
    ) -> Option<Vec<Arc<LanguageData>>> {
        let NativeSyntax::Markdown(markdown) = &self.syntax else {
            return None;
        };
        let site = self.markdown_presence_site(position, side)?;
        let mount = site.markdown_mount?;
        if let Some(wast) = site.wast() {
            let map = mount.source_map();
            return Some(wast.configured_data_in_markdown(
                state,
                map,
                || markdown.wast_sublanguage_position(position, map.bounds().start),
                side,
            ));
        }
        let row = site.lr_root_type()?;
        Some(
            row.data_facet()
                .map_or_else(Vec::new, |facet| facet.read(state)),
        )
    }

    pub(crate) fn markdown_wast_fold(
        &self,
        state: &EditorState,
        start: usize,
        end: usize,
    ) -> Option<Range<usize>> {
        let source = self.text().as_str();
        if splits_crlf(source, start) || splits_crlf(source, end) {
            return None;
        }
        self.markdown_mounts.iter().find_map(|mount| {
            mount
                .wast()?
                .foldable_in_markdown(state, mount.source_map(), start, end)
        })
    }

    pub(crate) fn markdown_wast_body(
        &self,
        body: &Range<usize>,
        parser: &WastLanguage,
    ) -> Option<&MarkdownMount> {
        self.markdown_mounts.iter().find(|mount| {
            let map = mount.source_map();
            mount.uses_wast_parser(parser) && map.is_contiguous() && &map.bounds() == body
        })
    }

    pub(crate) fn markdown_wast_indentation(
        &self,
        state: &EditorState,
        lines: &IndentContext<'_>,
    ) -> Option<usize> {
        let position = lines.position();
        if splits_crlf(self.text().as_str(), position) {
            return None;
        }
        let site = self.site(position, Assoc::Before)?;
        let map = site.markdown_mount?.source_map();
        // A WAST indentation strategy cannot own a position at a fence or
        // at a gap boundary unless the mount covers both sides.
        map.segment_at(position, Assoc::After)?;
        site.wast()?.indentation_in_markdown(map, state, lines)
    }

    /// Return WAST regions in original UTF-8 bytes and source order. A WAST
    /// document returns its whole range, even when empty. Markdown mounts
    /// with the stock WAST facet contribute one range per source segment.
    /// This query does not grant permission to edit those ranges.
    pub fn wast_regions(&self) -> Vec<Range<usize>> {
        self.regions_with_facet(NativeLanguage::Wast, WastDataFacet::stock())
    }

    /// Typed metadata for the deepest supported language, with sublanguage
    /// data before the base data. For WAST, JSON, and Lezer this returns
    /// the effective root's defaults only. Positions must be UTF-8 boundaries.
    pub fn data_at(&self, position: usize, side: Assoc) -> Vec<Arc<LanguageData>> {
        let Some(site) = self.site(position, side) else {
            return Vec::new();
        };
        if let Some(row) = site.lr_root_type() {
            return row
                .data_facet()
                .and_then(LrDataFacet::base)
                .cloned()
                .into_iter()
                .collect();
        }
        let mut values = Vec::new();
        if route::in_jsx(&site) {
            values.push(Arc::new(LanguageData::default().with(
                "commentTokens",
                CommentTokens {
                    line: None,
                    block: Some(javascript::JSX_BLOCK_COMMENT),
                },
            )));
        }
        let outer_brackets: Option<&[_]> = match site.html_host {
            HtmlHost::Plain => None,
            HtmlHost::Angular => Some(&["[", "{", "\""]),
            HtmlHost::Vue => Some(&["{", "\""]),
            HtmlHost::Template => Some(&["{"]),
        };
        if let Some(brackets) = outer_brackets {
            let mut overlay = LanguageData::default().with(
                "closeBrackets",
                CloseBracketData {
                    brackets: Some(brackets),
                    ..Default::default()
                },
            );
            if site.html_host == HtmlHost::Angular {
                overlay = overlay.with(
                    "indentOnInput",
                    angular::template_indent_on_input as fn(&str) -> bool,
                );
            }
            values.push(Arc::new(overlay));
        }
        values.push(Arc::new(metadata(site.language)));
        values
    }

    /// Attach positional language data to the matching editor snapshot. The
    /// provider returns no values if the editor text differs. For an extension
    /// that follows edits, use
    /// [`NativeLanguageSupport::tracked_language_data_extension`].
    pub fn language_data_extension(self: &Arc<Self>) -> Extension {
        self.language_data_extension_in(self.text(), 0..self.text().len())
            .expect("the document matches itself")
    }

    /// Install snapshot data for a matching byte range in a larger source.
    /// Return `None` if the source range is invalid or differs from this snapshot.
    pub fn language_data_extension_in(
        self: &Arc<Self>,
        source: &Text,
        scope: Range<usize>,
    ) -> Option<Extension> {
        if source.slice(scope.clone())? != self.text().as_str() {
            return None;
        }
        let document = Arc::clone(self);
        let source = source.clone();
        Some(language_data_provider(move |state, position, side| {
            if (scope.start..=scope.end).contains(&position) && *state.text() == source {
                document.data_at(position - scope.start, side)
            } else {
                Vec::new()
            }
        }))
    }

    /// Return the exact bundled language configuration for a highlight span
    /// from this snapshot. `range` is a nonempty UTF-8 byte range. In HTML, a
    /// span must lie wholly inside one supported mount or wholly outside all
    /// mounts. Unknown or mixed origins, including template-language and
    /// Markdown projections, have no scope. This is not a caret language query.
    pub fn highlight_scope(&self, range: &Range<usize>) -> Option<NativeLanguage> {
        if range.is_empty() || !self.text().is_valid_range(range.clone()) {
            return None;
        }
        match &self.syntax {
            NativeSyntax::Html(doc) => {
                // Host highlights are cut at each mount. Child highlights are
                // generated inside it and shifted to the same byte coordinates.
                let mounts = doc.mounts();
                let index = mounts.partition_point(|mount| mount.range().end <= range.start);
                let Some(mount) = mounts
                    .get(index)
                    .filter(|mount| mount.range().start < range.end)
                else {
                    return Some(NativeLanguage::Html);
                };
                let mounted = mount.range();
                if range.start < mounted.start || range.end > mounted.end {
                    return None;
                }
                use HtmlEmbeddedLanguage::*;
                match (mount.language(), mount.document()) {
                    (Css, HtmlEmbeddedDocument::Css(_)) => Some(NativeLanguage::Css),
                    (
                        kind @ (JavaScript | TypeScript | Jsx | Tsx),
                        HtmlEmbeddedDocument::Javascript(_),
                    ) => Some(NativeLanguage::Javascript(javascript::JavascriptOptions {
                        jsx: matches!(kind, Jsx | Tsx),
                        typescript: matches!(kind, TypeScript | Tsx),
                    })),
                    // The CSS Styles and JS SingleExpression roots are different
                    // native parser configurations, as are user-defined routes.
                    _ => None,
                }
            }
            NativeSyntax::Asn1(_)
            | NativeSyntax::Cpp(_)
            | NativeSyntax::Css(_)
            | NativeSyntax::Go(_)
            | NativeSyntax::Java(_)
            | NativeSyntax::Javascript(_)
            | NativeSyntax::Json(_)
            | NativeSyntax::Legacy(_)
            | NativeSyntax::Less(_)
            | NativeSyntax::LezerGrammar(_)
            | NativeSyntax::Python(_)
            | NativeSyntax::Rust(_)
            | NativeSyntax::Sass(_)
            | NativeSyntax::Sql(_)
            | NativeSyntax::Wast(_)
            | NativeSyntax::Xml(_)
            | NativeSyntax::Yaml(_) => Some(self.language),
            NativeSyntax::Angular(_)
            | NativeSyntax::Jinja(_)
            | NativeSyntax::Liquid(_)
            | NativeSyntax::Markdown(_)
            | NativeSyntax::Php(_)
            | NativeSyntax::Vue(_) => None,
        }
    }

    /// Return the scope for a nonempty highlight range in original UTF-8
    /// bytes. JSON, Lezer, and WAST include the actual root row and its
    /// effective facet. A direct Markdown mount of one of these languages
    /// supplies that context within one source segment, not across gaps.
    pub fn highlight_context(&self, range: &Range<usize>) -> Option<HighlightContext> {
        if matches!(&self.syntax, NativeSyntax::Markdown(_)) {
            if range.is_empty() || !self.text().is_valid_range(range.clone()) {
                return None;
            }
            return self.markdown_mounts.iter().find_map(|mount| {
                let root = mount.syntax().lr_root_type()?;
                mount
                    .source_ranges()
                    .find(|segment| segment.start <= range.start && range.end <= segment.end)?;
                HighlightContext::from_root(mount.language(), root)
            });
        }
        let language = self.highlight_scope(range)?;
        match self.syntax.lr_root_type() {
            Some(root) => HighlightContext::from_root(language, root),
            None => Some(HighlightContext::native(language)),
        }
    }

    /// Return syntax highlights, with mounted Markdown languages replacing
    /// the host highlights in their source ranges.
    pub fn highlights(&self, range: Range<usize>) -> Option<Vec<HighlightSpan<'_>>> {
        let base = self.syntax.highlights(range.clone())?;
        if self.markdown_mounts.is_empty() {
            return Some(base.collect());
        }
        let mut covered: Vec<_> = self
            .markdown_mounts
            .iter()
            .flat_map(MarkdownMount::source_ranges)
            .collect();
        covered.sort_by_key(|range| range.start);
        let mut spans = Vec::new();
        for span in base {
            let mut from = span.range.start;
            let first = covered.partition_point(|range| range.end <= from);
            for hole in covered[first..]
                .iter()
                .take_while(|range| range.start < span.range.end)
            {
                if from < hole.start {
                    spans.push(HighlightSpan {
                        range: from..hole.start,
                        tags: span.tags,
                    });
                }
                from = from.max(hole.end);
            }
            if from < span.range.end {
                spans.push(HighlightSpan {
                    range: from..span.range.end,
                    tags: span.tags,
                });
            }
        }
        for span in self
            .markdown_mounts
            .iter()
            .flat_map(MarkdownMount::highlights)
        {
            let start = range.start.max(span.range.start);
            let end = range.end.min(span.range.end);
            if start < end {
                spans.push(HighlightSpan {
                    range: start..end,
                    tags: span.tags,
                });
            }
        }
        spans.sort_by_key(|span| span.range.start);
        Some(spans)
    }
}

fn markdown_mounts(
    syntax: &NativeSyntax,
    settings: StreamSettings,
    parser: Option<&MarkdownParserConfig>,
) -> Result<Vec<MarkdownMount>, ParseError> {
    let (NativeSyntax::Markdown(doc), Some(parser)) = (syntax, parser) else {
        return Ok(Vec::new());
    };
    doc.mounts_with_settings(|info| parser.code_parser(info), settings)
        .collect()
}

fn metadata(language: NativeLanguage) -> LanguageData {
    use NativeLanguage::*;
    let mut data = match language {
        Html => html::language_data(),
        Jinja => jinja::language_data(),
        Liquid => liquid::language_data(),
        Php => php::language_data(),
        LezerGrammar => lezer_grammar::language_data(),
        Markdown => markdown::language_data(),
        Asn1 => asn1::language_data(),
        Json => json::editor_language_data(),
        Legacy(mode) => mode.language().language_data().clone(),
        _ => LanguageData::default(),
    };
    const JS_BRACKETS: &[&str] = &["(", "[", "{", "'", "\"", "`"];
    let brackets = match language {
        Cpp => Some((None, cpp::STRING_PREFIXES.as_slice())),
        Go | Javascript(_) | Sql(_) => Some((Some(JS_BRACKETS), [].as_slice())),
        Legacy(_) => data
            .get::<&[&str]>("closeBrackets")
            .copied()
            .map(|items| (Some(items), [].as_slice())),
        Php => Some((None, php::STRING_PREFIXES.as_slice())),
        Python => Some((
            Some(python::CLOSE_BRACKETS.as_slice()),
            python::STRING_PREFIXES.as_slice(),
        )),
        Rust => Some((None, rust::STRING_PREFIXES.as_slice())),
        _ => None,
    };
    if let Some((brackets, string_prefixes)) = brackets {
        data = data.with(
            "closeBrackets",
            CloseBracketData {
                brackets,
                string_prefixes,
                ..Default::default()
            },
        );
    }
    let indent: Option<fn(&str) -> bool> = match language {
        Cpp => Some(cpp::indent_on_input),
        Css => Some(css::indent_on_input),
        Go => Some(go::indent_on_input),
        Java => Some(java::indent_on_input),
        Javascript(_) => Some(javascript::indent_on_input),
        Less => Some(less::indent_on_input),
        Python => Some(python::indent_on_input),
        Rust => Some(rust::indent_on_input),
        Sass(_) => Some(sass::indent_on_input),
        Xml => Some(xml::indent_on_input),
        Yaml => Some(yaml::indent_on_input),
        _ => None,
    };
    if let Some(indent) = indent {
        data = data.with("indentOnInput", indent);
    }
    let tokens = language.comment_tokens();
    if tokens.line.is_some() || tokens.block.is_some() {
        data = data.with("commentTokens", tokens);
    }
    if let Some(chars) = language.word_characters() {
        data = data.with("wordChars", chars);
    }
    data
}
