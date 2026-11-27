use std::ops::Range;

use gpui::Context;
use gpui_editor_codemirror::{
    EditError, EditorSelection, Transaction,
    lsp::{
        PositionIndex, Range as LspRange,
        features::FeatureRequest,
        references::{
            ReferenceLocation, ReferencePanel, ReferencePanelKeyAction, close_reference_panel,
            get_reference_panel, references_request,
        },
        rpc::{RequestId, RpcResolution},
    },
    view::panels::PanelId,
};
use serde_json::Value;

use super::{Editor, Pending, SectionKind, StdioClient, send_request};
use crate::document::{BlockPreview, Section};

const MAX_LOCATIONS: usize = 64;
const MAX_RANGE_BYTES: usize = 4096;
const MAX_FRAGMENT_CHARS: usize = 120;

/// A snapshot of the panel for the current attachment and document revision.
#[derive(Debug, Eq, PartialEq)]
pub struct ReferenceResults {
    pub id: PanelId,
    pub selected: usize,
    pub rows: Vec<ReferenceRow>,
}

/// Literal, single-line text from the attached draft. Do not interpret the
/// fragments as markup. A row does not contain a URI or a path.
#[derive(Debug, Eq, PartialEq)]
pub struct ReferenceRow {
    pub line_number: u32,
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Clone, Copy)]
struct Target {
    revision: u64,
    offset: usize,
}

#[derive(Default)]
pub(super) struct References {
    wanted: Option<Target>,
    pending: Option<Pending<Target, FeatureRequest<Vec<ReferenceLocation>>>>,
    panel_revision: Option<u64>,
}

impl Editor {
    /// Queue Find References at the primary selection head. Initialization and
    /// capability checks take place during host polling. A new target supersedes
    /// an in-flight query but does not close the current panel.
    pub fn request_source_lsp_references(&mut self) -> bool {
        let offset = self.cursor();
        if self.source_lsp.is_none()
            || !self.reference_input_allowed()
            || !ReferenceScope::new(self).allows(&(offset..offset))
        {
            return false;
        }
        let references = &mut self.source_lsp.as_mut().expect("attached").references;
        references.wanted = Some(Target {
            revision: self.view.revision(),
            offset,
        });
        true
    }

    /// Read bounded rows from the native panel. No panel is exposed after an
    /// edit, during composition, or while editor input is disabled.
    pub fn source_lsp_references(&self) -> Option<ReferenceResults> {
        let references = &self.source_lsp.as_ref()?.references;
        if !self.reference_input_allowed()
            || references.panel_revision != Some(self.view.revision())
        {
            return None;
        }
        let panel = get_reference_panel(self.view.state())?;
        Some(ReferenceResults {
            id: panel.id(),
            selected: panel.selected_index(),
            rows: panel
                .entries()
                .iter()
                .filter_map(|entry| entry.preview.as_ref())
                .map(|preview| ReferenceRow {
                    line_number: preview.line_number,
                    before: literal_fragment(&preview.before),
                    matched: literal_fragment(&preview.matched),
                    after: literal_fragment(&preview.after),
                })
                .collect(),
        })
    }

    /// Handle a key from the panel with this ID. The host owns keyboard focus.
    /// Enter moves the caret only if the selected range is still safe to reveal.
    pub fn source_lsp_reference_key(
        &mut self,
        id: PanelId,
        key: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.current_reference_panel(id) else {
            return false;
        };
        if key == "Escape" {
            let dismissed = self.dismiss_source_lsp_references();
            cx.notify();
            return dismissed;
        }
        match panel.key_action(key) {
            Some(ReferencePanelKeyAction::Keep) => true,
            Some(ReferencePanelKeyAction::Update(transaction)) => {
                let updated = apply_panel_effect(self, transaction);
                if updated {
                    cx.notify();
                }
                updated
            }
            Some(ReferencePanelKeyAction::Activate) => {
                let lsp = self
                    .source_lsp
                    .as_ref()
                    .expect("current panel has an attachment");
                let location = &panel.entries()[panel.selected_index()];
                let Some(preview) = &location.preview else {
                    return false;
                };
                let range = &preview.byte_range;
                if location.uri != lsp.uri || !ReferenceScope::new(self).allows(range) {
                    return false;
                }
                let target = range.start;
                if self
                    .source_fold_ranges()
                    .iter()
                    .any(|fold| fold.start < target && target < fold.end)
                {
                    return false;
                }
                let selection = EditorSelection::single(target, target);
                let Some(transaction) = lsp.client.with_workspace(|workspace| {
                    let (uri, transaction) = panel.activate(workspace).ok()?;
                    (uri == lsp.uri && transaction.selection() == Some(&selection))
                        .then_some(transaction)
                }) else {
                    return false;
                };
                let active = self.active.clone();
                if self
                    .view
                    .update_checked(
                        transaction.with_add_to_history(false),
                        active.clone(),
                        |accepted| check_selection_only(accepted, Some(&selection)),
                        |_| (None, active),
                    )
                    .is_err()
                {
                    return false;
                }
                let was_active = self.active.is_some();
                self.leave_source_if_outside(target);
                if was_active && self.active.is_none() {
                    self.layout = None;
                }
                self.completion = None;
                self.preferred_x = None;
                self.upstream = false;
                self.view.take_scroll_target();
                self.reveal_source(target, cx);
                true
            }
            None => false,
        }
    }

