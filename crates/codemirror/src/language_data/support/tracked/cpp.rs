use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension,
    language::{
        LanguageData,
        cpp::NAME,
        lezer::{NodePropSource, NodeType, ParseError, SyntaxTree},
    },
    language_data::{LrDataFacet, NativeLanguage, NativeLanguageSupport, support::LrGrammar},
};

use super::lr::LrLanguage;

/// An installable C and C++ language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct CppLanguage(LrLanguage);

/// The stock permissive C and C++ language.
pub fn cpp_language() -> &'static CppLanguage {
    static LANGUAGE: LazyLock<CppLanguage> =
        LazyLock::new(|| CppLanguage(LrLanguage::new(LrGrammar::Cpp, NAME, false)));
    &LANGUAGE
}

impl CppLanguage {
    /// Create a distinct parser identity with the same grammar and metadata.
    /// `None` keeps the current setting. An empty name also keeps the current
    /// name. Set strict to `Some(false)` to restore recovery.
    pub fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        Self(self.0.configure(strict, name))
    }

    /// Create a distinct parser identity with updated typed node rows.
    /// The root's language data property controls metadata lookup and presence.
    /// Whole-document fold queries, bracket matching, and isolation marks
    /// read these rows. State-free folding, indentation, and highlighting
    /// keep their stock rules.
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

    /// The name of the active whole-document C or C++ attachment, if installed.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Cpp)
    }

    /// Track synchronous syntax and metadata without input handlers or keys.
    /// Parse with the default editor line projection. Strict mode checks the
    /// final text without recovery or checkpoint reuse.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// The stock metadata facet shared by all C and C++ configurations.
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

    /// Return active C and C++ ranges in original bytes and source order.
    pub fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        self.0.find_regions(state)
    }

    /// Parse original bytes without projecting editor line breaks. Strict mode
    /// skips recovery. Permissive mode can use bounded grammar recovery, not
    /// local editor fallback. A native limit returns [`ParseError::LimitExceeded`].
    /// Neither mode checks compiler rules.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.0.parse(source)
    }
}

impl NativeLanguageSupport {
    /// Return the shared stock handle for C and C++ support.
    pub fn cpp_language(self) -> Option<&'static CppLanguage> {
        (self.language() == NativeLanguage::Cpp).then(cpp_language)
    }
}
