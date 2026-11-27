use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, Extension, StateField, StateFieldOptions,
    language::{LanguageData, language_data_provider, yaml},
    language_data::NativeLanguage,
};

use super::{YamlFrontmatterDocument, YamlFrontmatterLanguage};

impl YamlFrontmatterLanguage {
    /// Track this wrapper in an editor state. Install the result with
    /// [`StateField::extension`]. Use [`Extension::default`] for no extra support.
    ///
    /// The supplied extensions are installed unchanged in the editor's
    /// configuration, not scoped to the body. Pass support extensions only,
    /// not a separate positional provider for the body parser.
    ///
    /// The field parses at installation and updates after accepted text edits.
    /// It retains the snapshot on other updates and reuses validated mount
    /// checkpoints when available. If parsing fails, the snapshot and its
    /// language data are absent until a later edit parses.
    pub fn state_field(self, support: Extension) -> StateField<Option<YamlFrontmatterDocument>> {
        StateField::define_shared(
            move |state| Ok(Arc::new(self.parse(state.text().clone()).ok())),
            move |document, update| {
                let changes = update.transaction().changes();
                if changes.is_empty() {
                    return Ok(Arc::clone(document));
                }
                let updated = document
                    .as_ref()
                    .as_ref()
                    .and_then(|old| old.reparse(changes).ok())
                    .or_else(|| self.parse(update.state().text().clone()).ok());
                Ok(Arc::new(updated))
            },
            StateFieldOptions::<Option<YamlFrontmatterDocument>>::default().provide(|field| {
                Extension::group([
                    language_data_provider(move |state, position, side| {
                        state
                            .context()
                            .field(&field)
                            .expect("typed front matter field")
                            .as_deref()
                            .and_then(Option::as_ref)
                            .map_or_else(Vec::new, |document| document.data_at(position, side))
                    }),
                    support,
                ])
            }),
        )
    }
}

impl YamlFrontmatterDocument {
    fn data_at(&self, position: usize, side: Assoc) -> Vec<Arc<LanguageData>> {
        if enters(self.body_range(), position, side) {
            return self.body.data_at(position - self.body_start, side);
        }
        if self
            .frontmatter()
            .is_some_and(|front| enters(front.yaml().root().range(), position, side))
        {
            static YAML: LazyLock<Arc<LanguageData>> = LazyLock::new(|| {
                Arc::new(
                    LanguageData::default()
                        .with("commentTokens", NativeLanguage::Yaml.comment_tokens())
                        .with("indentOnInput", yaml::indent_on_input as fn(&str) -> bool),
                )
            });
            return vec![Arc::clone(&YAML)];
        }
        Vec::new()
    }
}

fn enters(range: Range<usize>, position: usize, side: Assoc) -> bool {
    match side {
        Assoc::Before => range.start < position && position <= range.end,
        Assoc::After => range.start <= position && position < range.end,
    }
}
