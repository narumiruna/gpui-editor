//! Headless completion, snippets, and bracket pairs from `@codemirror/autocomplete` 6.20.4.
//! See `docs/licenses/codemirror-autocomplete.txt` for the upstream MIT notice.
//!
//! All public positions and ranges use UTF-8 byte offsets. The caller runs
//! the sources and timers, dispatches commands, and draws the completion list.

mod bindings;
pub mod brackets;
mod matcher;
mod ranking;
pub mod session;
mod snippet;
mod snippet_state;

pub use bindings::{AutocompleteCommand, close_brackets_keymap, completion_keymap, snippet_keymap};
pub use matcher::{CompletionMatch, CompletionMatcher, MatchMode};
pub use ranking::{
    Completion, CompletionResult, CompletionSection, RankedCompletion, SectionRank,
    insert_completion_text, rank_completions, rank_completions_by,
};
pub use snippet::{Snippet, SnippetSession};
pub use snippet_state::{
    active_snippet, clear_snippet, has_next_snippet_field, has_previous_snippet_field,
    next_snippet_field, previous_snippet_field, snippet_range_in_active_field,
    snippet_state_extension, start_snippet,
};
