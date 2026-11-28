use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, EditorState, Extension, Text,
    language::{
        LanguageData, StreamSettings, cpp, go, java, json,
        lezer::{LrParseJob, NodePropSource, NodeType, NodeTypes, ParseError, SyntaxTree},
        lezer_grammar,
        line_projection::splits_crlf,
        markdown::markdown_snapshot,
        rust, yaml,
    },
    language_data::{LrDataFacet, NativeSyntax},
};

use super::{
    Attachment, SupportedDocument, active_lr_viewport, active_support, attached_language_snapshot,
    install_attachment,
};
use crate::language_data::support::{LrGrammar, StrictParser};

impl LrGrammar {
    pub(in crate::language_data::support) fn parse_editor(
        self,
        text: Text,
        strict: bool,
        node_types: &NodeTypes,
    ) -> Result<NativeSyntax, ParseError> {
        use {
            cpp::CppDocument, go::GoDocument, java::JavaDocument, json::JsonDocument,
            lezer_grammar::LezerDocument, rust::RustDocument, yaml::YamlDocument,
        };
        Ok(match (self, strict) {
            (Self::Cpp, true) => NativeSyntax::Cpp(
                CppDocument::parse_editor_strict(text)?.with_node_types(node_types.clone()),
            ),
            (Self::Cpp, false) => {
                NativeSyntax::Cpp(CppDocument::parse(text).with_node_types(node_types.clone()))
            }
            (Self::Go, true) => NativeSyntax::Go(
                GoDocument::parse_editor_strict(text)?.with_node_types(node_types.clone()),
            ),
            (Self::Go, false) => {
                NativeSyntax::Go(GoDocument::parse(text).with_node_types(node_types.clone()))
            }
            (Self::Java(top), strict) => NativeSyntax::Java(JavaDocument::parse_editor(
                text,
                top,
                strict,
                node_types.clone(),
            )?),
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
            (Self::Rust, true) => NativeSyntax::Rust(
                RustDocument::parse_editor_strict(text)?.with_node_types(node_types.clone()),
            ),
            (Self::Rust, false) => {
                NativeSyntax::Rust(RustDocument::parse(text).with_node_types(node_types.clone()))
            }
            (Self::Yaml, true) => NativeSyntax::Yaml(YamlDocument::parse_editor_strict(text)?),
            (Self::Yaml, false) => NativeSyntax::Yaml(YamlDocument::parse(text)),
        })
    }
}

#[derive(Clone)]
pub(crate) struct LrConfig {
    grammar: LrGrammar,
    name: Arc<str>,
    strict: bool,
    node_types: NodeTypes,
}

impl LrConfig {
    pub(crate) fn language(&self) -> crate::language_data::NativeLanguage {
        self.grammar.language()
    }

    pub(crate) fn viewport_job(&self, text: Text) -> LrParseJob {
        let rows = self.node_types.clone();
        match self.grammar {
            LrGrammar::Json => {
                debug_assert!(!self.strict);
                json::JsonParseJob::with_node_types(text, rows).0
            }
            LrGrammar::Lezer => {
                lezer_grammar::LezerParseJob::with_config(text, rows, self.strict).0
            }
            LrGrammar::Cpp
            | LrGrammar::Go
            | LrGrammar::Java(_)
            | LrGrammar::Rust
            | LrGrammar::Yaml => {
                unreachable!("synchronous LR grammar")
            }
        }
    }

    fn root_data_facet(&self) -> Option<&LrDataFacet> {
        self.node_types
            .iter()
            .find(|row| row.is_top())?
            .data_facet()
    }

    pub(crate) fn lr_viewport_data(
        &self,
        state: &EditorState,
        at: usize,
    ) -> Vec<Arc<LanguageData>> {
        if !state.text().is_char_boundary(at) || splits_crlf(state.text().as_str(), at) {
            return Vec::new();
        }
        self.root_data_facet()
            .map_or_else(Vec::new, |facet| facet.read(state))
    }

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
        self.configure_grammar(self.grammar(), strict, name)
    }

    pub(super) fn grammar(&self) -> LrGrammar {
        self.config.grammar
    }

    pub(super) fn configure_grammar(
        &self,
        grammar: LrGrammar,
        strict: Option<bool>,
        name: Option<&str>,
    ) -> Self {
        debug_assert_eq!(grammar.language(), self.config.language());
        let mut config = (*self.config).clone();
        config.grammar = grammar;
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

    pub(super) fn json_parse_job(&self, text: Text) -> Option<json::JsonParseJob> {
        debug_assert!(self.config.grammar == LrGrammar::Json);
        (!self.config.strict).then(|| json::JsonParseJob(self.config.viewport_job(text)))
    }

    pub(super) fn lezer_parse_job(&self, text: Text) -> Option<lezer_grammar::LezerParseJob> {
        debug_assert!(self.config.grammar == LrGrammar::Lezer);
        Some(lezer_grammar::LezerParseJob(self.config.viewport_job(text)))
    }

    pub(super) fn viewport_extension(&self) -> Option<Extension> {
        match self.config.grammar {
            LrGrammar::Json if self.config.strict => None,
            LrGrammar::Cpp
            | LrGrammar::Go
            | LrGrammar::Java(_)
            | LrGrammar::Rust
            | LrGrammar::Yaml => None,
            LrGrammar::Json | LrGrammar::Lezer => Some(install_attachment(Attachment::LrViewport(
                self.config.clone(),
            ))),
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.config.name
    }

    pub(super) fn extension(&self) -> Extension {
        self.extension.clone()
    }

    pub(super) fn active_name(state: &EditorState, grammar: LrGrammar) -> Option<Arc<str>> {
        match active_support(&state.context()).expect("typed attached parser")? {
            Attachment::Lr(config) | Attachment::LrViewport(config)
                if config.language() == grammar.language() =>
            {
                Some(Arc::clone(&config.name))
            }
            _ => None,
        }
    }

    pub(super) fn data_facet(&self) -> &'static LrDataFacet {
        self.config.grammar.data_facet()
    }

    pub(super) fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        if let Some(config) = active_lr_viewport(&state.context()) {
            return config.grammar == self.config.grammar
                && state.text().is_char_boundary(position)
                && !splits_crlf(state.text().as_str(), position)
                && config.root_data_facet() == Some(self.data_facet());
        }
        language_document(state)
            .is_some_and(|doc| doc.root_facet_at(position, side) == Some(self.data_facet()))
    }

    pub(super) fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        if active_lr_viewport(&state.context())
            .is_some_and(|config| config.grammar == self.config.grammar)
        {
            return std::iter::once(0..state.text().len()).collect();
        }
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
            LrGrammar::Cpp => cpp::parse_raw(source, self.config.strict),
            LrGrammar::Go => go::parse_raw(source, self.config.strict),
            LrGrammar::Java(top) => java::parse_raw(top, source, self.config.strict),
            LrGrammar::Json => json::parse_raw(source, self.config.strict),
            LrGrammar::Lezer => lezer_grammar::parse_raw(source, self.config.strict),
            LrGrammar::Rust => rust::parse_raw(source, self.config.strict),
            LrGrammar::Yaml => yaml::parse_raw(source, self.config.strict),
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
