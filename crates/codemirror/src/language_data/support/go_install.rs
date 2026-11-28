use crate::{
    EditorState, Extension,
    autocomplete::session::SourceResult,
    language::{
        LanguageData,
        go::{GoDocument, go_local_completion, go_snippet_completion},
    },
};

use super::{NativeSyntax, attached_language_snapshot, go_language};

/// A callable Go `autocomplete` value. Positions are original UTF-8 bytes.
pub type GoCompletionSource = fn(&EditorState, usize, bool) -> Option<SourceResult>;

/// Install the singleton Go parser and metadata without completion sources.
/// The active snapshot follows accepted edits. This does not install keys,
/// matching, automatic closing, or reindentation on input.
pub fn go_language_extension() -> Extension {
    go_language().extension()
}

/// Install the Go parser with its default snippet, keyword, and local-name
/// completion sources, in that order. It does not install a completion UI.
pub fn go_extension() -> Extension {
    let fixed: GoCompletionSource =
        |state, at, explicit| complete(state, at, explicit, go_snippet_completion);
    let local: GoCompletionSource =
        |state, at, explicit| complete(state, at, explicit, go_local_completion);
    let register =
        |source| go_language().data(LanguageData::default().with("autocomplete", source));
    Extension::group([go_language_extension(), register(fixed), register(local)])
}

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
