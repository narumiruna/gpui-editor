use std::{collections::HashMap, fmt, ops::Range, sync::Arc};

use crate::{
    Assoc, Text,
    language::{
        HighlightSpan, StreamSettings,
        html::{HtmlDocument, HtmlOptions},
        lezer::ParseError,
        wast::WastDocument,
    },
    language_data::{
        JsonLanguage, LanguageDescription, LezerLanguage, NativeAsyncLanguageLoad, NativeLanguage,
        NativeLanguageSupport, NativeSyntax, RustLanguage, SupportedDocument, SyntaxUpdateError,
        UnavailableLanguage, WastLanguage,
    },
};

use super::{MarkdownDialect, MarkdownDocument, MarkdownNodeKind, MarkdownOverlay};

/// Parser choices for the native Markdown language extension.
#[derive(Clone, Default)]
pub struct MarkdownParserConfig {
    pub base: MarkdownDialect,
    pub code_languages: Option<MarkdownCodeLanguages>,
    /// Use this language when neither a named parser nor a configured fallback is found.
    pub default_code_language: Option<NativeLanguage>,
}

/// Find a native parser for the first word of a nonempty code info string.
pub type MarkdownCodeResolver = dyn Fn(&str) -> Option<NativeLanguage> + Send + Sync;

/// A native code parser. Configured variants retain their parser identity and
/// strict policy rather than selecting the grammar's default parser.
#[derive(Clone, PartialEq, Eq)]
pub enum MarkdownCodeParser {
    Native(NativeLanguage),
    Json(JsonLanguage),
    Lezer(LezerLanguage),
    Rust(RustLanguage),
    Wast(WastLanguage),
}

impl MarkdownCodeParser {
    /// Parse an editor body, retaining original UTF-8 offsets and parser policy.
    /// Errors refer to the body; the caller maps them into its host document.
    pub fn parse(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<SupportedDocument, SyntaxUpdateError> {
        match self {
            Self::Native(language) => language
                .support()
                .parse_with_code_languages(text, settings, &[])
                .map_err(SyntaxUpdateError::Tokenize),
            Self::Json(language) => language
                .parse_for_editor(text, settings)
                .map_err(SyntaxUpdateError::StrictSyntax),
            Self::Lezer(language) => language
                .parse_for_editor(text, settings)
                .map_err(SyntaxUpdateError::StrictSyntax),
            Self::Rust(language) => language
                .parse_for_editor(text, settings)
                .map_err(SyntaxUpdateError::StrictSyntax),
            Self::Wast(language) => language
                .parse_for_editor(text, settings)
                .map_err(SyntaxUpdateError::StrictSyntax),
        }
    }

    pub fn language(&self) -> NativeLanguage {
        match self {
            Self::Native(language) => *language,
            Self::Json(_) => NativeLanguage::Json,
            Self::Lezer(_) => NativeLanguage::LezerGrammar,
            Self::Rust(_) => NativeLanguage::Rust,
            Self::Wast(_) => NativeLanguage::Wast,
        }
    }
}

/// Find a parser without discarding its configuration.
pub type MarkdownCodeParserResolver = dyn Fn(&str) -> Option<MarkdownCodeParser> + Send + Sync;

/// A shared load for a Markdown fence. The host owns and polls its attempt.
pub type MarkdownCodeLoad =
    NativeAsyncLanguageLoad<'static, NativeLanguageSupport, UnavailableLanguage>;

/// A selected fence parser or an async handle for one. Neither uses the fallback.
#[derive(Clone)]
pub enum MarkdownCodeSelection {
    Ready(MarkdownCodeParser),
    Load(Arc<MarkdownCodeLoad>),
}

/// Select a parser or load handle by the first word of the code info string.
/// Return `None` for no match. Reuse the same handle for the same description.
pub type MarkdownCodeSelectionResolver =
    dyn Fn(&str) -> Option<MarkdownCodeSelection> + Send + Sync;

fn loaded_parser(load: &MarkdownCodeLoad) -> Option<MarkdownCodeParser> {
    load.support()
        .map(|support| MarkdownCodeParser::Native(support.language()))
}

/// Resolve the first word of a fenced code block's info string.
#[derive(Clone)]
pub enum MarkdownCodeLanguages {
    /// Match a canonical name or alias in catalog order, including fuzzy names.
    Catalog(Arc<[LanguageDescription]>),
    /// Return a native language, or `None` to use the fallback parser.
    Resolve(Arc<MarkdownCodeResolver>),
    /// Resolve a configured parser. `fallback` takes precedence over
    /// `MarkdownParserConfig::default_code_language` when no name matches.
    Configured {
        resolve: Arc<MarkdownCodeParserResolver>,
        fallback: Option<MarkdownCodeParser>,
    },
    /// Allow a named language to load on demand. Use `fallback` only when no
    /// language matches, not when a selected load has no support yet.
    Deferred {
        resolve: Arc<MarkdownCodeSelectionResolver>,
        fallback: Option<MarkdownCodeParser>,
    },
}

impl MarkdownParserConfig {
    /// Resolve the grammar for a raw info string without the fence markers.
    /// Use [`Self::code_parser`] to retain the parser configuration.
    pub fn code_language(&self, info: &str) -> Option<NativeLanguage> {
        self.code_parser(info).map(|parser| parser.language())
    }

