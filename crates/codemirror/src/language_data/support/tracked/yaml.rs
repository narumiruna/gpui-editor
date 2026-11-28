use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension, Text,
    language::{
        LanguageData, StreamSettings,
        lezer::{ParseError, SyntaxTree},
        yaml::frontmatter_language::YamlFrontmatterDocument,
    },
    language_data::{
        LrDataFacet, NativeLanguage, NativeLanguageSupport, SupportedDocument, support::LrGrammar,
    },
};

use super::lr::LrLanguage;

/// An installable YAML language. Clones share one parser identity.
#[derive(Clone, PartialEq, Eq)]
pub struct YamlLanguage(LrLanguage);

/// The stock permissive YAML language.
pub fn yaml_language() -> &'static YamlLanguage {
    static LANGUAGE: LazyLock<YamlLanguage> =
        LazyLock::new(|| YamlLanguage(LrLanguage::new(LrGrammar::Yaml, "yaml", false)));
    &LANGUAGE
}

impl YamlLanguage {
    /// Create a distinct parser identity with the same grammar and metadata.
    /// `None` keeps the current setting. An empty name also keeps the current
    /// name. Set strict to `Some(false)` to restore recovery.
    pub fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        Self(self.0.configure(strict, name))
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The name of the active whole-document YAML attachment, if installed.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        LrLanguage::active_name(state, LrGrammar::Yaml)
    }

    /// Track synchronous syntax and metadata without input handlers or keys.
    /// In strict mode a parse failure rejects the state change. Strict edits
    /// parse the final text from the start.
    pub fn extension(&self) -> Extension {
        self.0.extension()
    }

    /// The stock metadata facet shared by all YAML configurations.
    pub fn data_facet(&self) -> &'static LrDataFacet {
        self.0.data_facet()
    }

    pub fn data(&self, data: LanguageData) -> Extension {
        self.data_facet().of(data)
    }

    /// Check presence at an original byte position, including YAML front matter
    /// and a mounted YAML body.
    pub fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        if let Some(snapshot) = YamlFrontmatterDocument::state_snapshot(state) {
            return snapshot
                .as_ref()
                .as_ref()
                .is_some_and(|doc| doc.yaml_at(position, side));
        }
        self.0.is_active_at(state, position, side)
    }

    /// Return active YAML ranges in original bytes and source order.
    pub fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        if let Some(snapshot) = YamlFrontmatterDocument::state_snapshot(state) {
            return snapshot
                .as_ref()
                .iter()
                .flat_map(YamlFrontmatterDocument::yaml_ranges)
                .collect();
        }
        self.0.find_regions(state)
    }

    pub(crate) fn parse_for_editor(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<SupportedDocument, ParseError> {
        self.0.parse_for_editor(text, settings)
    }

    /// Parse original bytes without editor line projection or local fallback.
    /// Strict mode skips recovery. Either mode can reach a native parse limit.
    /// This is not YAML validation.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        self.0.parse(source)
    }
}

impl NativeLanguageSupport {
    /// Return the stock YAML handle for YAML support.
    pub fn yaml_language(self) -> Option<&'static YamlLanguage> {
        (self.language() == NativeLanguage::Yaml).then(yaml_language)
    }
}
