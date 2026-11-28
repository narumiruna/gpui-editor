use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension, Text,
    language::{
        LanguageData, StreamSettings,
        lezer::{NodePropSource, NodeType, ParseError, SyntaxTree},
        lezer_grammar::{LezerParseJob, NAME},
    },
    language_data::{LrDataFacet, SupportedDocument, support::LrGrammar},
};

use super::lr::LrLanguage;

/// An installable Lezer grammar language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct LezerLanguage(LrLanguage);

/// The stock permissive Lezer grammar language.
pub fn lezer_language() -> &'static LezerLanguage {
    static LANGUAGE: LazyLock<LezerLanguage> =
        LazyLock::new(|| LezerLanguage(LrLanguage::new(LrGrammar::Lezer, NAME, false)));
    &LANGUAGE
}

impl LezerLanguage {
    /// Create a distinct parser identity with the same grammar and metadata.
    /// `None` keeps the previous setting, as does an empty name. Set strict
    /// to `Some(false)` to restore recovery.
    pub fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        Self(self.0.configure(strict, name))
    }

    /// Apply property sources in order to each grammar term. Keep the current
    /// name, strictness, and handle facet, and create a new parser identity.
    pub fn with_node_props(&self, sources: &[NodePropSource]) -> Self {
        Self(self.0.with_node_props(sources))
    }

    /// Return the immutable node-type rows in term order, including repeats.
    pub fn node_types(&self) -> impl ExactSizeIterator<Item = &NodeType> {
        self.0.node_types()
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The name of the active whole-document Lezer grammar attachment, if any.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Lezer)
    }

    /// Track editor syntax and metadata without installing optional input or
    /// key bindings. In strict mode a parse failure rejects the state change.
    /// A strict edit parses from the start rather than reusing checkpoints.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// Install the opt-in viewport parser in place of synchronous syntax.
    /// A strict parse failure stops the viewport job without publishing a tree.
    pub fn viewport_extension(&self) -> Option<Extension> {
        self.0.viewport_extension()
    }

    /// Start a bounded job with this handle's strictness and node rows.
    /// A strict prefix tree can contain zero-width errors from staged closure.
    pub fn parse_job(&self, text: Text) -> Option<LezerParseJob> {
        self.0.lezer_parse_job(text)
    }

    /// The stock metadata identity shared by all Lezer grammar configurations.
    pub fn data_facet(&self) -> &'static LrDataFacet {
        self.0.data_facet()
    }

    /// Install metadata for the stock Lezer facet at the extension's precedence.
    pub fn data(&self, data: LanguageData) -> Extension {
        self.data_facet().of(data)
    }

    /// Check presence at an original byte position in the active attachment
    /// or a direct Markdown mount. Presence does not grant edit access.
    pub fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        self.0.is_active_at(state, position, side)
    }

    /// Return active Lezer grammar ranges in source order, without including gaps.
    pub fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        self.0.find_regions(state)
    }

    pub(crate) fn parse_for_editor(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<SupportedDocument, ParseError> {
        self.0.parse_for_editor(text, settings)
    }

    /// Parse original bytes without projecting editor line breaks.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.0.parse(source)
    }
}