    /// Resolve the named parser before either fallback. An empty info string
    /// bypasses named selection. A selected load without support returns
    /// `None`, not the fallback. This query does not start an attempt.
    pub fn code_parser(&self, info: &str) -> Option<MarkdownCodeParser> {
        match self.code_selection(info)? {
            MarkdownCodeSelection::Ready(parser) => Some(parser),
            MarkdownCodeSelection::Load(load) => loaded_parser(&load),
        }
    }

    fn code_selection(&self, info: &str) -> Option<MarkdownCodeSelection> {
        use MarkdownCodeSelection::Ready;
        let native = |language| Ready(MarkdownCodeParser::Native(language));
        let name = (!info.is_empty()).then(|| {
            info.split_once(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
                .map_or(info, |(name, _)| name)
        });
        let selected = match &self.code_languages {
            Some(MarkdownCodeLanguages::Catalog(catalog)) => name
                .and_then(|name| LanguageDescription::match_name(catalog, name, true))
                .and_then(LanguageDescription::built_in)
                .map(native),
            Some(MarkdownCodeLanguages::Resolve(resolve)) => {
                name.and_then(|name| resolve(name)).map(native)
            }
            Some(MarkdownCodeLanguages::Configured { resolve, fallback }) => name
                .and_then(|name| resolve(name))
                .or_else(|| fallback.clone())
                .map(Ready),
            Some(MarkdownCodeLanguages::Deferred { resolve, fallback }) => name
                .and_then(|name| resolve(name))
                .or_else(|| fallback.clone().map(Ready)),
            None => None,
        };
        selected.or(self.default_code_language.map(native))
    }
}

/// An embedded parser result and its source map into the Markdown document.
#[derive(Clone)]
pub struct MarkdownMount(Arc<MountedSyntax>);

struct MountedSyntax {
    code_parser: Option<MarkdownCodeParser>,
    syntax: NativeSyntax,
    segments: Vec<Segment>,
}

#[derive(Clone)]
pub(super) struct PendingCodeMount {
    load: Arc<MarkdownCodeLoad>,
    ranges: Vec<Range<usize>>,
}

impl fmt::Debug for PendingCodeMount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingCodeMount")
            .field("language", &self.load.description().name)
            .field("ranges", &self.ranges)
            .finish()
    }
}

#[derive(Debug)]
struct Segment {
    source: Range<usize>,
    start: usize,
}

