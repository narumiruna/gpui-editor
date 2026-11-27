use std::{ops::Range, sync::Arc};

use crate::{
    Text,
    language::{
        HighlightSpan, StreamSettings,
        html::{HtmlDocument, HtmlOptions},
        lezer::ParseError,
        wast::{WastDocument, project_error},
    },
    language_data::{
        LanguageDescription, NativeLanguage, NativeSyntax, SupportedDocument, SyntaxUpdateError,
        WastLanguage,
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

/// A native code parser, including a WAST parser with its own configuration.
/// Equality compares the native language and, for WAST handles, configuration identity.
#[derive(Clone, PartialEq, Eq)]
pub enum MarkdownCodeParser {
    Native(NativeLanguage),
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
            Self::Wast(language) => language
                .parse_for_editor(text, settings)
                .map_err(SyntaxUpdateError::StrictSyntax),
        }
    }

    pub fn language(&self) -> NativeLanguage {
        match self {
            Self::Native(language) => *language,
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
    language: NativeLanguage,
    syntax: NativeSyntax,
    segments: Vec<Segment>,
}

struct Segment {
    source: Range<usize>,
    start: usize,
}

impl Segment {
    fn parsed_end(&self) -> usize {
        self.start + self.source.len()
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
                Some(MarkdownCodeParser::Wast(language)) => match language.parse_document(text) {
                    Ok(syntax) => NativeSyntax::Wast(syntax),
                    Err(error) => {
                        return Some(Err(project_error(error, |offset| {
                            // At a join, the next segment owns the byte. At EOF,
                            // report the end of the last segment.
                            let i =
                                segments.partition_point(|segment| segment.parsed_end() <= offset);
                            segments.get(i).map_or_else(
                                || segments.last().unwrap().source.end,
                                |segment| segment.source.start + offset - segment.start,
                            )
                        })));
                    }
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
                language: language.map_or(NativeLanguage::Html, |parser| parser.language()),
                syntax,
                segments,
            }))
        })
    }
}

impl MarkdownMount {
    pub fn language(&self) -> NativeLanguage {
        self.language
    }

    /// Syntax positions are relative to the concatenated embedded source.
    pub fn syntax(&self) -> &NativeSyntax {
        &self.syntax
    }

    /// The Markdown byte ranges consumed by this parser, in input order.
    pub fn source_ranges(&self) -> impl ExactSizeIterator<Item = Range<usize>> + '_ {
        self.segments.iter().map(|segment| segment.source.clone())
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
