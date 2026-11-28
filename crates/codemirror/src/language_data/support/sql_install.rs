use std::sync::{Arc, OnceLock};

use crate::{
    Assoc, EditorState, Extension,
    autocomplete::session::SourceResult,
    language::{LanguageData, sql::SqlDialect},
};

use super::{
    LrDataFacet, NativeLanguage, NativeSyntax, attached_language_snapshot,
    completion::sql_keywords, metadata,
};

/// A callable SQL `autocomplete` value. Positions are original UTF-8 bytes.
pub type SqlCompletionSource =
    Arc<dyn Fn(&EditorState, usize, bool) -> Option<SourceResult> + Send + Sync>;

impl SqlDialect {
    /// The language data facet of this stock dialect's synchronous attachment.
    pub fn data_facet(self) -> &'static LrDataFacet {
        static FACETS: [OnceLock<LrDataFacet>; 8] = [const { OnceLock::new() }; 8];
        FACETS[self as usize]
            .get_or_init(|| LrDataFacet::new(Some(metadata(NativeLanguage::Sql(self)))))
    }
}

pub(super) fn sql_extension(dialect: SqlDialect) -> Extension {
    static BUNDLES: [OnceLock<Extension>; 8] = [const { OnceLock::new() }; 8];
    BUNDLES[dialect as usize]
        .get_or_init(|| {
            let language = NativeLanguage::Sql(dialect);
            let source: SqlCompletionSource = Arc::new(move |state, at, explicit| {
                let active = attached_language_snapshot(state)?;
                if active.language_at(at, Assoc::Before)? != language {
                    return None;
                }
                let NativeSyntax::Sql(doc) = active.syntax() else {
                    return None;
                };
                sql_keywords(dialect).complete(doc, at, explicit)
            });
            Extension::group([
                language.support().tracked_syntax_extension(),
                dialect
                    .data_facet()
                    .of(LanguageData::default().with("autocomplete", source)),
            ])
        })
        .clone()
}