impl Segment {
    fn parsed_end(&self) -> usize {
        self.start + self.source.len()
    }
}

/// The ordered, nonempty segments of one mount. This map does not grant edit access.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MarkdownSourceMap<'a>(&'a [Segment]);

impl MarkdownSourceMap<'_> {
    pub(crate) fn bounds(self) -> Range<usize> {
        self.0[0].source.start..self.0.last().unwrap().source.end
    }

    pub(crate) fn is_contiguous(self) -> bool {
        self.bounds().len() == self.0.last().unwrap().parsed_end()
    }

    /// The LR parser advances to the next segment at a join, even when a
    /// token ends there. The final segment owns EOF.
    pub(crate) fn source_offset(self, local: usize) -> usize {
        let i = self
            .0
            .partition_point(|segment| segment.parsed_end() <= local);
        self.0.get(i).map_or_else(
            || self.0.last().unwrap().source.end,
            |segment| segment.source.start + (local - segment.start),
        )
    }

    /// A callback result may use either edge of a gap, but not its interior.
    pub(crate) fn contains_endpoint(self, at: usize) -> bool {
        let i = self.0.partition_point(|segment| segment.source.end < at);
        self.0
            .get(i)
            .is_some_and(|segment| segment.source.start <= at)
    }

    pub(crate) fn segment_at(self, at: usize, side: Assoc) -> Option<(Range<usize>, usize)> {
        let i = self.0.partition_point(|segment| match side {
            Assoc::Before => segment.source.end < at,
            Assoc::After => segment.source.end <= at,
        });
        let segment = self.0.get(i)?;
        let start = segment.source.start;
        (start < at || (start == at && side == Assoc::After))
            .then(|| (segment.source.clone(), segment.start))
    }
}

