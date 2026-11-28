use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, ConfigurationError, EditorState, Extension, Text,
    language::{
        LanguageData, StreamSettings,
        lezer::{ParseError, SyntaxTree},
        markdown::markdown_snapshot,
        wast::{
            WastDataFacet, WastDocument, WastNodeType, WastNodeTypes, WastParseJob, WastPropSource,
            parse_raw_with_node_types,
        },
    },
};

use super::{
    Attachment, NativeLanguageSupport, SupportedDocument, active_support, active_wast_viewport,
    attached_language_snapshot, install_attachment,
};
use crate::language_data::{NativeLanguage, NativeSyntax, support::StrictParser};

/// One installable WAST language and its parser configuration. Configured
/// copies have separate parser identities and share WAST's original facet.
#[derive(Clone)]
pub struct WastLanguage {
    config: Arc<WastConfig>,
    extension: Extension,
}

impl PartialEq for WastLanguage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.config, &other.config)
    }
}

impl Eq for WastLanguage {}

pub(crate) struct WastConfig {
    name: Arc<str>,
    pub(super) strict: bool,
    node_types: WastNodeTypes,
}

impl WastConfig {
    pub(crate) fn root_node_type(&self) -> &WastNodeType {
        self.node_types.root()
    }

    pub(crate) fn viewport_job(&self, text: Text) -> WastParseJob {
        WastParseJob::with_node_types(text, self.node_types.clone())
    }

    pub(in crate::language_data::support) fn parse_document(
        &self,
        text: Text,
    ) -> Result<WastDocument, ParseError> {
        let node_types = self.node_types.clone();
        if self.strict {
            WastDocument::parse_strict(text, node_types)
        } else {
            Ok(WastDocument::parse_with_node_types(text, node_types))
        }
    }
}

/// The stock permissive language used by the WAST attachment and catalog.
pub fn wast_language() -> &'static WastLanguage {
    static LANGUAGE: LazyLock<WastLanguage> =
        LazyLock::new(|| WastLanguage::new("wast", false, WastNodeTypes::stock()));
    &LANGUAGE
}

impl WastLanguage {
    fn new(name: &str, strict: bool, node_types: WastNodeTypes) -> Self {
        let config = Arc::new(WastConfig {
            name: Arc::from(name),
            strict,
            node_types,
        });
        let extension = install_attachment(Attachment::Wast(Arc::clone(&config)));
        Self { config, extension }
    }

