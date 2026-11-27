use crate::{EditorState, Transaction, view::keymap::KeyBinding};

use super::{
    SearchPanelInput, close_search_panel, find_next, find_previous, open_search_panel,
    query_state::stored_query, replace_all, replace_next, select_matches, select_next_occurrence,
    select_selection_matches,
};

/// Search commands that a host can bind to keys or other input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchAction {
    OpenSearchPanel,
    CloseSearchPanel,
    FindNext,
    FindPrevious,
    SelectMatches,
    SelectSelectionMatches,
    SelectNextOccurrence,
    ReplaceNext,
    ReplaceAll,
    GotoLine,
}

#[derive(Debug)]
pub enum SearchActionResult {
    Unhandled,
    Update(Transaction),
    /// Open or focus the panel. Apply the optional state update first.
    OpenSearchPanel(Option<Transaction>),
    /// Open or focus the Go to line dialog. The host supplies the input and
    /// retains the opening state to resolve relative positions.
    OpenGotoLine,
}

impl SearchAction {
    /// Run the state portion of an action. Unlike the standalone match
    /// commands, actions request the search panel if the stored query is
    /// missing or invalid. The host owns focus, scrolling, and dialogs.
    pub fn run(self, state: &EditorState, input: SearchPanelInput) -> SearchActionResult {
        let command = match self {
            Self::OpenSearchPanel => {
                return SearchActionResult::OpenSearchPanel(open_search_panel(state, input));
            }
            Self::CloseSearchPanel => close_search_panel,
            Self::SelectSelectionMatches => select_selection_matches,
            Self::SelectNextOccurrence => select_next_occurrence,
            Self::GotoLine => return SearchActionResult::OpenGotoLine,
            action => {
                if stored_query(state).is_none_or(|query| !query.is_valid()) {
                    return SearchActionResult::OpenSearchPanel(open_search_panel(state, input));
                }
                match action {
                    Self::FindNext => find_next,
                    Self::FindPrevious => find_previous,
                    Self::SelectMatches => select_matches,
                    Self::ReplaceNext => replace_next,
                    Self::ReplaceAll => replace_all,
                    _ => unreachable!("only stored-query commands remain"),
                }
            }
        };
        command(state).map_or(SearchActionResult::Unhandled, SearchActionResult::Update)
    }
}

/// Default bindings from `@codemirror/search` 6.7.3. The host selects the
/// `editor` or `search-panel` scope and supplies the action handler.
pub fn search_keymap() -> [KeyBinding<SearchAction>; 7] {
    use SearchAction::*;

    let panel_binding = |key, action| KeyBinding {
        scope: "editor search-panel".into(),
        ..KeyBinding::new(key, action)
    };
    let find = |key| KeyBinding {
        shift: Some(FindPrevious),
        prevent_default: true,
        ..panel_binding(key, FindNext)
    };
    [
        panel_binding("Mod-f", OpenSearchPanel),
        find("F3"),
        find("Mod-g"),
        panel_binding("Escape", CloseSearchPanel),
        KeyBinding::new("Mod-Shift-l", SelectSelectionMatches),
        KeyBinding::new("Mod-Alt-g", GotoLine),
        KeyBinding {
            prevent_default: true,
            ..KeyBinding::new("Mod-d", SelectNextOccurrence)
        },
    ]
}
