//! Language catalog from `@codemirror/language-data` 6.5.2.
//! See `docs/licenses/codemirror-language-data.txt` for the upstream MIT notice.
//!
//! The catalog includes metadata and upstream load sources. Call
//! [`LanguageDescription::built_in`] for bundled native languages, or use
//! [`NativeLoaders`] to register additional implementations.

use std::{collections::HashMap, fmt, ops::Range, sync::LazyLock};

use regex::Regex;

use crate::{
    Extension, Text,
    commands::CommentTokens,
    language::{
        HighlightSpan, IndentOptions, StreamDocument, StreamSettings,
        angular::AngularDocument,
        cpp::CppDocument,
        css::CssDocument,
        go::GoDocument,
        html::HtmlDocument,
        java::JavaDocument,
        javascript::{JavascriptDocument, JavascriptOptions},
        jinja::JinjaDocument,
        json::JsonDocument,
        less::LessDocument,
        lezer_grammar::LezerDocument,
        liquid::LiquidDocument,
        markdown::MarkdownDocument,
        php::PhpDocument,
        python::PythonDocument,
        rust::RustDocument,
        sass::{SassDialect, SassDocument},
        sql::{SqlDialect, SqlDocument},
        vue::VueDocument,
        wast::WastDocument,
        xml::XmlDocument,
        yaml::YamlDocument,
    },
    legacy_modes::{LegacyMode, asn1, asn1::Asn1},
    stream_parser::TokenizeError,
};

mod async_load;
mod catalog;
mod load;
mod support;

pub use async_load::{LanguageLoadFuture, NativeAsyncLanguageLoad};
pub use catalog::LANGUAGES;
pub use load::NativeLanguageLoad;
pub use support::{
    CloseBracketData, GoCompletionSource, NativeLanguageSupport, SupportedDocument,
    SyntaxUpdateError, WastLanguage, attached_language_snapshot, go_extension,
    go_language_extension, wast_language,
};
pub(crate) use support::{WastConfig, active_wast_viewport};

/// Install JSON metadata and track syntax for state-only indentation and folds,
/// setup bracket matching, and syntax-aware automatic quote closing.
/// This is opt-in. It does not install linting, completion, key bindings,
/// bracket matching, automatic bracket closing, or reindentation on input.
pub fn json_extension() -> Extension {
    static EXTENSION: LazyLock<Extension> =
        LazyLock::new(|| NativeLanguage::Json.support().tracked_syntax_extension());
    EXTENSION.clone()
}

/// Attach the Lezer grammar language to the whole editor state. Track the
/// active syntax and metadata without installing any keys, matching, or input
/// handlers. See [`attached_language_snapshot`] for the current snapshot.
pub fn lezer_extension() -> Extension {
    static EXTENSION: LazyLock<Extension> = LazyLock::new(|| {
        NativeLanguage::LezerGrammar
            .support()
            .tracked_syntax_extension()
    });
    EXTENSION.clone()
}

/// Install WAST metadata and synchronous syntax for state-only indentation,
/// folds, bracket matching in the setup preset, and automatic quote closing.
/// Use this instead of the metadata-only extension. Loading WAST from the catalog
/// does not install it. Queries use the current tracked snapshot without parsing.
/// The native parser's recovery and LF, CRLF, and lone CR line rules still apply;
/// a caller-defined separator is not a new syntax line break.
pub fn wast_extension() -> Extension {
    wast_language().extension()
}

/// One item in the ordered upstream language catalog.
#[derive(Clone, Copy, Debug)]
pub struct LanguageDescription {
    pub name: &'static str,
    /// Alternative names in upstream order. The published catalog includes
    /// the lowercased canonical name. Custom descriptions can omit it and
    /// supply aliases in their original case.
    pub aliases: &'static [&'static str],
    /// File extensions without the leading dot, in upstream order.
    pub extensions: &'static [&'static str],
    pub filename: Option<FilenamePattern>,
    pub source: LoadSource,
}

