use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension, Text,
    language::{
        LanguageData, StreamSettings,
        lezer::{NodePropSource, NodeType, ParseError, SyntaxTree},
    },
    language_data::{
        LrDataFacet, NativeLanguage, NativeLanguageSupport, SupportedDocument, support::LrGrammar,
    },
};

use super::lr::LrLanguage;

/// An installable JSON language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct JsonLanguage(LrLanguage);

/// The stock permissive JSON language.
pub fn json_language() -> &'static JsonLanguage {
    static LANGUAGE: LazyLock<JsonLanguage> =
        LazyLock::new(|| JsonLanguage(LrLanguage::new(LrGrammar::Json, "json", false)));
    &LANGUAGE
}

impl JsonLanguage {
    /// Create a distinct parser identity with the same grammar and metadata.
    /// `None` or an empty name keeps the current name. Set strict to
    /// `Some(false)` to restore recovery.
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

    /// The name of the active whole-document JSON attachment, if any.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Json)
    }

    /// Track synchronous editor syntax and metadata. Strict parse failure
    /// rejects the state change. No lint source, optional input, or bindings
    /// are installed.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// The stock metadata identity shared by all JSON configurations.
    pub fn data_facet(&self) -> &'static LrDataFacet {
        self.0.data_facet()
    }

    /// Install metadata for the stock JSON facet at the extension's precedence.
    pub fn data(&self, data: LanguageData) -> Extension {
        self.data_facet().of(data)
    }

    /// Check presence at an original byte position in the active attachment
    /// or a direct Markdown mount. Presence does not grant edit access.
    pub fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        self.0.is_active_at(state, position, side)
    }

    /// Return active JSON ranges in source order, without including gaps.
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

impl NativeLanguageSupport {
    /// Return the stock JSON handle for a loaded JSON catalog entry.
    pub fn json_language(self) -> Option<&'static JsonLanguage> {
        (self.language() == NativeLanguage::Json).then(json_language)
    }
}
