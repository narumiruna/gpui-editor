use std::fmt;

use crate::{
    ChangeSet, EditError,
    language::{
        StreamUpdateError,
        lezer::{ParseError, ReuseStats},
    },
    stream_parser::TokenizeError,
};

use super::{NativeLanguage, NativeSyntax, SupportedDocument, markdown_mounts};

/// An incremental update that did not change the current syntax snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntaxUpdateError {
    Edit(EditError),
    Tokenize(TokenizeError),
    StrictSyntax(ParseError),
}

impl fmt::Display for SyntaxUpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit(error) => write!(f, "invalid syntax document edit: {error:?}"),
            Self::Tokenize(error) => error.fmt(f),
            Self::StrictSyntax(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SyntaxUpdateError {}

impl From<EditError> for SyntaxUpdateError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

impl From<StreamUpdateError> for SyntaxUpdateError {
    fn from(error: StreamUpdateError) -> Self {
        match error {
            StreamUpdateError::Edit(error) => Self::Edit(error),
            StreamUpdateError::Tokenize(error) => Self::Tokenize(error),
        }
    }
}

impl SupportedDocument {
    /// Apply edits in old-document byte coordinates. Reuse validated parser
    /// state when the current grammar supports it. Otherwise parse a new
    /// snapshot. An invalid edit or a strict mount failure leaves the existing
    /// syntax and mounts intact.
    pub fn apply_changes(&mut self, changes: &ChangeSet) -> Result<(), SyntaxUpdateError> {
        if changes.is_empty() {
            return Ok(());
        }
        match &mut self.syntax {
            NativeSyntax::Asn1(doc) => doc.apply_changes(changes).map_err(Into::into),
            NativeSyntax::Legacy(doc) => doc.apply_changes(changes).map_err(Into::into),
            _ => {
                *self = self.reparse(changes)?;
                Ok(())
            }
        }
    }

    /// Create a new snapshot without changing the one held by the old state.
    pub(crate) fn reparse(&self, changes: &ChangeSet) -> Result<Self, SyntaxUpdateError> {
        let syntax = match &self.syntax {
            NativeSyntax::Asn1(_) | NativeSyntax::Legacy(_) => {
                // Stream checkpoints support in-place updates only.
                let mut text = self.text().clone();
                changes.apply_to(&mut text)?;
                self.language
                    .parse_with_stream_settings(text, self.settings)
                    .map_err(SyntaxUpdateError::Tokenize)?
            }
            NativeSyntax::Angular(doc) => NativeSyntax::Angular(doc.reparse(changes)?),
            NativeSyntax::Cpp(doc) => NativeSyntax::Cpp(doc.reparse(changes)?),
            NativeSyntax::Css(doc) => NativeSyntax::Css(doc.reparse(changes)?),
            NativeSyntax::Go(doc) => NativeSyntax::Go(doc.reparse(changes)?),
            NativeSyntax::Html(doc) => NativeSyntax::Html(doc.reparse(changes)?),
            NativeSyntax::Java(doc) => NativeSyntax::Java(doc.reparse(changes)?),
            NativeSyntax::Javascript(doc) => NativeSyntax::Javascript(doc.reparse(changes)?),
            NativeSyntax::Jinja(doc) => NativeSyntax::Jinja(doc.reparse(changes)?),
            NativeSyntax::Json(doc) => NativeSyntax::Json(doc.reparse(changes)?),
            NativeSyntax::Less(doc) => NativeSyntax::Less(doc.reparse(changes)?),
            NativeSyntax::LezerGrammar(doc) => NativeSyntax::LezerGrammar(doc.reparse(changes)?),
            NativeSyntax::Liquid(doc) => NativeSyntax::Liquid(doc.reparse(changes)?),
            NativeSyntax::Markdown(doc) => NativeSyntax::Markdown(doc.reparse(changes)?),
            NativeSyntax::Php(doc) => NativeSyntax::Php(Box::new(doc.reparse(changes)?)),
            NativeSyntax::Python(doc) => NativeSyntax::Python(doc.reparse(changes)?),
            NativeSyntax::Rust(doc) => NativeSyntax::Rust(doc.reparse(changes)?),
            NativeSyntax::Sass(doc) => NativeSyntax::Sass(doc.reparse(changes)?),
            NativeSyntax::Sql(doc) => {
                let NativeLanguage::Sql(dialect) = self.language else {
                    unreachable!("matched SQL syntax")
                };
                NativeSyntax::Sql(dialect.language().reparse(doc, changes)?)
            }
            NativeSyntax::Vue(doc) => NativeSyntax::Vue(doc.reparse(changes)?),
            NativeSyntax::Wast(doc) => {
                NativeSyntax::Wast(if let Some(config) = &self.wast_config {
                    let mut text = doc.text().clone();
                    changes.apply_to(&mut text)?;
                    config
                        .parse_document(text)
                        .map_err(SyntaxUpdateError::StrictSyntax)?
                } else {
                    doc.reparse(changes)?
                })
            }
            NativeSyntax::Xml(doc) => NativeSyntax::Xml(doc.reparse(changes)?),
            NativeSyntax::Yaml(doc) => NativeSyntax::Yaml(doc.reparse(changes)?),
        };
        Ok(Self {
            language: self.language,
            settings: self.settings,
            markdown_parser: self.markdown_parser.clone(),
            wast_config: self.wast_config.clone(),
            markdown_mounts: markdown_mounts(
                &syntax,
                self.settings,
                self.markdown_parser.as_deref(),
            )
            .map_err(SyntaxUpdateError::StrictSyntax)?,
            syntax,
        })
    }

    /// Lezer checkpoint reuse for the current syntax tree. Stream modes keep
    /// line state instead, and do not report it in these counts.
    pub fn reuse_stats(&self) -> ReuseStats {
        match &self.syntax {
            NativeSyntax::Angular(doc) => doc.reuse_stats(),
            NativeSyntax::Cpp(doc) => doc.reuse_stats(),
            NativeSyntax::Css(doc) => doc.reuse_stats(),
            NativeSyntax::Go(doc) => doc.reuse_stats(),
            NativeSyntax::Html(doc) => doc.reuse_stats(),
            NativeSyntax::Java(doc) => doc.reuse_stats(),
            NativeSyntax::Javascript(doc) => doc.reuse_stats(),
            NativeSyntax::Jinja(doc) => doc.reuse_stats(),
            NativeSyntax::Json(doc) => doc.reuse_stats(),
            NativeSyntax::Less(doc) => doc.reuse_stats(),
            NativeSyntax::LezerGrammar(doc) => doc.reuse_stats(),
            NativeSyntax::Liquid(doc) => doc.reuse_stats(),
            NativeSyntax::Php(doc) => doc.reuse_stats(),
            NativeSyntax::Python(doc) => doc.reuse_stats(),
            NativeSyntax::Rust(doc) => doc.reuse_stats(),
            NativeSyntax::Sass(doc) => doc.reuse_stats(),
            NativeSyntax::Sql(doc) => doc.reuse_stats(),
            NativeSyntax::Vue(doc) => doc.reuse_stats(),
            NativeSyntax::Wast(doc) => doc.reuse_stats(),
            NativeSyntax::Xml(doc) => doc.reuse_stats(),
            NativeSyntax::Yaml(doc) => doc.reuse_stats(),
            NativeSyntax::Asn1(_) | NativeSyntax::Legacy(_) | NativeSyntax::Markdown(_) => {
                ReuseStats::default()
            }
        }
    }
}