/// A language with a parser bundled in this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeLanguage {
    Angular,
    Asn1,
    Cpp,
    Java,
    Less,
    Liquid,
    Php,
    Sass(SassDialect),
    Css,
    Go,
    Html,
    Javascript(JavascriptOptions),
    Jinja,
    Json,
    LezerGrammar,
    Markdown,
    Python,
    Rust,
    Sql(SqlDialect),
    Vue,
    Wast,
    Xml,
    Yaml,
    Legacy(LegacyMode),
}

impl NativeLanguage {
    /// Compose the bundled parser with the native default editor services.
    pub fn support(self) -> NativeLanguageSupport {
        NativeLanguageSupport::new(self)
    }

    /// Whether syntax services treat lone CR as a logical line break.
    pub fn uses_projected_lines(self) -> bool {
        matches!(
            self,
            Self::Cpp
                | Self::Go
                | Self::Java
                | Self::Json
                | Self::LezerGrammar
                | Self::Rust
                | Self::Wast
                | Self::Yaml
        )
    }

    /// Additional word characters in the outer language, if configured.
    pub fn word_characters(self) -> Option<&'static str> {
        use crate::language::{css, html, javascript, less, php, sass};
        match self {
            Self::Css => Some(css::WORD_CHARS),
            Self::Html => Some(html::WORD_CHARS),
            Self::Javascript(_) => Some(javascript::WORD_CHARS),
            Self::Less => Some(less::WORD_CHARS),
            Self::Php => Some(php::WORD_CHARS),
            Self::Sass(_) => Some(sass::WORD_CHARS),
            Self::Legacy(mode) => mode
                .language()
                .language_data()
                .get::<&'static str>("wordChars")
                .copied(),
            _ => None,
        }
    }

    /// Comment markers in the outer language. Embedded parsers can supply
    /// other markers at their own positions.
    pub fn comment_tokens(self) -> CommentTokens<'static> {
        use crate::language::{cpp, css, html, less, liquid, python, sql, wast};
        let (line, block) = match self {
            Self::Cpp
            | Self::Go
            | Self::Java
            | Self::Javascript(_)
            | Self::LezerGrammar
            | Self::Php
            | Self::Rust
            | Self::Sass(_) => (Some(cpp::LINE_COMMENT), Some(cpp::BLOCK_COMMENT)),
            Self::Css => (None, Some(css::BLOCK_COMMENT)),
            Self::Angular | Self::Html | Self::Markdown | Self::Vue | Self::Xml => {
                (None, Some(html::BLOCK_COMMENT))
            }
            Self::Less => (Some(less::LINE_COMMENT), Some(less::BLOCK_COMMENT)),
            Self::Liquid => (Some(liquid::LINE_COMMENT), None),
            Self::Python | Self::Yaml => (Some(python::LINE_COMMENT), None),
            Self::Sql(_) => (Some(sql::LINE_COMMENT), Some(sql::BLOCK_COMMENT)),
            Self::Wast => (Some(wast::LINE_COMMENT), Some(wast::BLOCK_COMMENT)),
            Self::Legacy(mode) => {
                let data = mode.language().language_data();
                return data
                    .get::<CommentTokens>("commentTokens")
                    .copied()
                    .or_else(|| {
                        data.get::<crate::legacy_modes::CommentTokens>("commentTokens")
                            .map(|tokens| CommentTokens {
                                line: Some(tokens.line),
                                block: None,
                            })
                    })
                    .unwrap_or_default();
            }
            Self::Asn1 => (Some(asn1::LINE_COMMENT), None),
            Self::Json | Self::Jinja => (None, None),
        };
        CommentTokens { line, block }
    }

    pub fn parse(self, text: Text) -> Result<NativeSyntax, TokenizeError> {
        self.parse_with_stream_settings(text, StreamSettings::default())
    }

    pub fn parse_with_stream_settings(
        self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<NativeSyntax, TokenizeError> {
        Ok(match self {
            Self::Angular => NativeSyntax::Angular(AngularDocument::parse(text)),
            Self::Asn1 => {
                NativeSyntax::Asn1(Box::new(asn1::default_language().parse(text, settings)?))
            }
            Self::Cpp => NativeSyntax::Cpp(CppDocument::parse(text)),
            Self::Java => NativeSyntax::Java(JavaDocument::parse(text)),
            Self::Less => NativeSyntax::Less(LessDocument::parse(text)),
            Self::Liquid => NativeSyntax::Liquid(LiquidDocument::parse(text)),
            Self::Php => NativeSyntax::Php(Box::new(PhpDocument::parse(text))),
            Self::Sass(dialect) => NativeSyntax::Sass(SassDocument::parse(text, dialect)),
            Self::Css => NativeSyntax::Css(CssDocument::parse(text)),
            Self::Go => NativeSyntax::Go(GoDocument::parse(text)),
            Self::Html => NativeSyntax::Html(HtmlDocument::parse(text)),
            Self::Javascript(options) => {
                NativeSyntax::Javascript(JavascriptDocument::parse_with_options(text, options))
            }
            Self::Jinja => NativeSyntax::Jinja(JinjaDocument::parse(text)),
            Self::Json => NativeSyntax::Json(JsonDocument::parse(text)),
            Self::LezerGrammar => NativeSyntax::LezerGrammar(LezerDocument::parse(text)),
            Self::Markdown => NativeSyntax::Markdown(MarkdownDocument::parse(text)),
            Self::Python => NativeSyntax::Python(PythonDocument::parse(text)),
            Self::Rust => NativeSyntax::Rust(RustDocument::parse(text)),
            Self::Sql(dialect) => NativeSyntax::Sql(SqlDocument::parse_with_dialect(text, dialect)),
            Self::Vue => NativeSyntax::Vue(VueDocument::parse(text)),
            Self::Wast => NativeSyntax::Wast(WastDocument::parse(text)),
            Self::Xml => NativeSyntax::Xml(XmlDocument::parse(text)),
            Self::Yaml => NativeSyntax::Yaml(YamlDocument::parse(text)),
            Self::Legacy(mode) => {
                NativeSyntax::Legacy(Box::new(mode.language().parse(text, settings)?))
            }
        })
    }
}