    pub(super) fn activate_source_lsp_reference_row(
        &mut self,
        id: PanelId,
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.current_reference_panel(id) else {
            return false;
        };
        if index >= panel.entries().len() {
            return false;
        }
        if let Some(transaction) = panel.select_row(index) {
            if !apply_panel_effect(self, transaction) {
                return false;
            }
            cx.notify();
            if self
                .current_reference_panel(id)
                .is_none_or(|panel| panel.selected_index() != index)
            {
                return false;
            }
        }
        self.source_lsp_reference_key(id, "Enter", cx)
    }

    fn current_reference_panel(&self, id: PanelId) -> Option<ReferencePanel> {
        let lsp = self.source_lsp.as_ref()?;
        if !self.reference_input_allowed()
            || lsp.references.panel_revision != Some(self.view.revision())
        {
            return None;
        }
        get_reference_panel(self.view.state()).filter(|panel| panel.id() == id)
    }

    /// Close the panel and retire queued or in-flight queries. Keep polling to
    /// drain the wire response. Notify a live editor after a successful close.
    pub fn dismiss_source_lsp_references(&mut self) -> bool {
        let Some(mut lsp) = self.source_lsp.take() else {
            return false;
        };
        let dismissed = lsp.references.dismiss(self);
        self.source_lsp = Some(lsp);
        dismissed
    }

    pub(super) fn reference_input_allowed(&self) -> bool {
        self.input_enabled && !self.is_composing() && !self.view.state().read_only()
    }
}

impl References {
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.wanted.is_none() && self.pending.is_none()
    }

    pub(super) fn can_send(&self) -> bool {
        self.wanted.is_some() && self.pending.is_none()
    }

    pub(super) fn retain_wanted(&mut self, allowed: bool, revision: u64) {
        self.wanted = self
            .wanted
            .filter(|target| allowed && target.revision == revision);
    }

    pub(super) fn send(&mut self, client: &mut StdioClient, uri: &str) -> bool {
        send_request(
            client,
            &mut self.wanted,
            &mut self.pending,
            |workspace, target| references_request(workspace, uri, target.offset),
        )
    }

    pub(super) fn is_pending(&self, id: RequestId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
    }

    pub(super) fn timed_out(&mut self, id: RequestId) {
        self.pending.take_if(|pending| pending.id == id);
    }

    pub(super) fn panel_is_stale(&self, revision: u64) -> bool {
        self.panel_revision.is_some_and(|opened| opened != revision)
    }

    pub(super) fn close(&mut self, editor: &mut Editor) -> bool {
        self.panel_revision = None;
        close_reference_panel(editor.view.state())
            .is_some_and(|transaction| apply_panel_effect(editor, transaction))
    }

    fn dismiss(&mut self, editor: &mut Editor) -> bool {
        self.wanted.take().is_some()
            | self
                .pending
                .as_mut()
                .and_then(|pending| pending.target.take())
                .is_some()
            | self.close(editor)
    }

    pub(super) fn resolve(
        &mut self,
        client: &StdioClient,
        uri: &str,
        editor: &mut Editor,
        reply: RpcResolution,
    ) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let revision = editor.view.revision();
        if reply.cancel_requested
            || self.wanted.is_some()
            || !editor.reference_input_allowed()
            || pending
                .target
                .is_none_or(|target| target.revision != revision)
        {
            return;
        }
        let Ok(Value::Array(locations)) = reply.outcome else {
            return;
        };
        if locations.is_empty() || locations.len() > MAX_LOCATIONS {
            return;
        }
        let state = editor.view.state();
        let index = PositionIndex::new(state.text());
        let scope = ReferenceScope::new(editor);
        let mut accepted = Vec::new();
        for location in locations {
            if location.get("uri").and_then(Value::as_str) != Some(uri) {
                continue;
            }
            let Ok(range) = location.get("range").unwrap_or(&Value::Null).try_into() else {
                return;
            };
            let LspRange { start, end } = range;
            let (Ok(start), Ok(end)) = (index.offset(start), index.offset(end)) else {
                return;
            };
            if start > end {
                return;
            }
            if scope.allows(&(start..end)) {
                accepted.push(location);
            }
        }
        if accepted.is_empty() {
            return;
        }
        let transaction = client.with_workspace(|workspace| {
            pending
                .request
                .resolve_panel(state, workspace, &Value::Array(accepted))
        });
        if let Ok(Some(transaction)) = transaction
            && apply_panel_effect(editor, transaction)
        {
            self.panel_revision = Some(revision);
        }
    }
}