    /// Create a separate parser configuration. An empty name or `None`
    /// retains the current name. `Some(false)` restores recovery after a
    /// strict configuration. The node-type rows do not change.
    pub fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        Self::new(
            name.filter(|name| !name.is_empty()).unwrap_or(self.name()),
            strict.unwrap_or(self.config.strict),
            self.config.node_types.clone(),
        )
    }

    /// Create a separate parser configuration with one batch of property sources.
    /// Visit rows in term order and sources in slice order. Each source reads
    /// the original row. Each value updates the pending row before the next
    /// source runs. Rows without contributions retain their identity.
    ///
    /// These properties are available on direct, synchronous, and viewport
    /// trees. They do not change the grammar. Editor services read the
    /// property keys they support.
    pub fn with_node_props(&self, sources: &[WastPropSource]) -> Self {
        Self::new(
            self.name(),
            self.config.strict,
            self.config.node_types.extend(sources),
        )
    }

    /// The parser's node-type rows in term ID order, including anonymous rows.
    pub fn node_types(&self) -> impl ExactSizeIterator<Item = &WastNodeType> {
        self.config.node_types.iter()
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Install this language without adding key bindings or input handlers.
    pub fn extension(&self) -> Extension {
        self.extension.clone()
    }

    /// Install the opt-in viewport parser for a native `EditorView`. A strict
    /// configuration returns `None`. A bare state uses the configured root
    /// facet. When a view supplies a publication, state and view data queries
    /// both use its tree for positional rules.
    pub fn viewport_extension(&self) -> Option<Extension> {
        (!self.config.strict)
            .then(|| install_attachment(Attachment::WastViewport(Arc::clone(&self.config))))
    }

    /// Parse original bytes directly, without projecting editor line breaks.
    pub fn parse(&self, source: &str) -> Result<SyntaxTree, ParseError> {
        parse_raw_with_node_types(source, self.config.strict, self.config.node_types.clone())
    }

    pub(crate) fn parse_for_editor(
        &self,
        text: Text,
        settings: StreamSettings,
    ) -> Result<SupportedDocument, ParseError> {
        parse_configured(text, settings, &self.config)
    }

    /// The original stock facet, even if parser props change the root facet.
    pub fn data_facet(&self) -> &'static WastDataFacet {
        WastDataFacet::stock()
    }

    /// Add data to the original stock facet. A configured parser can
    /// select another facet on its root, including in a Markdown mount.
    pub fn data(&self, data: LanguageData) -> Extension {
        self.data_facet().of(data)
    }

    /// The name of the active whole-document WAST configuration, if any.
    pub fn active_name(state: &EditorState) -> Option<Arc<str>> {
        match active_support(&state.context()).expect("typed attached parser")? {
            Attachment::Wast(config) | Attachment::WastViewport(config) => {
                Some(Arc::clone(&config.name))
            }
            Attachment::Native(_) | Attachment::Lr(_) | Attachment::LrViewport(_) => None,
        }
    }

    /// Test whether the effective top node uses this language's original
    /// data facet at a UTF-8 byte boundary. Sublanguage rules do not change
    /// this identity. A viewport attachment checks the configured root without
    /// a syntax tree.
    pub fn is_active_at(&self, state: &EditorState, position: usize, side: Assoc) -> bool {
        if !state.text().is_char_boundary(position) {
            return false;
        }
        if let Some(config) = active_wast_viewport(&state.context()) {
            return config.root_node_type().data_facet() == Some(self.data_facet());
        }
        language_document(state)
            .is_some_and(|doc| doc.root_facet_at(position, side) == Some(self.data_facet()))
    }

    /// Return this language's regions in original UTF-8 bytes. An active
    /// whole-document WAST language returns the whole range even after a root
    /// facet change. A Markdown mount contributes only if its root facet matches.
    pub fn find_regions(&self, state: &EditorState) -> Vec<Range<usize>> {
        if active_wast_viewport(&state.context()).is_some() {
            return std::iter::once(0..state.text().len()).collect();
        }
        language_document(state).map_or_else(Vec::new, |doc| doc.wast_regions())
    }
}

fn language_document(state: &EditorState) -> Option<Arc<SupportedDocument>> {
    attached_language_snapshot(state).or_else(|| markdown_snapshot(state))
}

impl NativeLanguageSupport {
    /// Return the shared WAST root language from a loaded WAT or WAST support.
    pub fn wast_language(self) -> Option<&'static WastLanguage> {
        (self.language() == NativeLanguage::Wast).then(wast_language)
    }
}

pub(super) fn parse(
    text: Text,
    config: &Arc<WastConfig>,
) -> Result<SupportedDocument, ConfigurationError> {
    parse_configured(text, StreamSettings::default(), config)
        .map_err(ConfigurationError::StrictSyntax)
}

fn parse_configured(
    text: Text,
    settings: StreamSettings,
    config: &Arc<WastConfig>,
) -> Result<SupportedDocument, ParseError> {
    let syntax = config.parse_document(text)?;
    Ok(SupportedDocument {
        language: NativeLanguage::Wast,
        syntax: NativeSyntax::Wast(syntax),
        settings,
        markdown_parser: None,
        markdown_mounts: Vec::new(),
        strict_parser: config
            .strict
            .then(|| StrictParser::Wast(Arc::clone(config))),
    })
}
