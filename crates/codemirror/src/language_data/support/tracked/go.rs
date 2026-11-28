use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension,
    language::{
        LanguageData,
        go::NAME,
        lezer::{NodePropSource, NodeType, ParseError, SyntaxTree},
    },
    language_data::{LrDataFacet, NativeLanguage, NativeLanguageSupport, support::LrGrammar},
};

use super::lr::LrLanguage;

/// An installable Go language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct GoLanguage(LrLanguage);

/// The stock permissive Go language.
pub fn go_language() -> &'static GoLanguage {
    static LANGUAGE: LazyLock<GoLanguage> =
        LazyLock::new(|| GoLanguage(LrLanguage::new(LrGrammar::Go, NAME, false)));
    &LANGUAGE
}

impl GoLanguage {
    /// Create a distinct parser identity. Omitted settings and an empty name
    /// keep the current values. Set strict to `Some(false)` to restore recovery.
    pub fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        Self(self.0.configure(strict, name))
    }

    /// Create a distinct parser identity with updated typed node rows.
    /// The root facet controls data lookup and presence. Native highlighting,
    /// folding, indentation, brackets, and isolation do not use these rows.
    pub fn with_node_props(&self, sources: &[NodePropSource]) -> Self {
        Self(self.0.with_node_props(sources))
    }

    /// Return immutable node-type rows in term order, including repeats.
    pub fn node_types(&self) -> impl ExactSizeIterator<Item = &NodeType> {
        self.0.node_types()
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// Return the active whole-document Go attachment's name, if installed.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Go)
    }

    /// Track Go syntax and metadata without completion sources or input
    /// handlers. The editor projects CRLF and lone CR to LF. Strict mode
    /// checks the final text without recovery or checkpoint reuse.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// Return the stock metadata facet shared by all Go configurations.
    pub fn data_facet(&self) -> &'static LrDataFacet {
        self.0.data_facet()
    }

    pub fn data(&self, data: LanguageData) -> Extension {
        self.data_facet().of(data)
    }

    /// Check presence by the effective root facet at an original byte offset.
    pub fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        self.0.is_active_at(state, position, side)
    }

    /// Return the whole document range in original bytes if Go is attached.
    pub fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        self.0.find_regions(state)
    }

    /// Parse original bytes, without editor line projection or fallback.
    /// Permissive mode can use bounded grammar recovery. Native limits return
    /// [`ParseError::LimitExceeded`]. Neither mode checks Go compiler rules.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.0.parse(source)
    }
}

impl NativeLanguageSupport {
    /// Return the shared stock handle for Go support.
    pub fn go_language(self) -> Option<&'static GoLanguage> {
        (self.language() == NativeLanguage::Go).then(go_language)
    }
}
