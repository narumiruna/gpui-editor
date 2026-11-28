use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension, Text,
    language::{
        LanguageData, StreamSettings,
        lezer::{NodePropSource, NodeType, ParseError, SyntaxTree},
        rust::NAME,
    },
    language_data::{
        LrDataFacet, NativeLanguage, NativeLanguageSupport, SupportedDocument, support::LrGrammar,
    },
};

use super::lr::LrLanguage;

/// An installable Rust language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct RustLanguage(LrLanguage);

/// The stock permissive Rust language.
pub fn rust_language() -> &'static RustLanguage {
    static LANGUAGE: LazyLock<RustLanguage> =
        LazyLock::new(|| RustLanguage(LrLanguage::new(LrGrammar::Rust, NAME, false)));
    &LANGUAGE
}

impl RustLanguage {
    /// Create a distinct parser identity with the same grammar and metadata.
    /// `None` keeps the current setting. An empty name also keeps the current
    /// name. Set strict to `Some(false)` to restore recovery.
    pub fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        Self(self.0.configure(strict, name))
    }

    /// Create a distinct parser identity with updated typed node rows.
    /// The root's language data property controls metadata lookup and presence.
    /// Attached fold queries, configured bracket queries, and isolation
    /// marks read these rows. Whole-document indentation reads these rows
    /// for the active attachment.
    /// State-free folding, indentation, legacy bracket matching, and
    /// highlighting keep their stock rules.
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

    /// The name of the active whole-document Rust attachment, if installed.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Rust)
    }

    /// Track synchronous syntax and metadata without input handlers or keys.
    /// Strict mode checks the final text without recovery or checkpoint reuse.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// The stock metadata facet shared by all Rust configurations.
    pub fn data_facet(&self) -> &'static LrDataFacet {
        self.0.data_facet()
    }

    pub fn data(&self, data: LanguageData) -> Extension {
        self.data_facet().of(data)
    }

    /// Check presence by the effective root facet at an original byte position.
    pub fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        self.0.is_active_at(state, position, side)
    }

    /// Return active Rust ranges in original bytes and source order.
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

    /// Parse original bytes without projecting editor line breaks. Strict
    /// mode skips recovery. This does not check Rust compiler rules.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.0.parse(source)
    }
}

impl NativeLanguageSupport {
    /// Return the stock Rust handle for Rust support.
    pub fn rust_language(self) -> Option<&'static RustLanguage> {
        (self.language() == NativeLanguage::Rust).then(rust_language)
    }
}