impl MarkdownDocument {
    /// Parse the HTML and code overlays. The resolver receives the first
    /// word of the code info string, or an empty string if none was given.
    /// Return `None` to leave that block unparsed.
    pub fn mounts<'a>(
        &'a self,
        resolve: impl Fn(&str) -> Option<NativeLanguage> + 'a,
    ) -> impl Iterator<Item = MarkdownMount> + 'a {
        self.mounts_with_settings(
            move |info, _| {
                resolve(info.split_whitespace().next().unwrap_or(""))
                    .map(MarkdownCodeParser::Native)
            },
            StreamSettings::default(),
        )
        .filter_map(Result::ok)
    }

    // A host section can include the line break after the code block.
    // No other difference in the block bounds identifies the same source.
    pub(crate) fn code_block_ranges(
        &self,
        block: &Range<usize>,
    ) -> Option<impl Iterator<Item = Range<usize>> + Clone + '_> {
        use MarkdownNodeKind::{CodeBlock, CodeText, FencedCode};

        self.text().slice(block.clone())?;
        let (node, _) = self.path(block.start, false).find(|(node, _)| {
            matches!(node.kind, CodeBlock | FencedCode)
                && node.range.start == block.start
                && matches!(
                    self.text().slice(node.range.end..block.end),
                    Some("" | "\n" | "\r\n" | "\r")
                )
        })?;
        Some(
            node.children
                .iter()
                .filter(|child| child.kind == CodeText && !child.range.is_empty())
                .map(|child| child.range.clone()),
        )
    }

    pub(crate) fn configure_mounts(
        &mut self,
        config: &MarkdownParserConfig,
        settings: StreamSettings,
    ) -> Result<Vec<MarkdownMount>, ParseError> {
        let mut pending = Vec::new();
        let mounts = self
            .mounts_with_settings(
                |info, ranges| match config.code_selection(info)? {
                    MarkdownCodeSelection::Ready(parser) => Some(parser),
                    MarkdownCodeSelection::Load(load) => {
                        let parser = loaded_parser(&load);
                        if parser.is_none() {
                            // Start or join even for an empty body. The handle retains the
                            // attempt; parsing does not poll it.
                            drop(load.load());
                            let ranges: Vec<_> = ranges
                                .iter()
                                .filter(|range| !range.is_empty())
                                .cloned()
                                .collect();
                            if !ranges.is_empty() {
                                pending.push(PendingCodeMount { load, ranges });
                            }
                        }
                        parser
                    }
                },
                settings,
            )
            .collect::<Result<_, _>>()?;
        self.pending_code = pending.into();
        Ok(mounts)
    }

    /// Source segments covered by placeholders, not by child syntax.
    pub(crate) fn pending_code_ranges(&self) -> Option<impl Iterator<Item = Range<usize>> + '_> {
        (!self.pending_code.is_empty()).then(|| {
            self.pending_code
                .iter()
                .flat_map(|fence| fence.ranges.iter().cloned())
        })
    }

    pub(crate) fn refresh_mounts(
        &self,
        settings: StreamSettings,
    ) -> Result<Option<(Self, Vec<MarkdownMount>)>, ParseError> {
        let mut support = HashMap::new();
        let mut mounts = Vec::new();
        let mut pending = Vec::new();
        for fence in &*self.pending_code {
            // Fences that share a handle use the same readiness result.
            let parser = support
                .entry(Arc::as_ptr(&fence.load))
                .or_insert_with(|| loaded_parser(&fence.load));
            if let Some(parser) = parser
                && let Some(mount) = self
                    .mount_with_settings(Some(parser.clone()), &fence.ranges, settings)
                    .transpose()?
            {
                mounts.push(mount);
            } else {
                pending.push(fence);
            }
        }
        if mounts.is_empty() {
            return Ok(None);
        }
        let mut doc = self.clone();
        doc.pending_code = pending.into_iter().cloned().collect();
        doc.parsed_bytes = 0;
        doc.reused_blocks = self.root().children.len();
        Ok(Some((doc, mounts)))
    }

    fn mounts_with_settings<'a>(
        &'a self,
        mut resolve: impl FnMut(&str, &[Range<usize>]) -> Option<MarkdownCodeParser> + 'a,
        settings: StreamSettings,
    ) -> impl Iterator<Item = Result<MarkdownMount, ParseError>> + 'a {
        self.overlays().filter_map(move |overlay| {
            let (language, ranges) = match overlay {
                MarkdownOverlay::Code { info, ranges } => (Some(resolve(info, &ranges)?), ranges),
                MarkdownOverlay::Html { ranges } => (None, ranges),
            };
            self.mount_with_settings(language, &ranges, settings)
        })
    }

    fn mount_with_settings(
        &self,
        language: Option<MarkdownCodeParser>,
        ranges: &[Range<usize>],
        settings: StreamSettings,
    ) -> Option<Result<MarkdownMount, ParseError>> {
        let mut input = String::new();
        let mut segments = Vec::with_capacity(ranges.len());
        for source in ranges.iter().filter(|range| !range.is_empty()) {
            let start = input.len();
            input.push_str(self.text().slice(source.clone())?);
            segments.push(Segment {
                source: source.clone(),
                start,
            });
        }
        if segments.is_empty() {
            return None;
        }
        let text = Text::new(input);
        let syntax = match &language {
            Some(MarkdownCodeParser::Native(language)) => {
                language.parse_with_stream_settings(text, settings).ok()?
            }
            Some(parser) => match parser.parse(text, settings) {
                Ok(document) => document.into_syntax(),
                Err(SyntaxUpdateError::StrictSyntax(error)) => {
                    return Some(Err(error.map_offset(|offset| {
                        MarkdownSourceMap(&segments).source_offset(offset)
                    })));
                }
                Err(_) => return None,
            },
            None => NativeSyntax::Html(HtmlDocument::parse_with_options(
                text,
                HtmlOptions {
                    match_closing_tags: false,
                    ..HtmlOptions::default()
                },
            )),
        };
        Some(Ok(MarkdownMount(Arc::new(MountedSyntax {
            code_parser: language,
            syntax,
            segments,
        }))))
    }
}

