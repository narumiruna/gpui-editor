use std::ops::Range;

use crate::{
    Assoc,
    language::{
        cpp, go, java, json,
        lezer::{NodeType, NodeTypes},
        lezer_grammar,
        line_projection::splits_crlf,
        rust, yaml,
    },
};

use super::{
    LrGrammar, NativeLanguage, NativeSyntax, SupportedDocument,
    route::{Grammar, Site},
};

pub use crate::language::lezer::LanguageFacet as LrDataFacet;

impl LrGrammar {
    pub(super) fn language(self) -> NativeLanguage {
        match self {
            Self::Cpp => NativeLanguage::Cpp,
            Self::Go => NativeLanguage::Go,
            Self::Java(_) => NativeLanguage::Java,
            Self::Json => NativeLanguage::Json,
            Self::Lezer => NativeLanguage::LezerGrammar,
            Self::Rust => NativeLanguage::Rust,
            Self::Yaml => NativeLanguage::Yaml,
        }
    }

    pub(super) fn data_facet(self) -> &'static LrDataFacet {
        match self {
            Self::Cpp => cpp::stock_data_facet(),
            Self::Go => go::stock_data_facet(),
            Self::Java(_) => java::stock_data_facet(),
            Self::Json => json::stock_data_facet(),
            Self::Lezer => lezer_grammar::stock_data_facet(),
            Self::Rust => rust::stock_data_facet(),
            Self::Yaml => yaml::stock_data_facet(),
        }
    }

    pub(super) fn node_types(self) -> &'static NodeTypes {
        match self {
            Self::Cpp => cpp::stock_node_types(),
            Self::Go => go::stock_node_types(),
            Self::Java(_) => java::stock_node_types(),
            Self::Json => json::stock_node_types(),
            Self::Lezer => lezer_grammar::stock_node_types(),
            Self::Rust => rust::stock_node_types(),
            Self::Yaml => yaml::stock_node_types(),
        }
    }
}

impl NativeSyntax {
    pub(super) fn lr_root_type(&self) -> Option<&NodeType> {
        match self {
            Self::Cpp(doc) => Some(doc.root().node_type()),
            Self::Java(doc) => doc.node_type(doc.root().term()),
            Self::Json(doc) => Some(doc.root().node_type()),
            Self::LezerGrammar(doc) => doc.root().node_type(),
            Self::Rust(doc) => doc.root().node_type(),
            Self::Wast(doc) => Some(doc.root().node_type()),
            Self::Yaml(_) => Some(yaml::stock_root_type()),
            _ => None,
        }
    }
}

impl<'a> Site<'a> {
    pub(super) fn data_root_type(&self) -> Option<&'a NodeType> {
        // Configured Go rows are used for data only, not for editor services.
        if let Grammar::Native(NativeSyntax::Go(doc)) = self.grammar {
            return doc.configured_node_types()?.get(doc.root().term());
        }
        self.lr_root_type()
    }

    pub(super) fn lr_root_type(&self) -> Option<&'a NodeType> {
        match self.grammar {
            Grammar::Native(syntax) => syntax.lr_root_type(),
            _ => None,
        }
    }
}

impl SupportedDocument {
    pub(crate) fn root_facet_at(&self, position: usize, side: Assoc) -> Option<&LrDataFacet> {
        let site = self.markdown_presence_site(position, side)?;
        if matches!(
            site.language,
            NativeLanguage::Cpp
                | NativeLanguage::Java
                | NativeLanguage::Rust
                | NativeLanguage::Yaml
        ) && splits_crlf(self.text().as_str(), position)
        {
            return None;
        }
        site.data_root_type()?.data_facet()
    }

    pub(super) fn regions_with_facet(
        &self,
        language: NativeLanguage,
        facet: &LrDataFacet,
    ) -> Vec<Range<usize>> {
        if self.language == language {
            return std::iter::once(0..self.text().len()).collect();
        }
        self.markdown_mounts
            .iter()
            .filter(|mount| {
                mount.syntax().lr_root_type().and_then(NodeType::data_facet) == Some(facet)
            })
            .flat_map(super::MarkdownMount::source_ranges)
            .collect()
    }
}
