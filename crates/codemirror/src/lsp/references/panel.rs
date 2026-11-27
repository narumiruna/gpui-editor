use std::sync::{Arc, LazyLock};

use serde_json::Value;

use crate::{
    EditorSelection, EditorState, StateEffectType, StateField, StateFieldOptions, Transaction,
    view::panels::{Panel, PanelId, PanelPlacement, SHOW_PANEL},
};

use super::{
    DocumentWorkspace, FeatureError, FeatureRequest, PositionIndex, ReferenceLocation,
    RequestDocument, byte_range, mapped_range, stale,
};

/// One opening of the references panel. Entries and preview text are fixed
/// at resolution time. Use the locations and text as data, not paths or HTML.
#[derive(Clone, Debug)]
pub struct ReferencePanel {
    id: PanelId,
    request: Arc<RequestDocument>,
    entries: Arc<[ReferenceLocation]>,
    selected: usize,
}

impl ReferencePanel {
    pub fn id(&self) -> PanelId {
        self.id
    }

    /// Only references to documents open when the request started. All rows
    /// have previews. Headers do not count as rows. Keep order and duplicates.
    pub fn entries(&self) -> &[ReferenceLocation] {
        &self.entries
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Handle a key in the panel, not in the editor. Use the current panel
    /// from [`get_reference_panel`]. The host owns event handling and focus,
    /// including returning focus to the editor on Escape.
    pub fn key_action(&self, key: &str) -> Option<ReferencePanelKeyAction> {
        let last = self.entries.len() - 1;
        let index = match key {
            "Escape" => return Some(ReferencePanelKeyAction::Update(self.close())),
            "Enter" => return Some(ReferencePanelKeyAction::Activate),
            "ArrowUp" | "PageUp" => self.selected.checked_sub(1).unwrap_or(last),
            "ArrowDown" | "PageDown" => {
                if self.selected == last {
                    0
                } else {
                    self.selected + 1
                }
            }
            "Home" => 0,
            "End" => last,
            _ => return None,
        };
        Some(self.select_row(index).map_or(
            ReferencePanelKeyAction::Keep,
            ReferencePanelKeyAction::Update,
        ))
    }

    /// Select a row in this panel, without changing the document selection.
    /// Return `None` for an invalid or already selected row. An effect for a
    /// closed or replaced panel does not change its replacement.
    pub fn select_row(&self, index: usize) -> Option<Transaction> {
        (index < self.entries.len() && index != self.selected).then(|| {
            Transaction::default()
                .with_effects([PANEL_EFFECT.of(PanelEffect::Select(self.id, index))])
        })
    }

    /// Build a cursor and scroll transaction for the selected reference in
    /// the current draft. Dispatch it only to the editor attached to the
    /// returned URI. This does not open a file, focus an editor, or close the
    /// panel. It rejects a touched range or a closed or replaced snapshot.
    pub fn activate(
        &self,
        workspace: &DocumentWorkspace,
    ) -> Result<(&str, Transaction), FeatureError> {
        self.request.changes(workspace)?;
        let location = &self.entries[self.selected];
        let uri = location.uri.as_str();
        let (version, text) = &self.request.targets[uri];
        let changes = self
            .request
            .mapping
            .get_mapping(uri)
            .ok_or_else(|| stale(workspace, uri, *version))?;
        let range = byte_range(&PositionIndex::new(text), location.range)?;
        let offset = mapped_range(&changes, range)
            .ok_or_else(|| stale(workspace, uri, *version))?
            .start;
        Ok((
            uri,
            Transaction::default()
                .with_selection(EditorSelection::single(offset, offset))
                .with_scroll_into_view(),
        ))
    }

    fn close(&self) -> Transaction {
        Transaction::default().with_effects([PANEL_EFFECT.of(PanelEffect::Close(self.id))])
    }
}

/// A handled panel key. Unrecognized keys return `None` from `key_action`.
#[derive(Debug)]
pub enum ReferencePanelKeyAction {
    /// Selection is already at the requested row. No transaction is necessary.
    Keep,
    Update(Transaction),
    /// Resolve the selected reference with [`ReferencePanel::activate`].
    Activate,
}

impl FeatureRequest<Vec<ReferenceLocation>> {
    /// Resolve the matching result for the editor that started this request.
    /// A nonempty list of previews opens or replaces the bottom panel and
    /// selects its first row. Null, empty, and all-unavailable results return
    /// `None`, leaving the current panel and its selection unchanged.
    /// Dispatch the returned transaction against the supplied editor state.
    pub fn resolve_panel(
        &self,
        state: &EditorState,
        workspace: &DocumentWorkspace,
        result: &Value,
    ) -> Result<Option<Transaction>, FeatureError> {
        let Some(mut entries) = self.resolve(workspace, result)? else {
            return Ok(None);
        };
        entries.retain(|entry| entry.preview.is_some());
        if entries.is_empty() {
            return Ok(None);
        }
        let uri = &self.document.uri;
        if workspace.file(uri).map(|file| file.draft()) != Some(state.text()) {
            return Err(stale(workspace, uri, self.version()));
        }
        let panel = ReferencePanel {
            id: PanelId::new(),
            request: Arc::new(self.document.clone()),
            entries: entries.into(),
            selected: 0,
        };
        let install = panel_state(state)
            .is_none()
            .then(|| REFERENCE_PANEL.extension().append());
        Ok(Some(
            Transaction::default().with_effects(
                install
                    .into_iter()
                    .chain([PANEL_EFFECT.of(PanelEffect::Open(panel))]),
            ),
        ))
    }
}

/// Get the current list and selected row for the host panel.
pub fn get_reference_panel(state: &EditorState) -> Option<ReferencePanel> {
    panel_state(state)?.as_ref().clone()
}

/// Close the current references panel. Return `None` if it is not open.
/// Closing a panel does not cancel pending requests.
pub fn close_reference_panel(state: &EditorState) -> Option<Transaction> {
    get_reference_panel(state).map(|panel| panel.close())
}

fn panel_state(state: &EditorState) -> Option<Arc<Option<ReferencePanel>>> {
    state
        .context()
        .field(&REFERENCE_PANEL)
        .expect("valid reference panel")
}

enum PanelEffect {
    Open(ReferencePanel),
    Select(PanelId, usize),
    Close(PanelId),
}

static PANEL_EFFECT: LazyLock<StateEffectType<PanelEffect>> = LazyLock::new(StateEffectType::new);

static REFERENCE_PANEL: LazyLock<StateField<Option<ReferencePanel>>> = LazyLock::new(|| {
    StateField::<Option<ReferencePanel>>::define_shared(
        |_| Ok(Arc::new(None)),
        |previous, update| {
            let current = previous.as_ref().as_ref();
            for effect in update.transaction().effects() {
                match effect.get(&PANEL_EFFECT) {
                    Some(PanelEffect::Open(panel)) => return Ok(Arc::new(Some(panel.clone()))),
                    Some(PanelEffect::Select(id, selected)) => {
                        if let Some(panel) = current.filter(|panel| panel.id == *id) {
                            return Ok(Arc::new(Some(ReferencePanel {
                                selected: *selected,
                                ..panel.clone()
                            })));
                        }
                    }
                    Some(PanelEffect::Close(id))
                        if current.is_some_and(|panel| panel.id == *id) =>
                    {
                        return Ok(Arc::new(None));
                    }
                    _ => {}
                }
            }
            Ok(previous.clone())
        },
        StateFieldOptions::default().provide(|field| {
            SHOW_PANEL.from_field(field, |panel: &Option<ReferencePanel>| {
                panel.as_ref().map(|panel| Panel::Custom {
                    id: panel.id,
                    placement: PanelPlacement::Bottom,
                })
            })
        }),
    )
});