impl MarkdownMount {
    pub fn language(&self) -> NativeLanguage {
        self.0
            .code_parser
            .as_ref()
            .map_or(NativeLanguage::Html, MarkdownCodeParser::language)
    }

    pub(crate) fn code_parser(&self) -> Option<&MarkdownCodeParser> {
        self.0.code_parser.as_ref()
    }

    pub(crate) fn uses_parser(&self, expected: &MarkdownCodeParser) -> bool {
        self.code_parser() == Some(expected)
    }

    pub(crate) fn has_configured_folds(&self) -> bool {
        self.wast().is_some()
            || matches!(
                &self.0.code_parser,
                Some(
                    MarkdownCodeParser::Json(_)
                        | MarkdownCodeParser::Lezer(_)
                        | MarkdownCodeParser::Rust(_)
                )
            )
    }

    /// Syntax positions are relative to the concatenated embedded source.
    pub fn syntax(&self) -> &NativeSyntax {
        &self.0.syntax
    }

    /// The Markdown byte ranges consumed by this parser, in input order.
    pub fn source_ranges(&self) -> impl ExactSizeIterator<Item = Range<usize>> + '_ {
        self.0.segments.iter().map(|segment| segment.source.clone())
    }

    pub(crate) fn source_map(&self) -> MarkdownSourceMap<'_> {
        MarkdownSourceMap(&self.0.segments)
    }

    pub(crate) fn wast(&self) -> Option<&WastDocument> {
        match self.syntax() {
            NativeSyntax::Wast(doc) => Some(doc),
            _ => None,
        }
    }

    /// Project embedded highlights into the original Markdown. If a token
    /// crosses excluded markup, split its span at that markup.
    pub fn highlights(&self) -> impl Iterator<Item = HighlightSpan<'_>> {
        let syntax = self.syntax();
        let segments = &self.0.segments;
        syntax
            .highlights(0..syntax.text().len())
            .into_iter()
            .flatten()
            .flat_map(|span| {
                let start =
                    segments.partition_point(|segment| segment.parsed_end() <= span.range.start);
                segments[start..]
                    .iter()
                    .take_while(move |segment| segment.start < span.range.end)
                    .map(move |segment| HighlightSpan {
                        range: segment.source.start + span.range.start.saturating_sub(segment.start)
                            ..segment.source.start + span.range.end.min(segment.parsed_end())
                                - segment.start,
                        tags: span.tags,
                    })
            })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::language::wast::WastNode;

    #[test]
    fn wast_node_projection_matches_pinned_lr_joins() {
        let span = |node: WastNode<'_>| {
            let range = node.range();
            json!([node.node_type().name(), range.start, range.end])
        };
        for row in include_str!("../../../tests/fixtures/wast_markdown_callbacks.jsonl")
            .lines()
            .take(2)
        {
            let row: Value = serde_json::from_str(row).unwrap();
            assert_eq!(row["step"], "LR joins");
            let source = row["source"].as_str().unwrap();
            let mut input = String::new();
            let mut segments = Vec::new();
            for range in row["ranges"].as_array().unwrap() {
                let [from, to] = serde_json::from_value::<[usize; 2]>(range.clone()).unwrap();
                segments.push(Segment {
                    source: from..to,
                    start: input.len(),
                });
                input.push_str(&source[from..to]);
            }
            let length = input.len();
            let doc = WastDocument::parse(Text::new(input));
            let map = MarkdownSourceMap(&segments);
            let nodes: Vec<_> = doc
                .nodes()
                .map(|node| {
                    let node = node.in_markdown(map);
                    json!([span(node), node.parent().map(span)])
                })
                .collect();
            assert_eq!(json!(nodes), row["nodes"]);
            assert_eq!(doc.root().range(), 0..length);
        }
    }
}
