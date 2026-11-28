use std::sync::{Arc, LazyLock, OnceLock};

use crate::{
    Assoc, EditorState, Extension,
    autocomplete::session::SourceResult,
    language::{
        LanguageData,
        css::{CssPropertySnapshot, css_completion_source_with_properties},
        html::HtmlEmbeddedDocument,
        sass::SassDialect,
    },
};

use super::{
    LrDataFacet, NativeLanguage, attached_language_snapshot, metadata,
    route::{self, HtmlDataSite},
};

/// A callable stylesheet `autocomplete` value. Positions are original UTF-8 bytes.
pub type StylesheetCompletionSource =
    Arc<dyn Fn(&EditorState, usize, bool) -> Option<SourceResult> + Send + Sync>;

impl NativeLanguage {
    /// The stock stylesheet language data facet, or `None` for other languages.
    /// Indented Sass and SCSS use the same facet but different parsers.
    pub fn stylesheet_data_facet(self) -> Option<&'static LrDataFacet> {
        static CSS: LazyLock<LrDataFacet> =
            LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Css))));
        static LESS: LazyLock<LrDataFacet> =
            LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Less))));
        static SASS: LazyLock<LrDataFacet> = LazyLock::new(|| {
            LrDataFacet::new(Some(metadata(NativeLanguage::Sass(SassDialect::Scss))))
        });
        Some(match self {
            Self::Css => &*CSS,
            Self::Less => &*LESS,
            Self::Sass(_) => &*SASS,
            _ => return None,
        })
    }
}

pub(super) fn extension(language: NativeLanguage) -> Extension {
    static CSS: OnceLock<Extension> = OnceLock::new();
    static LESS: OnceLock<Extension> = OnceLock::new();
    static SASS: OnceLock<Extension> = OnceLock::new();
    static SCSS: OnceLock<Extension> = OnceLock::new();
    let bundle = match language {
        NativeLanguage::Css => &CSS,
        NativeLanguage::Less => &LESS,
        NativeLanguage::Sass(SassDialect::Indented) => &SASS,
        NativeLanguage::Sass(SassDialect::Scss) => &SCSS,
        _ => unreachable!("stock stylesheet catalog bundle"),
    };
    bundle
        .get_or_init(|| {
            let support = match language {
                NativeLanguage::Css => css_support(),
                _ => {
                    let facet = language.stylesheet_data_facet().expect("stock stylesheet");
                    let source = if matches!(language, NativeLanguage::Sass(_)) {
                        // Each Sass bundle has a distinct input for a shared source.
                        static SOURCE: OnceLock<StylesheetCompletionSource> = OnceLock::new();
                        SOURCE.get_or_init(|| completion_source(facet)).clone()
                    } else {
                        completion_source(facet)
                    };
                    facet.of(LanguageData::default().with("autocomplete", source))
                }
            };
            Extension::group([language.support().tracked_syntax_extension(), support])
        })
        .clone()
}

pub(super) fn css_support() -> Extension {
    let facet = NativeLanguage::Css
        .stylesheet_data_facet()
        .expect("stock CSS");
    static SOURCE: LazyLock<StylesheetCompletionSource> = LazyLock::new(|| {
        completion_source(
            NativeLanguage::Css
                .stylesheet_data_facet()
                .expect("stock CSS"),
        )
    });
    facet.of(LanguageData::default().with("autocomplete", Arc::clone(&SOURCE)))
}

fn completion_source(facet: &'static LrDataFacet) -> StylesheetCompletionSource {
    Arc::new(move |state, at, explicit| {
        let active = attached_language_snapshot(state)?;
        let properties = CssPropertySnapshot::source_for(state);
        if let Some(site) = route::html_data_site(&active, at, Assoc::Before) {
            if NativeLanguage::Css.stylesheet_data_facet()? != facet {
                return None;
            }
            let HtmlDataSite::Mount(mount, query) = site else {
                return None;
            };
            let HtmlEmbeddedDocument::Css(doc) = mount.document() else {
                return None;
            };
            // The callable is present at the left bracket, but its
            // left-biased syntax query does not see the CSS tree there.
            if query.at == 0 {
                return None;
            }
            let properties = match properties.as_deref() {
                Some(properties) => properties,
                None => CssPropertySnapshot::stock(),
            };
            let result =
                css_completion_source_with_properties(doc, query.at, explicit, properties)?;
            return active.project_completion(&query, result);
        }
        let language = active.language_at(at, Assoc::Before)?;
        if language.stylesheet_data_facet()? != facet {
            return None;
        }
        active
            .indexed_completions_with_css_properties(at, explicit, properties.as_deref())
            .next()
            .map(|(_, result)| result)
    })
}
