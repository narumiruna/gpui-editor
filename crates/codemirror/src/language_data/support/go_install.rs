use std::sync::{Arc, LazyLock};

use crate::{
    EditorState, Extension,
    autocomplete::session::SourceResult,
    language::{
        LanguageData,
        go::{GoDocument, go_local_completion, go_snippet_completion},
        language_data_provider,
    },
};

use super::{NativeLanguage, NativeSyntax, attached_language_snapshot};

/// A callable Go `autocomplete` value. Positions are original UTF-8 bytes.
pub type GoCompletionSource = fn(&EditorState, usize, bool) -> Option<SourceResult>;

/// Install the singleton Go parser and metadata without completion sources.
/// The active snapshot follows accepted edits. This does not install keys,
/// matching, automatic closing, or reindentation on input.
pub fn go_language_extension() -> Extension {
    static EXTENSION: LazyLock<Extension> =
        LazyLock::new(|| NativeLanguage::Go.support().tracked_syntax_extension());
    EXTENSION.clone()
}

/// Install the Go parser with its default snippet, keyword, and local-name
/// completion sources, in that order. It does not install a completion UI.
pub fn go_extension() -> Extension {
    Extension::group([
        go_language_extension(),
        language_data_provider(|state, at, _| {
            let Some(doc) = attached_language_snapshot(state) else {
                return Vec::new();
            };
            if matches!(doc.syntax(), NativeSyntax::Go(_)) && doc.text().is_char_boundary(at) {
                SOURCES.to_vec()
            } else {
                Vec::new()
            }
        }),
    ])
}

static SOURCES: LazyLock<[Arc<LanguageData>; 2]> = LazyLock::new(|| {
    let fixed: GoCompletionSource =
        |state, at, explicit| complete(state, at, explicit, go_snippet_completion);
    let local: GoCompletionSource =
        |state, at, explicit| complete(state, at, explicit, go_local_completion);
    [fixed, local].map(|source| Arc::new(LanguageData::default().with("autocomplete", source)))
});

fn complete(
    state: &EditorState,
    at: usize,
    explicit: bool,
    source: fn(&GoDocument, usize, bool) -> Option<SourceResult>,
) -> Option<SourceResult> {
    let doc = attached_language_snapshot(state)?;
    let NativeSyntax::Go(doc) = doc.syntax() else {
        return None;
    };
    source(doc, at, explicit)
}
