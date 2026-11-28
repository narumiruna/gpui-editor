//! Headless panels and dialogs from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.
//!
//! The editor provides order, placement, and lifecycle events. The host owns
//! content, input, and focus. An ID identifies a single panel. In contrast to
//! repeated upstream constructors, repeated native IDs use the first input.
//! Changing a container moves a live panel without destroying it.

use std::{
    collections::HashSet,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{ViewPlugin, ViewUpdate};
use crate::{EditorState, Extension, Facet};

mod dialogs;

pub use dialogs::{
    Dialog, DialogConfig, DialogContent, DialogElementId, DialogFocus, DialogId, DialogInput,
    DialogInputKind, DialogOutcome, DialogRequest, DialogSubmission, get_dialog, show_dialog,
};

/// The stable identity of a panel, independent of its position or container.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PanelId(u64);

impl PanelId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("panel IDs exhausted"),
        )
    }
}

impl Default for PanelId {
    fn default() -> Self {
        Self::new()
    }
}

/// The host's identifier for an alternative panel container.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PanelContainerId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PanelPlacement {
    Top,
    #[default]
    Bottom,
}

/// Panel data for the host. The host supplies content for a `Custom` panel
/// by its ID. A `Dialog` includes a description of its content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Panel {
    Custom {
        id: PanelId,
        placement: PanelPlacement,
    },
    Dialog(Dialog),
}

impl Panel {
    pub fn id(&self) -> PanelId {
        match self {
            Self::Custom { id, .. } => *id,
            Self::Dialog(dialog) => dialog.id().panel_id(),
        }
    }

    pub fn placement(&self) -> PanelPlacement {
        match self {
            Self::Custom { placement, .. } => *placement,
            Self::Dialog(dialog) => dialog.config().placement,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PanelConfig {
    pub top_container: Option<PanelContainerId>,
    pub bottom_container: Option<PanelContainerId>,
}

static PANEL_CONFIG: LazyLock<Facet<PanelConfig, PanelConfig>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&PanelConfig]| PanelConfig {
        top_container: inputs.iter().find_map(|input| input.top_container),
        bottom_container: inputs.iter().find_map(|input| input.bottom_container),
    })
});

/// Set panel containers. The first defined container for each side wins in
/// extension precedence order. `None` uses the editor's default container.
pub fn panels(config: PanelConfig) -> Extension {
    PANEL_CONFIG.of(config)
}

pub fn panel_config(state: &EditorState) -> PanelConfig {
    *state
        .context()
        .facet(&PANEL_CONFIG)
        .expect("valid panel config")
}

type PanelInputs = Facet<Option<Panel>, Vec<Panel>>;

/// Open panels through extension inputs. Remove an input to close its panel.
/// `None` is ignored. Inputs follow extension precedence order.
pub static SHOW_PANEL: LazyLock<PanelInputs> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Option<Panel>]| {
        let mut seen = HashSet::new();
        inputs
            .iter()
            .filter_map(|input| input.as_ref())
            .filter(|panel| seen.insert(panel.id()))
            .cloned()
            .collect()
    })
});

pub fn active_panels(state: &EditorState) -> Arc<Vec<Panel>> {
    state
        .context()
        .facet(&SHOW_PANEL)
        .expect("valid panel list")
}

pub fn get_panel(state: &EditorState, id: PanelId) -> Option<Panel> {
    active_panels(state)
        .iter()
        .find(|panel| panel.id() == id)
        .cloned()
}

/// The location of one active panel. `index` is zero-based within its side.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PanelSlot {
    pub panel: Panel,
    pub container: Option<PanelContainerId>,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PanelEvent {
    Mounted(PanelSlot),
    Updated(PanelSlot),
    Destroyed(PanelSlot),
    /// Sent once per destroyed dialog, after its `Destroyed` event.
    DialogFinished(DialogId, DialogOutcome),
}

/// Observe accepted view transitions. Add this as a view plugin if the host
/// needs lifecycle events. A rejected transaction emits no events.
///
/// On update, retained panels update in new facet order. Removed panels are
/// destroyed from top to bottom in old order. New panels mount in new facet
/// order. Unregistering the plugin or destroying the view ends the observed
/// panel lifecycle. The plugin does not change editor state.
pub struct PanelLifecycle<F> {
    active: Vec<PanelSlot>,
    emit: F,
}

impl<F: FnMut(PanelEvent)> PanelLifecycle<F> {
    /// Use the view's current state. Report the initial mounts in facet order,
    /// then track the active panels.
    pub fn new(state: &EditorState, mut emit: F) -> Self {
        let active = slots(state);
        for slot in &active {
            emit(PanelEvent::Mounted(slot.clone()));
        }
        Self { active, emit }
    }

    fn remove(&mut self, active: Vec<PanelSlot>, update: Option<&ViewUpdate<'_>>) {
        for placement in [PanelPlacement::Top, PanelPlacement::Bottom] {
            for slot in active
                .iter()
                .filter(|slot| slot.panel.placement() == placement)
            {
                (self.emit)(PanelEvent::Destroyed(slot.clone()));
                if let Panel::Dialog(dialog) = &slot.panel {
                    (self.emit)(PanelEvent::DialogFinished(
                        dialog.id(),
                        dialogs::outcome(update, dialog.id()),
                    ));
                }
            }
        }
    }
}

impl<F: FnMut(PanelEvent)> ViewPlugin for PanelLifecycle<F> {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let next = slots(update.state);
        let previous_ids: HashSet<_> = self.active.iter().map(|slot| slot.panel.id()).collect();
        let next_ids: HashSet<_> = next.iter().map(|slot| slot.panel.id()).collect();
        for slot in next
            .iter()
            .filter(|slot| previous_ids.contains(&slot.panel.id()))
        {
            (self.emit)(PanelEvent::Updated(slot.clone()));
        }
        let mut removed = std::mem::replace(&mut self.active, next);
        removed.retain(|slot| !next_ids.contains(&slot.panel.id()));
        self.remove(removed, Some(update));
        for slot in self
            .active
            .iter()
            .filter(|slot| !previous_ids.contains(&slot.panel.id()))
        {
            (self.emit)(PanelEvent::Mounted(slot.clone()));
        }
    }

    fn destroy(&mut self) {
        let active = std::mem::take(&mut self.active);
        self.remove(active, None);
    }
}

fn slots(state: &EditorState) -> Vec<PanelSlot> {
    let config = panel_config(state);
    let (mut top, mut bottom) = (0, 0);
    active_panels(state)
        .iter()
        .map(|panel| {
            let (container, counter) = match panel.placement() {
                PanelPlacement::Top => (config.top_container, &mut top),
                PanelPlacement::Bottom => (config.bottom_container, &mut bottom),
            };
            let index = *counter;
            *counter += 1;
            PanelSlot {
                panel: panel.clone(),
                container,
                index,
            }
        })
        .collect()
}
