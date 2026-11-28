use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, EditorState, Extension, Transaction,
    autocomplete::session::SourceResult,
    language::{
        LanguageData,
        html::HtmlEmbeddedDocument,
        javascript::{self, JavascriptDocument, JavascriptOptions},
        python::{self, PythonDocument},
    },
    view::input::input_completion_handler,
};

use super::{
    LrDataFacet, NativeLanguage, NativeSyntax, SupportedDocument, attached_language_snapshot,
    metadata,
    route::{self, HtmlDataSite},
};

/// A callable Python or JavaScript-family `autocomplete` value.
/// Positions are original UTF-8 bytes in the supplied state.
pub type ScriptCompletionSource =
    Arc<dyn Fn(&EditorState, usize, bool) -> Option<SourceResult> + Send + Sync>;

static PYTHON_DATA: LazyLock<LrDataFacet> =
    LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Python))));
// Both dialects use the same language data, but install distinct fixed lists.
static JAVASCRIPT_DATA: LazyLock<LrDataFacet> = LazyLock::new(|| {
    LrDataFacet::new(Some(metadata(NativeLanguage::Javascript(
        JavascriptOptions::default(),
    ))))
});

pub(super) fn configured_data(
    state: &EditorState,
    document: &SupportedDocument,
    position: usize,
    side: Assoc,
) -> Vec<Arc<LanguageData>> {
    match document.language_at(position, side) {
        Some(NativeLanguage::Python) => PYTHON_DATA.read(state),
        Some(NativeLanguage::Javascript(_)) => javascript_data(state),
        _ => document.data_at(position, side),
    }
}

pub(super) fn javascript_data(state: &EditorState) -> Vec<Arc<LanguageData>> {
    JAVASCRIPT_DATA.read(state)
}

pub(super) fn python_extension() -> Extension {
    static BUNDLE: LazyLock<Extension> = LazyLock::new(|| {
        install(
            NativeLanguage::Python,
            &PYTHON_DATA,
            [
                Arc::new(|state, at, explicit| {
                    complete_python(state, at, explicit, python::local_completion_source)
                }),
                Arc::new(|state, at, explicit| {
                    complete_python(state, at, explicit, python::global_completion_source)
                }),
            ],
        )
    });
    BUNDLE.clone()
}

pub(super) fn javascript_extension(language: NativeLanguage) -> Extension {
    let NativeLanguage::Javascript(options) = language else {
        unreachable!("stock JavaScript-family catalog bundle");
    };
    static BUNDLES: LazyLock<[Extension; 4]> = LazyLock::new(|| {
        [(false, false), (true, false), (false, true), (true, true)].map(|(typescript, jsx)| {
            let language = NativeLanguage::Javascript(JavascriptOptions { typescript, jsx });
            let support = Extension::group([
                language.support().tracked_syntax_extension(),
                javascript_support(typescript),
            ]);
            if jsx {
                Extension::group([support, jsx_input_handler()])
            } else {
                support
            }
        })
    });
    BUNDLES[usize::from(options.typescript) + 2 * usize::from(options.jsx)].clone()
}

// Each factory owns a fixed list input and a local input with a shared callable.
pub(super) fn javascript_support(typescript: bool) -> Extension {
    let fixed: ScriptCompletionSource = Arc::new(move |state, at, explicit| {
        complete_javascript(state, at, |doc, at| {
            javascript::javascript_completion_source(doc, at, explicit, typescript)
        })
    });
    static LOCAL: LazyLock<ScriptCompletionSource> = LazyLock::new(|| {
        Arc::new(|state, at, explicit| {
            complete_javascript(state, at, |doc, at| {
                javascript::local_completion_source(doc, at, explicit)
            })
        })
    });
    Extension::group(source_inputs(&JAVASCRIPT_DATA, [fixed, Arc::clone(&LOCAL)]))
}

static EMPTY_JS: LazyLock<JavascriptDocument> =
    LazyLock::new(|| JavascriptDocument::parse(crate::Text::new("")));

fn jsx_input_handler() -> Extension {
    static CLOSE_TAGS: LazyLock<Extension> = LazyLock::new(|| {
        input_completion_handler(|input, default| {
            let start = input.state();
            let at = input.range();
            let typed = input.text();
            if start.read_only() || !at.is_empty() || !matches!(typed, ">" | "/") {
                return Ok(None);
            }
            if !matches!(
                attached_language_snapshot(start)
                    .and_then(|doc| doc.language_at(at.start, Assoc::Before)),
                Some(NativeLanguage::Javascript(_))
            ) {
                return Ok(None);
            }
            let state = &default()?.state;
            let Some(snapshot) = attached_language_snapshot(state) else {
                return Ok(None);
            };
            let NativeSyntax::Javascript(doc) = snapshot.syntax() else {
                return Ok(None);
            };
            Ok(javascript::close_tags_after(state, doc, 0, typed, true)?
                .map(Transaction::with_scroll_into_view))
        })
    });
    CLOSE_TAGS.clone()
}

fn install(
    language: NativeLanguage,
    data: &LrDataFacet,
    sources: [ScriptCompletionSource; 2],
) -> Extension {
    let [first, second] = source_inputs(data, sources);
    Extension::group([language.support().tracked_syntax_extension(), first, second])
}

fn source_inputs(data: &LrDataFacet, sources: [ScriptCompletionSource; 2]) -> [Extension; 2] {
    sources.map(|source| data.of(LanguageData::default().with("autocomplete", source)))
}

fn complete_python(
    state: &EditorState,
    at: usize,
    explicit: bool,
    source: fn(&PythonDocument, usize, bool) -> Option<SourceResult>,
) -> Option<SourceResult> {
    let doc = attached_language_snapshot(state)?;
    let NativeSyntax::Python(doc) = doc.syntax() else {
        return None;
    };
    source(doc, at, explicit)
}

fn complete_javascript(
    state: &EditorState,
    at: usize,
    source: impl FnOnce(&JavascriptDocument, usize) -> Option<SourceResult>,
) -> Option<SourceResult> {
    let active = attached_language_snapshot(state)?;
    let syntax = active.syntax();
    if let NativeSyntax::Javascript(doc) = syntax {
        return source(doc, at);
    }
    let script = |side| match route::html_data_site(&active, at, side)? {
        HtmlDataSite::Script(doc, query) => Some((doc, query)),
        HtmlDataSite::Mount(mount, query) => match mount.document() {
            HtmlEmbeddedDocument::Javascript(doc) => Some((doc, query)),
            _ => None,
        },
        _ => None,
    };
    let (doc, query) = script(Assoc::Before).or_else(|| {
        // The template data query can enter an unbracketed child from the
        // right. Its callable still uses a left-biased completion context.
        if !matches!(syntax, NativeSyntax::Vue(_) | NativeSyntax::Angular(_)) {
            return None;
        }
        script(Assoc::After).filter(|(_, query)| query.at == 0)
    })?;
    let doc = if query.at == 0 { &EMPTY_JS } else { doc };
    let result = source(doc, query.at)?;
    active.project_completion(&query, result)
}
