use std::sync::{Arc, LazyLock};

use crate::{EditorState, Extension, Facet, extension::FacetId, language::LanguageData};

use super::{NodeProp, NodeType};

type Entries = Facet<Arc<LanguageData>, Vec<Arc<LanguageData>>>;

/// Typed metadata for a language root. Clones refer to the same installed
/// entries. Two definitions with equal defaults still have distinct identities.
#[derive(Clone)]
pub struct LanguageFacet {
    facet: Arc<Entries>,
    base: Option<Arc<LanguageData>>,
}

impl LanguageFacet {
    /// Define a facet with optional defaults after its installed entries.
    pub fn new(base: Option<LanguageData>) -> Self {
        let base = base.map(Arc::new);
        let defaults = base.clone();
        let facet = Arc::new(Facet::new(move |entries: &[&Arc<LanguageData>]| {
            entries
                .iter()
                .copied()
                .chain(defaults.iter())
                .cloned()
                .collect()
        }));
        Self { facet, base }
    }

    /// Install a data entry at the extension's precedence.
    pub fn of(&self, data: LanguageData) -> Extension {
        self.facet.of(Arc::new(data))
    }

    pub(crate) fn base(&self) -> Option<&Arc<LanguageData>> {
        self.base.as_ref()
    }

    pub(crate) fn id(&self) -> FacetId {
        self.facet.id()
    }

    pub(crate) fn read(&self, state: &EditorState) -> Vec<Arc<LanguageData>> {
        state
            .context()
            .facet(&self.facet)
            .expect("typed LR language data")
            .as_ref()
            .clone()
    }
}

impl PartialEq for LanguageFacet {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.facet, &other.facet)
    }
}

impl Eq for LanguageFacet {}

/// Select the language data facet on a root node type. `None` disables the
/// root's data. Changing this property does not change the language handle's
/// own facet identity.
pub static LANGUAGE_DATA: LazyLock<NodeProp<Option<LanguageFacet>>> = LazyLock::new(NodeProp::new);

impl NodeType {
    pub(crate) fn data_facet(&self) -> Option<&LanguageFacet> {
        self.prop(&LANGUAGE_DATA).and_then(Option::as_ref)
    }
}