/// The parsed document for a bundled language, with a shared highlighting API.
pub enum NativeSyntax {
    Angular(AngularDocument),
    Asn1(Box<StreamDocument<'static, Asn1>>),
    Cpp(CppDocument),
    Java(JavaDocument),
    Less(LessDocument),
    Liquid(LiquidDocument),
    Php(Box<PhpDocument>),
    Sass(SassDocument),
    Css(CssDocument),
    Go(GoDocument),
    Html(HtmlDocument),
    Javascript(JavascriptDocument),
    Jinja(JinjaDocument),
    Json(JsonDocument),
    LezerGrammar(LezerDocument),
    Markdown(MarkdownDocument),
    Python(PythonDocument),
    Rust(RustDocument),
    Sql(SqlDocument),
    Vue(VueDocument),
    Wast(WastDocument),
    Xml(XmlDocument),
    Yaml(YamlDocument),
    Legacy(Box<StreamDocument<'static, LegacyMode>>),
}

impl NativeSyntax {
    pub fn text(&self) -> &Text {
        match self {
            Self::Angular(doc) => doc.text(),
            Self::Cpp(doc) => doc.text(),
            Self::Java(doc) => doc.text(),
            Self::Less(doc) => doc.text(),
            Self::Liquid(doc) => doc.text(),
            Self::Php(doc) => doc.text(),
            Self::Sass(doc) => doc.text(),
            Self::Css(doc) => doc.text(),
            Self::Go(doc) => doc.text(),
            Self::Html(doc) => doc.text(),
            Self::Javascript(doc) => doc.text(),
            Self::Jinja(doc) => doc.text(),
            Self::Json(doc) => doc.text(),
            Self::LezerGrammar(doc) => doc.text(),
            Self::Markdown(doc) => doc.text(),
            Self::Python(doc) => doc.text(),
            Self::Rust(doc) => doc.text(),
            Self::Sql(doc) => doc.text(),
            Self::Vue(doc) => doc.text(),
            Self::Wast(doc) => doc.text(),
            Self::Xml(doc) => doc.text(),
            Self::Yaml(doc) => doc.text(),
            Self::Legacy(doc) => doc.text(),
            Self::Asn1(doc) => doc.text(),
        }
    }

    /// Logical lines for languages that recognize lone CR as a break.
    /// With no lone CR, these languages return the original text.
    pub fn syntax_lines(&self) -> Option<&Text> {
        let lines = match self {
            Self::Cpp(doc) => doc.syntax_lines(),
            Self::Go(doc) => doc.syntax_lines(),
            Self::Java(doc) => doc.syntax_lines(),
            Self::Json(doc) => doc.syntax_lines(),
            Self::LezerGrammar(doc) => doc.syntax_lines(),
            Self::Rust(doc) => doc.syntax_lines(),
            Self::Wast(doc) => doc.syntax_lines(),
            Self::Yaml(doc) => doc.syntax_lines(),
            _ => return None,
        };
        Some(lines)
    }

    /// Same-width logical lines when they differ from the original text.
    pub fn projected_lines(&self) -> Option<&Text> {
        self.syntax_lines()
            .filter(|lines| !std::ptr::eq(*lines, self.text()))
    }

    /// Check for a gap between adjacent syntax delimiters. The newline
    /// command validates whitespace, line bounds, and the selection.
    pub fn bracket_gap_for_indent(&self, position: usize) -> Option<Range<usize>> {
        use crate::language::json::JsonNodeKind::*;
        match self {
            Self::Go(doc) => doc.bracket_gap_for_indent(position),
            Self::LezerGrammar(doc) => doc.bracket_gap_for_indent(position),
            Self::Wast(doc) => doc.bracket_gap_for_indent(position),
            Self::Json(doc) => {
                let node = doc.node_at(position, None)?;
                let mut children = node.children();
                let open = children.next()?;
                let close = children.next()?;
                (children.len() == 0
                    && matches!(
                        (open.kind(), close.kind()),
                        (OpenBrace, CloseBrace) | (OpenBracket, CloseBracket)
                    ))
                .then(|| open.range().end..close.range().start)
            }
            _ => None,
        }
    }

    /// Query syntax indentation at a UTF-8 byte position. Stream parsers use
    /// the settings from parsing; grammar adapters use `settings` here.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        if !self.text().is_char_boundary(position) {
            return None;
        }
        match self {
            Self::Angular(doc) => doc.indentation(position, settings, options),
            Self::Cpp(doc) => doc.indentation(position, settings, options),
            Self::Java(doc) => doc.indentation(position, settings, options),
            Self::Less(doc) => doc.indentation(position, settings, options),
            Self::Liquid(doc) => doc.indentation(position, settings, options),
            Self::Php(doc) => doc.indentation(position, settings, options),
            Self::Sass(doc) => doc.indentation(position, settings, options),
            Self::Css(doc) => doc.indentation(position, settings, options),
            Self::Go(doc) => doc.indentation(position, settings, options),
            Self::Html(doc) => doc.indentation(position, settings, options),
            Self::Javascript(doc) => doc.indentation(position, settings, options),
            Self::Jinja(doc) => doc.indentation(position, settings, options),
            Self::Json(doc) => doc.indentation(position, settings, options),
            Self::LezerGrammar(doc) => doc.indentation(position, settings, options),
            Self::Markdown(doc) => doc.indentation(position),
            Self::Python(doc) => doc.indentation(position, settings, options),
            Self::Rust(doc) => doc.indentation(position, settings, options),
            Self::Sql(doc) => doc.indentation(position, settings, options),
            Self::Vue(doc) => doc.indentation(position, settings, options),
            Self::Wast(doc) => doc.indentation(position, settings, options),
            Self::Xml(doc) => doc.indentation(position, settings, options),
            Self::Yaml(doc) => doc.indentation(position, settings, options),
            Self::Legacy(doc) => doc.indentation(position, options).ok().flatten(),
            Self::Asn1(doc) => doc.indentation(position, options).ok().flatten(),
        }
    }

