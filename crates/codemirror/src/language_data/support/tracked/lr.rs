use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, EditorState, Extension, Text,
    language::{
        StreamSettings, json,
        lezer::{NodePropSource, NodeType, NodeTypes, ParseError, SyntaxTree},
        lezer_grammar,
        markdown::markdown_snapshot,
    },
    language_data::{LrDataFacet, NativeSyntax},
};

use super::{
    Attachment, SupportedDocument, active_support, attached_language_snapshot, install_attachment,
};
use crate::language_data::support::{LrGrammar, StrictParser};

impl LrGrammar {
    pub(in crate::language_data::support) fn parse_editor(
        self,
        text: Text,
        strict: bool,
        node_types: &NodeTypes,
    ) -> Result<NativeSyntax, ParseError> {
        use {json::JsonDocument, lezer_grammar::LezerDocument};
        Ok(match (self, strict) {
            (Self::Json, true) => NativeSyntax::Json(
                JsonDocument::parse_editor_strict(text)?.with_node_types(node_types.clone()),
            ),
            (Self::Json, false) => NativeSyntax::Json(
                JsonDocument::parse_editor(text).with_node_types(node_types.clone()),
            ),
            (Self::Lezer, true) => NativeSyntax::LezerGrammar(
                LezerDocument::parse_strict(text)?.with_node_types(node_types.clone()),
            ),
            (Self::Lezer, false) => NativeSyntax::LezerGrammar(
                LezerDocument::parse(text).with_node_types(node_types.clone()),
            ),
        })
    }
}

#[derive(Clone)]
pub(super) struct LrConfig {
    grammar: LrGrammar,
    name: Arc<str>,
    strict: bool,
    node_types: NodeTypes,
}

impl LrConfig {
    pub(super) fn parse(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<SupportedDocument, ParseError> {
        let syntax = self
            .grammar
            .parse_editor(text, self.strict, &self.node_types)?;
        Ok(SupportedDocument {
            language: self.grammar.language(),
            syntax,
            settings,
            markdown_parser: None,
            markdown_mounts: Vec::new(),
            strict_parser: self.strict.then_some(StrictParser::Lr(self.grammar)),
        })
    }
}

#[derive(Clone)]
pub(super) struct LrLanguage {
    config: Arc<LrConfig>,
    extension: Extension,
}

impl PartialEq for LrLanguage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.config, &other.config)
    }
}

impl Eq for LrLanguage {}

impl LrLanguage {
    pub(super) fn new(grammar: LrGrammar, name: &str, strict: bool) -> Self {
        Self::from_config(LrConfig {
            grammar,
            name: Arc::from(name),
            strict,
            node_types: grammar.node_types().clone(),
        })
    }

    fn from_config(config: LrConfig) -> Self {
        let config = Arc::new(config);
        let extension = install_attachment(Attachment::Lr(Arc::clone(&config)));
        Self { config, extension }
    }

    pub(super) fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        let mut config = (*self.config).clone();
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            config.name = Arc::from(name);
        }
        config.strict = strict.unwrap_or(config.strict);
        Self::from_config(config)
    }

    pub(super) fn with_node_props(&self, sources: &[NodePropSource]) -> Self {
        Self::from_config(LrConfig {
            node_types: self.config.node_types.extend(sources),
            ..(*self.config).clone()
        })
    }

    pub(super) fn node_types(&self) -> impl ExactSizeIterator<Item = &NodeType> {
        self.config.node_types.iter()
    }

    pub(super) fn name(&self) -> &str {
        &self.config.name
    }

    pub(super) fn extension(&self) -> Extension {
        self.extension.clone()
    }

    pub(super) fn active_name(state: &EditorState, grammar: LrGrammar) -> Option<Arc<str>> {
        match active_support(&state.context()).expect("typed attached parser")? {
            Attachment::Lr(config) if config.grammar == grammar => Some(Arc::clone(&config.name)),
            _ => None,
        }
    }

    pub(super) fn data_facet(&self) -> &'static LrDataFacet {
        self.config.grammar.data_facet()
    }

    pub(super) fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        language_document(state)
            .is_some_and(|doc| doc.root_facet_at(position, side) == Some(self.data_facet()))
    }

    pub(super) fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        language_document(state).map_or_else(Vec::new, |doc| {
            doc.regions_with_facet(self.config.grammar.language(), self.data_facet())
        })
    }

    pub(super) fn parse_for_editor(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<SupportedDocument, ParseError> {
        self.config.parse(text, settings)
    }

    pub(super) fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        match self.config.grammar {
            LrGrammar::Json => json::parse_raw(source, self.config.strict),
            LrGrammar::Lezer => lezer_grammar::parse_raw(source, self.config.strict),
        }
        .map(|tree| {
            tree.with_node_types(
                self.config.grammar.language(),
                self.config.node_types.clone(),
            )
        })
    }
}

fn language_document(state: &EditorState) -> Option<Arc<SupportedDocument>> {
    if active_support(&state.context())
        .expect("typed attached parser")
        .is_some()
    {
        attached_language_snapshot(state)
    } else {
        markdown_snapshot(state)
    }
}