fn apply_panel_effect(editor: &mut Editor, transaction: Transaction) -> bool {
    let active = editor.active.clone();
    editor
        .view
        .update_checked(
            transaction.without_filtering().with_add_to_history(false),
            active.clone(),
            |accepted| check_selection_only(accepted, None),
            |_| (None, active),
        )
        .is_ok()
}

fn check_selection_only(
    accepted: &Transaction,
    selection: Option<&EditorSelection>,
) -> Result<(), EditError> {
    if !accepted.changes().is_empty()
        || accepted.add_to_history()
        || accepted.selection() != selection
    {
        return Err(EditError::InvalidSelection);
    }
    Ok(())
}

pub(super) struct ReferenceScope<'a> {
    source: &'a str,
    sections: Vec<Section>,
    active: Option<(&'a Section, Option<Range<usize>>)>,
}

impl<'a> ReferenceScope<'a> {
    pub(super) fn new(editor: &'a Editor) -> Self {
        Self {
            source: editor.source(),
            sections: editor.markdown().sections(),
            active: editor.active.as_ref().map(|section| {
                let content = if section.kind == SectionKind::DraftCode {
                    editor.source_fence().map(|fence| fence.body)
                } else {
                    Some(section.content.clone())
                };
                (section, content)
            }),
        }
    }

    pub(super) fn allows(&self, range: &Range<usize>) -> bool {
        let valid_offset = |offset: usize| {
            self.source.is_char_boundary(offset)
                && !(self.source.as_bytes().get(offset) == Some(&b'\n')
                    && offset > 0
                    && self.source.as_bytes()[offset - 1] == b'\r')
        };
        let contains =
            |content: &Range<usize>| content.start <= range.start && range.end <= content.end;
        range.len() <= MAX_RANGE_BYTES
            && range.start <= range.end
            && valid_offset(range.start)
            && valid_offset(range.end)
            && self.active.as_ref().is_none_or(|(section, content)| {
                !intersects(&section.source, range) || content.as_ref().is_some_and(contains)
            })
            && self.sections.iter().all(|section| {
                if !intersects(&section.source, range) {
                    return true;
                }
                let content = match section.kind {
                    SectionKind::Heading
                    | SectionKind::Bullet
                    | SectionKind::Strong
                    | SectionKind::Emphasis => Some(&section.content),
                    SectionKind::Image | SectionKind::Link => None,
                    _ => self.active.as_ref().and_then(|(active, content)| {
                        let same_kind = active.kind == section.kind
                            || (active.kind == SectionKind::DraftCode
                                && matches!(section.preview, Some(BlockPreview::Code { .. })));
                        if active.source == section.source && same_kind {
                            content.as_ref()
                        } else {
                            None
                        }
                    }),
                };
                content.is_some_and(contains)
            })
    }
}

fn intersects(section: &Range<usize>, range: &Range<usize>) -> bool {
    if range.is_empty() {
        section.contains(&range.start)
    } else {
        range.start < section.end && section.start < range.end
    }
}

fn literal_fragment(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars
        .by_ref()
        .take(MAX_FRAGMENT_CHARS)
        .map(|ch| match ch {
            '\t' => ' ',
            '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => '�',
            ch if ch.is_control() => '�',
            ch => ch,
        })
        .collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

#[cfg(test)]
#[test]
fn reference_fragments_are_bounded_literal_single_line_text() {
    assert_eq!(literal_fragment("[link] <script>"), "[link] <script>");
    assert_eq!(literal_fragment("a\u{202e}b\t\nc"), "a�b �c");
    let long = literal_fragment(&"😀".repeat(MAX_FRAGMENT_CHARS + 1));
    assert_eq!(long.chars().count(), MAX_FRAGMENT_CHARS + 1);
    assert!(long.ends_with('…'));
}
