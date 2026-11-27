//! Positional language data from `@codemirror/state` 6.7.6.

use std::{
    any::Any,
    sync::{Arc, LazyLock},
};

use crate::{Assoc, EditorState, Extension, Facet};

use super::LanguageData;

type Provider = dyn Fn(&EditorState, usize, Assoc) -> Vec<Arc<LanguageData>> + Send + Sync;

static PROVIDERS: LazyLock<Facet<Arc<Provider>, Vec<Arc<Provider>>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Arc<Provider>]| {
        inputs.iter().map(|provider| Arc::clone(provider)).collect()
    })
});

/// Add a language data provider. Providers run in extension precedence order.
/// Each provider can return data for more than one language, in lookup order.
/// At a shared boundary, `Assoc::Before` prefers the language before the
/// position and `Assoc::After` prefers the language after it. The provider
/// must enforce its own region and boundary rules.
pub fn language_data_provider(
    provider: impl Fn(&EditorState, usize, Assoc) -> Vec<Arc<LanguageData>> + Send + Sync + 'static,
) -> Extension {
    PROVIDERS.of(Arc::new(provider))
}

impl EditorState {
    /// Return every matching value, in provider order. Skip absent values or
    /// values of a different type. An invalid UTF-8 offset calls no providers.
    pub fn language_data_at<T: Any + Clone>(
        &self,
        name: &str,
        position: usize,
        side: Assoc,
    ) -> Vec<T> {
        self.language_data_objects(position, side)
            .into_iter()
            .filter_map(|data| data.get::<T>(name).cloned())
            .collect()
    }

    pub(crate) fn language_data_objects(
        &self,
        position: usize,
        side: Assoc,
    ) -> Vec<Arc<LanguageData>> {
        if !self.text().is_char_boundary(position) {
            return Vec::new();
        }
        self.context()
            .facet(&PROVIDERS)
            .expect("typed language data providers")
            .iter()
            .flat_map(|provider| provider(self, position, side))
            .collect()
    }
}