    /// Get a syntax fold for a logical line in this document.
    pub fn foldable(&self, start: usize, end: usize) -> Option<Range<usize>> {
        if !self.text().is_valid_range(start..end) {
            return None;
        }
        match self {
            Self::Angular(doc) => doc.foldable(start, end),
            Self::Cpp(doc) => doc.foldable(start, end),
            Self::Java(doc) => doc.foldable(start, end),
            Self::Less(doc) => doc.foldable(start, end),
            Self::Liquid(doc) => doc.foldable(start, end),
            Self::Php(doc) => doc.foldable(start, end),
            Self::Sass(doc) => doc.foldable(start, end),
            Self::Css(doc) => doc.foldable(start, end),
            Self::Go(doc) => doc.foldable(start, end),
            Self::Html(doc) => doc.foldable(start, end),
            Self::Javascript(doc) => doc.foldable(start, end),
            Self::Jinja(doc) => doc.foldable(start, end),
            Self::Json(doc) => doc.foldable(start, end),
            Self::LezerGrammar(doc) => doc.foldable(start, end),
            Self::Markdown(doc) => doc.foldable(start, end),
            Self::Python(doc) => doc.foldable(start, end),
            Self::Rust(doc) => doc.foldable(start, end),
            Self::Sql(doc) => doc.foldable(start, end),
            Self::Vue(doc) => doc.foldable(start, end),
            Self::Wast(doc) => doc.foldable(start, end),
            Self::Xml(doc) => doc.foldable(start, end),
            Self::Yaml(doc) => doc.foldable(start, end),
            Self::Legacy(_) | Self::Asn1(_) => None,
        }
    }

    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<Box<dyn Iterator<Item = HighlightSpan<'_>> + '_>> {
        match self {
            Self::Angular(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Cpp(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Java(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Less(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Liquid(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Php(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Sass(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Css(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Go(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Html(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Javascript(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Jinja(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Json(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::LezerGrammar(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Markdown(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Python(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Rust(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Sql(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Vue(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Wast(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Xml(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Yaml(doc) => Some(Box::new(doc.highlights(range)?)),
            Self::Legacy(doc) => Some(Box::new(doc.tokens(range)?)),
            Self::Asn1(doc) => Some(Box::new(doc.tokens(range)?)),
        }
    }
}

impl LanguageDescription {
    /// Resolve a bundled native language. Other descriptions still require a loader.
    pub fn built_in(&self) -> Option<NativeLanguage> {
        match (
            self.source.module,
            self.source.export,
            self.source.invocation,
        ) {
            ("@codemirror/lang-angular", "angular", LoadInvocation::Call) => {
                Some(NativeLanguage::Angular)
            }
            ("@codemirror/lang-cpp", "cpp", LoadInvocation::Call) => Some(NativeLanguage::Cpp),
            ("@codemirror/lang-java", "java", LoadInvocation::Call) => Some(NativeLanguage::Java),
            ("@codemirror/lang-less", "less", LoadInvocation::Call) => Some(NativeLanguage::Less),
            ("@codemirror/lang-liquid", "liquid", LoadInvocation::Call) => {
                Some(NativeLanguage::Liquid)
            }
            ("@codemirror/lang-php", "php", LoadInvocation::Call) => Some(NativeLanguage::Php),
            ("@codemirror/lang-sass", "sass", LoadInvocation::Call) => {
                Some(NativeLanguage::Sass(SassDialect::Scss))
            }
            ("@codemirror/lang-sass", "sass", LoadInvocation::SassIndented) => {
                Some(NativeLanguage::Sass(SassDialect::Indented))
            }
            ("@codemirror/lang-css", "css", LoadInvocation::Call) => Some(NativeLanguage::Css),
            ("@codemirror/lang-go", "go", LoadInvocation::Call) => Some(NativeLanguage::Go),
            ("@codemirror/lang-html", "html", LoadInvocation::Call) => Some(NativeLanguage::Html),
            ("@codemirror/lang-javascript", "javascript", LoadInvocation::Call) => {
                Some(NativeLanguage::Javascript(JavascriptOptions::default()))
            }
            (
                "@codemirror/lang-javascript",
                "javascript",
                LoadInvocation::JavaScript { jsx, typescript },
            ) => Some(NativeLanguage::Javascript(JavascriptOptions {
                jsx,
                typescript,
            })),
            ("@codemirror/lang-sql", "sql", LoadInvocation::SqlDialect(dialect)) => {
                SqlDialect::from_export(dialect).map(NativeLanguage::Sql)
            }
            ("@codemirror/lang-jinja", "jinja", LoadInvocation::Call) => {
                Some(NativeLanguage::Jinja)
            }
            ("@codemirror/lang-json", "json", LoadInvocation::Call) => Some(NativeLanguage::Json),
            ("@codemirror/lang-markdown", "markdown", LoadInvocation::Call) => {
                Some(NativeLanguage::Markdown)
            }
            ("@codemirror/lang-python", "python", LoadInvocation::Call) => {
                Some(NativeLanguage::Python)
            }
            ("@codemirror/lang-rust", "rust", LoadInvocation::Call) => Some(NativeLanguage::Rust),
            ("@codemirror/lang-vue", "vue", LoadInvocation::Call) => Some(NativeLanguage::Vue),
            ("@codemirror/lang-wast", "wast", LoadInvocation::Call) => Some(NativeLanguage::Wast),
            ("@codemirror/lang-xml", "xml", LoadInvocation::Call) => Some(NativeLanguage::Xml),
            ("@codemirror/lang-yaml", "yaml", LoadInvocation::Call) => Some(NativeLanguage::Yaml),
            ("@codemirror/legacy-modes/mode/asn1", "asn1", LoadInvocation::EmptyOptions) => {
                Some(NativeLanguage::Asn1)
            }
            (module, export, LoadInvocation::Export) => {
                LegacyMode::from_export(module, export).map(NativeLanguage::Legacy)
            }
            _ => None,
        }
    }

    /// Create a reusable load handle for this description's bundled support.
    pub fn native_load(&self) -> NativeLanguageLoad<'_, NativeLanguage, UnavailableLanguage> {
        NativeLanguageLoad::new(self, |description| {
            description.built_in().ok_or(UnavailableLanguage {
                name: description.name,
                source: description.source,
            })
        })
    }

    /// Create a reusable load handle for the catalog's composed services.
    pub fn native_support_load(
        &self,
    ) -> NativeLanguageLoad<'_, NativeLanguageSupport, UnavailableLanguage> {
        NativeLanguageLoad::new(self, |description| {
            description
                .built_in()
                .map(NativeLanguage::support)
                .ok_or(UnavailableLanguage {
                    name: description.name,
                    source: description.source,
                })
        })
    }

    /// Create a shared future for loading the catalog's composed services.
    /// Bundled loaders do not wait for I/O.
    pub fn native_support_load_async(
        &self,
    ) -> NativeAsyncLanguageLoad<'_, NativeLanguageSupport, UnavailableLanguage> {
        NativeAsyncLanguageLoad::new(self, |description| async move {
            description
                .built_in()
                .map(NativeLanguage::support)
                .ok_or(UnavailableLanguage {
                    name: description.name,
                    source: description.source,
                })
        })
    }

    /// Return the first filename pattern match, or the first case-sensitive
    /// extension match. Use the input as given; do not extract a basename.
    pub fn match_filename<'a>(descriptions: &'a [Self], filename: &str) -> Option<&'a Self> {
        if let Some(description) = descriptions
            .iter()
            .find(|d| d.filename.is_some_and(|pattern| pattern.is_match(filename)))
        {
            return Some(description);
        }

        let (_, extension) = filename.rsplit_once('.')?;
        if extension.is_empty() {
            return None;
        }
        descriptions
            .iter()
            .find(|d| d.extensions.contains(&extension))
    }

    /// Return the first case-insensitive exact name or alias match. If there
    /// is no exact match and `fuzzy` is true, search for one in the input.
    /// Match custom aliases as if `LanguageDescription.of` had lowercased
    /// them and appended the canonical name. Upstream defaults to fuzzy mode.
    pub fn match_name<'a>(descriptions: &'a [Self], name: &str, fuzzy: bool) -> Option<&'a Self> {
        let name = lowercase_name(name);
        if let Some(description) = descriptions.iter().find(|description| {
            description
                .names()
                .any(|alias| lowercase_name(alias) == name)
        }) {
            return Some(description);
        }
        if !fuzzy {
            return None;
        }
        descriptions.iter().find(|description| {
            description
                .names()
                .any(|alias| fuzzy_match(&name, &lowercase_name(alias)))
        })
    }

    fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.aliases.iter().copied().chain([self.name])
    }
}

fn lowercase_name(value: &str) -> std::borrow::Cow<'_, str> {
    if value
        .bytes()
        .any(|byte| !byte.is_ascii() || byte.is_ascii_uppercase())
    {
        std::borrow::Cow::Owned(value.to_lowercase())
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

fn fuzzy_match(name: &str, alias: &str) -> bool {
    // Upstream checks only the first occurrence of each alias.
    let Some(start) = name.find(alias) else {
        return false;
    };
    if alias.encode_utf16().count() > 2 {
        return true;
    }
    // JS tests each adjacent code unit with /\w/. On a missing side, it
    // tests the string "undefined", which also matches /\w/.
    let non_word = |ch: char| !ch.is_ascii_alphanumeric() && ch != '_';
    name[..start].chars().next_back().is_some_and(non_word)
        && name[start + alias.len()..]
            .chars()
            .next()
            .is_some_and(non_word)
}

/// An upstream JavaScript filename regular expression and its native matcher.
#[derive(Clone, Copy, Debug)]
pub struct FilenamePattern {
    source: &'static str,
    flags: &'static str,
    regex: &'static LazyLock<Regex>,
}

impl FilenamePattern {
    /// The exact upstream expression, without the slash delimiters.
    pub fn source(self) -> &'static str {
        self.source
    }

    /// The exact upstream regular expression flags.
    pub fn flags(self) -> &'static str {
        self.flags
    }

    pub fn is_match(self, filename: &str) -> bool {
        self.regex.is_match(filename)
    }
}

/// The module, export, and invocation used by the upstream loader.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LoadSource {
    /// An npm package or a subpath in `@codemirror/legacy-modes`.
    pub module: &'static str,
    pub export: &'static str,
    pub invocation: LoadInvocation,
}

/// How the upstream loader uses the module export.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LoadInvocation {
    /// Use the exported legacy mode without calling it.
    Export,
    /// Call the factory without arguments.
    Call,
    /// Call the factory with an empty options object.
    EmptyOptions,
    JavaScript {
        jsx: bool,
        typescript: bool,
    },
    SassIndented,
    /// Pass the named `@codemirror/lang-sql` dialect export to the SQL factory.
    SqlDialect(&'static str),
}

/// Caller-supplied native loaders keyed by their upstream source.
/// A new registry has no loaders. Each load calls the factory without caching.
pub struct NativeLoaders<T> {
    loaders: HashMap<LoadSource, Box<dyn Fn() -> T + Send + Sync>>,
}

impl<T> Default for NativeLoaders<T> {
    fn default() -> Self {
        Self {
            loaders: HashMap::new(),
        }
    }
}

impl<T> NativeLoaders<T> {
    /// Add a factory or replace one registered for the same source.
    pub fn register(&mut self, source: LoadSource, loader: impl Fn() -> T + Send + Sync + 'static) {
        self.loaders.insert(source, Box::new(loader));
    }

    /// Return true if the caller has registered a factory for this language.
    pub fn is_available(&self, language: &LanguageDescription) -> bool {
        self.loaders.contains_key(&language.source)
    }

    /// Create a per-description load handle using this registry's factories.
    pub fn cached<'a>(
        &'a self,
        language: &'a LanguageDescription,
    ) -> NativeLanguageLoad<'a, T, UnavailableLanguage> {
        NativeLanguageLoad::new(language, |description| self.load(description))
    }

    /// Load this language, or report that its source has no native factory.
    pub fn load(&self, language: &LanguageDescription) -> Result<T, UnavailableLanguage> {
        let loader = self
            .loaders
            .get(&language.source)
            .ok_or(UnavailableLanguage {
                name: language.name,
                source: language.source,
            })?;
        Ok(loader())
    }
}

/// No native loader is registered for the selected language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnavailableLanguage {
    pub name: &'static str,
    pub source: LoadSource,
}

impl fmt::Display for UnavailableLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "no native loader for {} ({})",
            self.name, self.source.module
        )
    }
}

impl std::error::Error for UnavailableLanguage {}
