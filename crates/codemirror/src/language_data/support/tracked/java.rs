use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension,
    language::{
        LanguageData,
        java::{JavaTop, NAME},
        lezer::{NodePropSource, NodeType, ParseError, SyntaxTree},
    },
    language_data::{LrDataFacet, NativeLanguage, NativeLanguageSupport, support::LrGrammar},
};

use super::lr::LrLanguage;

/// An installable Java language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct JavaLanguage(LrLanguage);

/// The stock permissive Java language with the `Program` top rule.
pub fn java_language() -> &'static JavaLanguage {
    static LANGUAGE: LazyLock<JavaLanguage> = LazyLock::new(|| {
        JavaLanguage(LrLanguage::new(
            LrGrammar::Java(JavaTop::Program),
            NAME,
            false,
        ))
    });
    &LANGUAGE
}

impl JavaLanguage {
    /// Create a distinct parser identity, even when all options are omitted.
    /// `None` keeps the current setting, and an empty name keeps the current
    /// name. Set strict to `Some(false)` to restore recovery.
    pub fn configure(
        &self,
        top: Option<JavaTop>,
        strict: Option<bool>,
        name: Option<&str>,
    ) -> Self {
        let top = top.unwrap_or_else(|| self.top());
        Self(self.0.configure_grammar(LrGrammar::Java(top), strict, name))
    }

    /// Create a distinct parser identity with updated node-type properties.
    /// Metadata and presence read the selected root's language data property.
    /// Native editor services keep their stock rules.
    pub fn with_node_props(&self, sources: &[NodePropSource]) -> Self {
        Self(self.0.with_node_props(sources))
    }

    /// Return the 240 grammar rows in term order, including anonymous repeats.
    pub fn node_types(&self) -> impl ExactSizeIterator<Item = &NodeType> {
        self.0.node_types()
    }

    /// The selected Java top rule.
    pub fn top(&self) -> JavaTop {
        let LrGrammar::Java(top) = self.0.grammar() else {
            unreachable!("a Java handle always selects a Java top rule");
        };
        top
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The name of the active whole-document Java attachment, if installed.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Java(JavaTop::Program))
    }

    /// Track synchronous syntax and metadata without input handlers or keys.
    /// Parse normalized line breaks and report original UTF-8 byte positions.
    /// Strict mode checks the final text without recovery.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// The stock data facet shared by both top rules and all configurations.
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

    /// Return active Java ranges in original bytes and source order.
    pub fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        self.0.find_regions(state)
    }

    /// Parse original bytes without normalizing editor line breaks. Permissive
    /// parsing uses bounded grammar recovery, not local editor fallback.
    /// A work limit returns [`ParseError::LimitExceeded`]. Neither setting
    /// checks Java compiler rules.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.0.parse(source)
    }
}

impl NativeLanguageSupport {
    /// Return the shared stock handle for Java support.
    pub fn java_language(self) -> Option<&'static JavaLanguage> {
        (self.language() == NativeLanguage::Java).then(java_language)
    }
}
