use std::sync::{Arc, LazyLock};

use super::{SearchOptions, SearchQuery, panel};
use crate::{
    EditorState, Extension, Facet, StateContext, StateEffect, StateEffectType, StateField,
    StateFieldOptions, Transaction,
    view::panels::{Panel, PanelId, PanelPlacement, SHOW_PANEL},
};

static SEARCH_OPTIONS: LazyLock<Facet<SearchOptions, SearchOptions>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&SearchOptions]| {
        inputs.first().map_or_else(SearchOptions::default, |v| **v)
    })
});
static PANEL_ID: LazyLock<PanelId> = LazyLock::new(PanelId::new);
static PLACEMENT: LazyLock<Facet<PanelPlacement, PanelPlacement>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&PanelPlacement]| {
        inputs
            .first()
            .map_or_else(PanelPlacement::default, |first| **first)
    })
});

/// Set the placement of the registered search panel. The default is bottom.
/// Multiple inputs use extension precedence. An open native panel keeps its
/// identity when its declared placement changes.
pub fn search_panel_placement(placement: PanelPlacement) -> Extension {
    PLACEMENT.of(placement)
}

/// Identify the native search panel in the shared panel registry.
/// The host owns its controls, event routing, and focus.
pub fn search_panel_id() -> PanelId {
    *PANEL_ID
}

#[derive(Clone)]
struct SearchState {
    query: Arc<SearchQuery>,
    panel_open: bool,
}

static SEARCH_STATE: LazyLock<StateField<SearchState>> = LazyLock::new(|| {
    StateField::define(
        |state| {
            Ok(SearchState {
                query: Arc::new(default_query(state, *state.facet(&SEARCH_OPTIONS)?, None)),
                panel_open: false,
            })
        },
        |value, update| {
            let mut next = value.clone();
            for effect in update.transaction().effects() {
                if let Some(query) = effect.get(&SET_QUERY) {
                    next.query = Arc::new(query.clone());
                } else if let Some(open) = effect.get(&TOGGLE_PANEL) {
                    next.panel_open = *open;
                }
            }
            Ok(next)
        },
        StateFieldOptions::<SearchState>::default()
            .compare(|a, b| a.panel_open == b.panel_open && Arc::ptr_eq(&a.query, &b.query))
            .provide(|field| {
                Extension::group([
                    SHOW_PANEL.compute_with_dependencies(
                        |deps| {
                            deps.field(&field);
                            deps.facet(&PLACEMENT);
                        },
                        move |state| {
                            if !state.field(&field)?.is_some_and(|search| search.panel_open) {
                                return Ok(None);
                            }
                            let placement = *state.facet(&PLACEMENT)?;
                            Ok(Some(Panel::Custom {
                                id: search_panel_id(),
                                placement,
                            }))
                        },
                    ),
                    panel::lifecycle(),
                ])
            }),
    )
});
static SET_QUERY: LazyLock<StateEffectType<SearchQuery>> = LazyLock::new(StateEffectType::new);
static TOGGLE_PANEL: LazyLock<StateEffectType<bool>> = LazyLock::new(StateEffectType::new);

/// Enable persistent search query state. The first options input wins.
/// The initial query uses the main selection at installation. If appending the
/// extension in a transaction, the initial query uses the start state.
pub fn extension(options: SearchOptions) -> Extension {
    Extension::group([SEARCH_OPTIONS.of(options), SEARCH_STATE.extension()])
}

/// Get the stored query. Without the extension, derive it from the main
/// selection with default options. Empty selections and selections longer
/// than 100 UTF-16 units yield an empty query.
pub fn get_search_query(state: &EditorState) -> SearchQuery {
    stored_query(state).map_or_else(
        || default_query(&state.context(), SearchOptions::default(), None),
        |query| (*query).clone(),
    )
}

/// Change the query in an editor with the search extension. The last query
/// effect in a transaction wins. Without the extension, the effect is ignored.
pub fn set_search_query(query: SearchQuery) -> StateEffect {
    SET_QUERY.of(query)
}

/// Status of the main input in an open search panel. The host supplies this
/// status. It has no effect when the panel is closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchPanelInput {
    Missing,
    Focused,
    Unfocused,
}

/// Whether the editor's search panel is open. This does not track focus.
pub fn search_panel_open(state: &EditorState) -> bool {
    search_state(state).is_some_and(|search| search.panel_open)
}

/// Prepare the state update to open or refocus the search panel. The host
/// owns the panel and moves focus. An already open panel may need no update.
pub fn open_search_panel(state: &EditorState, input: SearchPanelInput) -> Option<Transaction> {
    let mut effects = Vec::new();
    if let Some(search) = search_state(state) {
        if search.panel_open && input != SearchPanelInput::Unfocused {
            return None;
        }
        let query = default_query(
            &state.context(),
            search.query.options(),
            Some(&search.query),
        );
        if !search.panel_open {
            effects.push(TOGGLE_PANEL.of(true));
        } else if !query.is_valid() {
            return None;
        }
        effects.push(set_search_query(query));
    } else {
        effects.push(TOGGLE_PANEL.of(true));
        effects.push(Extension::append(SEARCH_STATE.extension()));
    }
    Some(Transaction::default().with_effects(effects))
}

/// Close an open panel without removing its query or extension.
pub fn close_search_panel(state: &EditorState) -> Option<Transaction> {
    search_panel_open(state).then(|| Transaction::default().with_effects([TOGGLE_PANEL.of(false)]))
}

pub(super) fn stored_query(state: &EditorState) -> Option<Arc<SearchQuery>> {
    search_state(state).map(|search| search.query.clone())
}

fn search_state(state: &EditorState) -> Option<Arc<SearchState>> {
    state
        .context()
        .field(&SEARCH_STATE)
        .expect("valid search field")
}

fn default_query(
    state: &StateContext<'_>,
    options: SearchOptions,
    fallback: Option<&SearchQuery>,
) -> SearchQuery {
    let selected = state
        .text()
        .slice(state.selection().main().range())
        .expect("valid selection");
    let selected = if selected.encode_utf16().take(101).count() > 100 {
        ""
    } else {
        selected
    };
    if selected.is_empty()
        && let Some(fallback) = fallback
    {
        return fallback.clone();
    }
    if !options.literal && selected.contains('\n') {
        SearchQuery::from_input(&selected.replace('\n', "\\n"), options)
    } else {
        SearchQuery::from_input(selected, options)
    }
}
