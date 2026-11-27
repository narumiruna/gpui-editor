use std::sync::{Arc, LazyLock};

use crate::{
    EditorState, Extension, Facet,
    view::{ViewPlugin, ViewPluginType, ViewUpdate, panels::get_panel},
};

use super::query_state::search_panel_id;

/// A custom search panel instance. The view owns the instance and calls its
/// lifecycle hooks. The host supplies content, input, and focus.
pub trait SearchPanel: 'static {
    /// Called once after creation for the registered search panel.
    fn mount(&mut self) {}

    /// Called on view updates that retain this panel, including reconfiguration.
    fn update(&mut self, _update: &ViewUpdate<'_>) {}

    /// Called once when this panel closes or its view lifecycle ends.
    fn destroy(&mut self) {}
}

type Factory = Arc<dyn Fn(&EditorState) -> Box<dyn SearchPanel> + Send + Sync>;

static FACTORY: LazyLock<Facet<Factory, Option<Factory>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Factory]| inputs.first().map(|input| (*input).clone()))
});

/// Set the factory for the next search panel in a view. The first factory in
/// extension precedence order wins. This does not install query state; use
/// [`super::extension`] to install it before opening a panel.
///
/// The factory receives the accepted open state. An open panel retains its
/// instance across factory changes. A close and reopen uses the new factory.
/// Without a factory, the host manages the stock panel as before. Set the
/// registry slot separately with [`super::search_panel_placement`].
pub fn search_panel_factory<P: SearchPanel>(
    create: impl Fn(&EditorState) -> P + Send + Sync + 'static,
) -> Extension {
    FACTORY.of(Arc::new(move |state| Box::new(create(state))))
}

// Reuse the binding so that a factory or placement change cannot reset the
// live view plugin. The search field provides this for both install paths.
static LIFECYCLE: LazyLock<Extension> = LazyLock::new(|| {
    ViewPluginType::define(|state, &()| {
        let mut lifecycle = SearchPanelLifecycle(None);
        if registered(state) {
            lifecycle.mount(state);
        }
        Ok(lifecycle)
    })
    .of(())
});

pub(super) fn lifecycle() -> Extension {
    LIFECYCLE.clone()
}

struct SearchPanelLifecycle(Option<Box<dyn SearchPanel>>);

impl SearchPanelLifecycle {
    fn mount(&mut self, state: &EditorState) {
        let factory = state
            .context()
            .facet(&FACTORY)
            .expect("valid search panel factory");
        if let Some(create) = factory.as_ref() {
            let mut panel = create(state);
            panel.mount();
            self.0 = Some(panel);
        }
    }
}

impl ViewPlugin for SearchPanelLifecycle {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        if !registered(update.state) {
            self.destroy();
        } else if let Some(panel) = &mut self.0 {
            panel.update(update);
        } else if !registered(update.start_state) {
            // A retained stock panel does not adopt a newly configured factory.
            self.mount(update.state);
        }
    }

    fn destroy(&mut self) {
        if let Some(mut panel) = self.0.take() {
            panel.destroy();
        }
    }
}

fn registered(state: &EditorState) -> bool {
    get_panel(state, search_panel_id()).is_some()
}
