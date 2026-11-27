use std::sync::{Arc, LazyLock};

use super::{Panel, PanelId, PanelPlacement, SHOW_PANEL, ViewUpdate, active_panels};
use crate::{EditorState, Extension, StateEffect, StateEffectType, StateField, Transaction};

/// A host identifier for custom content, a form, or a focus target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DialogElementId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DialogFocus {
    #[default]
    None,
    /// Focus the first input, or the first button when there is no input.
    First,
    Element(DialogElementId),
}

/// Input types for the built-in prompt description. The host owns editing
/// and validation. Other controls can use [`DialogContent::Custom`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DialogInputKind {
    #[default]
    Text,
    Password,
    Number,
    Email,
    Telephone,
    Url,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogInput {
    pub kind: DialogInputKind,
    /// An empty name uses `input`.
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogContent {
    Prompt {
        label: String,
        input: Option<DialogInput>,
        /// An empty label uses `OK`.
        submit_label: String,
    },
    Custom(DialogElementId),
}

impl Default for DialogContent {
    fn default() -> Self {
        Self::Prompt {
            label: String::new(),
            input: None,
            submit_label: "OK".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogConfig {
    pub placement: PanelPlacement,
    /// Extra identifier tokens. Pass each class name as a separate string.
    pub classes: Vec<String>,
    pub focus: DialogFocus,
    pub content: DialogContent,
}

/// A single opening of a dialog. Different openings have different IDs even
/// if their configurations are the same.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DialogId(PanelId);

impl DialogId {
    pub fn panel_id(self) -> PanelId {
        self.0
    }

    /// Close the dialog without a submission. The effect is a no-op if this
    /// dialog is not open. Use this for Escape, Cancel, and programmatic close.
    pub fn close(self) -> StateEffect {
        FINISH_DIALOG.of((self, DialogOutcome::Dismissed))
    }

    /// Close the dialog and deliver the supplied form data. Add the effect
    /// to the same transaction as any edit that accepts the input.
    pub fn submit(self, submission: DialogSubmission) -> StateEffect {
        FINISH_DIALOG.of((self, DialogOutcome::Submitted(submission)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dialog {
    id: DialogId,
    config: Arc<DialogConfig>,
}

impl Dialog {
    pub fn id(&self) -> DialogId {
        self.id
    }

    pub fn config(&self) -> &DialogConfig {
        &self.config
    }
}

/// Plain form data. Field names can repeat. A built-in prompt does not
/// need a form identifier.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogSubmission {
    pub form: Option<DialogElementId>,
    pub fields: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogOutcome {
    Submitted(DialogSubmission),
    Dismissed,
}

#[derive(Debug)]
pub struct DialogRequest {
    pub id: DialogId,
    pub transaction: Transaction,
}

/// Prepare a transaction to open a dialog. It installs dialog state on first
/// use. The host dispatches it and only acts on accepted view events.
///
/// A native close or submit effect ends the dialog in the accepting update.
/// There is no separate cleanup transaction or asynchronous form result.
pub fn show_dialog(state: &EditorState, mut config: DialogConfig) -> DialogRequest {
    if let DialogContent::Prompt {
        input,
        submit_label,
        ..
    } = &mut config.content
    {
        if let Some(input) = input.as_mut().filter(|input| input.name.is_empty()) {
            input.name = "input".into();
        }
        if submit_label.is_empty() {
            *submit_label = "OK".into();
        }
    }
    let id = DialogId(PanelId::new());
    let open = OPEN_DIALOG.of(Dialog {
        id,
        config: Arc::new(config),
    });
    let install = (dialog_list(state).is_none()).then(|| DIALOG_EXTENSION.clone().append());
    DialogRequest {
        id,
        transaction: Transaction::default().with_effects(install.into_iter().chain([open])),
    }
}

/// Find the newest open dialog with the given class token. `cm-dialog` and
/// `cm-panel` match all dialogs, just as they do upstream.
pub fn get_dialog(state: &EditorState, class_name: &str) -> Option<Dialog> {
    active_panels(state).iter().find_map(|panel| match panel {
        Panel::Dialog(dialog)
            if matches!(class_name, "cm-dialog" | "cm-panel")
                || dialog
                    .config
                    .classes
                    .iter()
                    .any(|class| class == class_name) =>
        {
            Some(dialog.clone())
        }
        _ => None,
    })
}

static OPEN_DIALOG: LazyLock<StateEffectType<Dialog>> = LazyLock::new(StateEffectType::new);
static FINISH_DIALOG: LazyLock<StateEffectType<(DialogId, DialogOutcome)>> =
    LazyLock::new(StateEffectType::new);

static DIALOG_LIST: LazyLock<StateField<Vec<Dialog>>> = LazyLock::new(|| {
    StateField::new(
        |_| Ok(Vec::<Dialog>::new()),
        |previous, update| {
            let mut dialogs = previous.clone();
            for effect in update.transaction().effects() {
                if let Some(dialog) = effect.get(&OPEN_DIALOG) {
                    if !dialogs.iter().any(|open| open.id == dialog.id) {
                        dialogs.insert(0, dialog.clone());
                    }
                } else if let Some((id, _)) = effect.get(&FINISH_DIALOG) {
                    dialogs.retain(|dialog| dialog.id != *id);
                }
            }
            Ok(dialogs)
        },
    )
});

static DIALOG_EXTENSION: LazyLock<Extension> = LazyLock::new(|| {
    Extension::group([
        DIALOG_LIST.extension(),
        SHOW_PANEL.compute_many(|state| {
            Ok(state
                .field(&DIALOG_LIST)?
                .iter()
                .flat_map(|dialogs| dialogs.iter())
                .map(|dialog| Some(Panel::Dialog(dialog.clone())))
                .collect())
        }),
    ])
});

fn dialog_list(state: &EditorState) -> Option<Arc<Vec<Dialog>>> {
    state
        .context()
        .field(&DIALOG_LIST)
        .expect("valid dialog list")
}

pub(super) fn outcome(update: Option<&ViewUpdate<'_>>, id: DialogId) -> DialogOutcome {
    // A removed field does not accept dialog effects. Destroying the view
    // or removing the configuration dismisses any live dialogs.
    update
        .filter(|update| dialog_list(update.state).is_some())
        .and_then(|update| {
            update
                .effects
                .iter()
                .find_map(|effect| match effect.get(&FINISH_DIALOG) {
                    Some((target, outcome)) if id == *target => Some(outcome.clone()),
                    _ => None,
                })
        })
        .unwrap_or(DialogOutcome::Dismissed)
}
