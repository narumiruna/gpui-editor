use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, Text,
    language::{
        HighlightSpan, StreamSettings,
        html::{HtmlDocument, HtmlOptions},
        lezer::ParseError,
        wast::WastDocument,
    },
    language_data::{
        JsonLanguage, LanguageDescription, LezerLanguage, NativeLanguage, NativeSyntax,
        SupportedDocument, SyntaxUpdateError, WastLanguage,
    },
};

use super::{MarkdownDialect, MarkdownDocument, MarkdownOverlay};

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
            Self::Wast(_) => NativeLanguage::Wast,
        }
    }
}

/// Find a parser without discarding its configuration.
pub type MarkdownCodeParserResolver = dyn Fn(&str) -> Option<MarkdownCodeParser> + Send + Sync;

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
}

impl MarkdownParserConfig {
    /// Resolve the grammar for a raw info string without the fence markers.
    /// Use [`Self::code_parser`] to retain the parser configuration.
    pub fn code_language(&self, info: &str) -> Option<NativeLanguage> {
        self.code_parser(info).map(|parser| parser.language())
    }

    /// Resolve the named parser before either fallback. An empty info string
    /// bypasses named selection.
    pub fn code_parser(&self, info: &str) -> Option<MarkdownCodeParser> {
        let name = (!info.is_empty()).then(|| {
            info.split_once(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
                .map_or(info, |(name, _)| name)
        });
        let parser = match &self.code_languages {
            Some(MarkdownCodeLanguages::Catalog(catalog)) => name
                .and_then(|name| LanguageDescription::match_name(catalog, name, true))
                .and_then(LanguageDescription::built_in)
                .map(MarkdownCodeParser::Native),
            Some(MarkdownCodeLanguages::Resolve(resolve)) => name
                .and_then(|name| resolve(name))
                .map(MarkdownCodeParser::Native),
            Some(MarkdownCodeLanguages::Configured { resolve, fallback }) => name
                .and_then(|name| resolve(name))
                .or_else(|| fallback.clone()),
            None => None,
        };
        parser.or(self.default_code_language.map(MarkdownCodeParser::Native))
    }
}

/// An embedded parser result and its source map into the Markdown document.
pub struct MarkdownMount {
    code_parser: Option<MarkdownCodeParser>,
    syntax: NativeSyntax,
    segments: Vec<Segment>,
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

/// Changes in the difference between original bytes and default CodeMirror
/// UTF-16 positions. Store only the ends of characters and line endings that
/// change the difference. A snapshot builds this index only for a predicate.
#[derive(Debug)]
pub(super) struct EditorOffsets(Box<[(usize, usize)]>);

impl EditorOffsets {
    pub(super) fn new(source: &str) -> Self {
        let mut offsets = Vec::new();
        let mut delta = 0;
        for (at, ch) in source.char_indices() {
            let width = ch.len_utf8();
            let crlf = usize::from(ch == '\r' && source.as_bytes().get(at + 1) == Some(&b'\n'));
            let removed = width - ch.len_utf16() + crlf;
            if removed > 0 {
                delta += removed;
                let end = at + width + crlf;
                offsets.push((end, end - delta));
            }
        }
        Self(offsets.into_boxed_slice())
    }

    pub(super) fn editor(&self, byte: usize) -> usize {
        let i = self.0.partition_point(|&(end, _)| end <= byte);
        byte - self.delta(i)
    }

    pub(super) fn source(&self, editor: usize) -> usize {
        let i = self.0.partition_point(|&(_, end)| end <= editor);
        // Between two surrogates this yields a byte inside the scalar. Only
        // compare that byte to node bounds; never use it to slice the text.
        editor + self.delta(i)
    }

    fn delta(&self, i: usize) -> usize {
        i.checked_sub(1).map_or(0, |i| {
            let (byte, editor) = self.0[i];
            byte - editor
        })
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
            move |info| {
                resolve(info.split_whitespace().next().unwrap_or(""))
                    .map(MarkdownCodeParser::Native)
            },
            StreamSettings::default(),
        )
        .filter_map(Result::ok)
    }

    pub(crate) fn mounts_with_settings<'a>(
        &'a self,
        resolve: impl Fn(&str) -> Option<MarkdownCodeParser> + 'a,
        settings: StreamSettings,
    ) -> impl Iterator<Item = Result<MarkdownMount, ParseError>> + 'a {
        self.overlays().filter_map(move |overlay| {
            let (language, ranges) = match overlay {
                MarkdownOverlay::Code { info, ranges } => (Some(resolve(info)?), ranges),
                MarkdownOverlay::Html { ranges } => (None, ranges),
            };
            let mut input = String::new();
            let mut segments = Vec::with_capacity(ranges.len());
            for source in ranges.into_iter().filter(|range| !range.is_empty()) {
                let start = input.len();
                input.push_str(self.text().slice(source.clone())?);
                segments.push(Segment { source, start });
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
            Some(Ok(MarkdownMount {
                code_parser: language,
                syntax,
                segments,
            }))
        })
    }
}

impl MarkdownMount {
    pub fn language(&self) -> NativeLanguage {
        self.code_parser
            .as_ref()
            .map_or(NativeLanguage::Html, MarkdownCodeParser::language)
    }

    pub(crate) fn uses_wast_parser(&self, expected: &WastLanguage) -> bool {
        matches!(&self.code_parser, Some(MarkdownCodeParser::Wast(parser)) if parser == expected)
    }

    pub(crate) fn has_configured_folds(&self) -> bool {
        self.wast().is_some()
            || matches!(
                &self.code_parser,
                Some(MarkdownCodeParser::Json(_) | MarkdownCodeParser::Lezer(_))
            )
    }

    /// Syntax positions are relative to the concatenated embedded source.
    pub fn syntax(&self) -> &NativeSyntax {
        &self.syntax
    }

    /// The Markdown byte ranges consumed by this parser, in input order.
    pub fn source_ranges(&self) -> impl ExactSizeIterator<Item = Range<usize>> + '_ {
        self.segments.iter().map(|segment| segment.source.clone())
    }

    pub(crate) fn source_map(&self) -> MarkdownSourceMap<'_> {
        MarkdownSourceMap(&self.segments)
    }

    pub(crate) fn wast(&self) -> Option<&WastDocument> {
        match &self.syntax {
            NativeSyntax::Wast(doc) => Some(doc),
            _ => None,
        }
    }

    /// Project embedded highlights into the original Markdown. If a token
    /// crosses excluded markup, split its span at that markup.
    pub fn highlights(&self) -> impl Iterator<Item = HighlightSpan<'_>> {
        self.syntax
            .highlights(0..self.syntax.text().len())
            .into_iter()
            .flatten()
            .flat_map(|span| {
                let start = self
                    .segments
                    .partition_point(|segment| segment.parsed_end() <= span.range.start);
                self.segments[start..]
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
