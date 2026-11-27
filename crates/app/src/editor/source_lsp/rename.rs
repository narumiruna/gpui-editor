//! Explicit rename in the active code fence of the attached draft.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::Context;
use gpui_editor_codemirror::{
    Assoc, ChangeSet, EditError, HistoryIsolation, Transaction,
    lsp::{DocumentChanges, EditRequest, PositionIndex, Range as LspRange, rename_request},
    view::{EditorView, PluginId, ViewPlugin, ViewUpdate},
};

use super::{Editor, Pending, RequestId, RpcResolution, StdioClient, Value, send_request};
use crate::document::{BlockPreview, Markdown, Section, SectionKind};

const MAX_EDITS: usize = 256;
const MAX_NAME_BYTES: usize = 1024;

/// A single-use rename batch for one attachment and editor snapshot.
pub struct SourceLspRename {
    attachment: Rc<Cell<u64>>,
    generation: u64,
    stamp: Stamp,
    plan: DocumentChanges,
    active_after: Section,
}

impl SourceLspRename {
    /// The attached draft identifier. Do not use it to open a file.
    pub fn uri(&self) -> &str {
        &self.plan.uri
    }

    /// Simultaneous edits in draft UTF-8 byte coordinates. Treat inserted text
    /// as literal source in a preview.
    pub fn changes(&self) -> &ChangeSet {
        &self.plan.changes
    }
}

struct SelectionTracker(Rc<Cell<u64>>);

impl ViewPlugin for SelectionTracker {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        if update.selection_set
            || update.focus_changed
            || update.start_state.read_only() != update.state.read_only()
        {
            self.0.set(self.0.get().wrapping_add(1));
        }
    }
}

#[derive(Eq, PartialEq)]
struct Stamp {
    revision: u64,
    selection_epoch: u64,
    fence: Range<usize>,
    body: Range<usize>,
}

struct Target {
    stamp: Stamp,
    offset: usize,
    new_name: String,
}

pub(super) struct Rename {
    selection_epoch: Rc<Cell<u64>>,
    plugin: PluginId,
    generation: u64,
    wanted: Option<Target>,
    pending: Option<Pending<Target, EditRequest<Vec<DocumentChanges>>>>,
    checked: Option<SourceLspRename>,
}

impl Editor {
    /// Queue a rename at an explicit draft byte offset. The host supplies the
    /// new name and position. The name must be nonblank, at most 1024 bytes,
    /// and contain no control characters. Poll to sync and check the reply.
    /// A valid new request supersedes earlier results. A queued request may
    /// not produce a result. There is no prompt or prepare request.
    pub fn request_source_lsp_rename(&mut self, offset: usize, new_name: &str) -> bool {
        let Some(lsp) = &self.source_lsp else {
            return false;
        };
        if new_name.len() > MAX_NAME_BYTES
            || new_name.trim().is_empty()
            || new_name.chars().any(char::is_control)
            || lsp.client.server_capabilities().is_some_and(|caps| {
                !matches!(
                    caps.get("renameProvider"),
                    Some(Value::Bool(true) | Value::Object(_))
                )
            })
        {
            return false;
        }
        let Some(stamp) = lsp.rename.stamp(self) else {
            return false;
        };
        if !allows_offset(self.source(), &stamp.body, offset) {
            return false;
        }
        let rename = &mut self.source_lsp.as_mut().expect("attached").rename;
        rename.generation = rename.generation.wrapping_add(1);
        rename.wanted = Some(Target {
            stamp,
            offset,
            new_name: new_name.into(),
        });
        if let Some(pending) = &mut rename.pending {
            pending.target = None;
        }
        rename.checked = None;
        true
    }

    /// Take a checked result without changing the draft or selection.
    pub fn take_source_lsp_rename(&mut self) -> Option<SourceLspRename> {
        let result = self.source_lsp.as_mut()?.rename.checked.take()?;
        self.source_lsp_rename_is_current(&result).then_some(result)
    }

    /// Apply the whole batch as one undo step. Recheck the snapshot and reject
    /// any extension rewrite that would change the edit, selection, effects,
    /// or undo behavior.
    pub fn apply_source_lsp_rename(
        &mut self,
        result: SourceLspRename,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.source_lsp_rename_is_current(&result) {
            return false;
        }
        let changes = result.plan.changes;
        let transaction = Transaction::new(changes.clone())
            .with_user_event("rename")
            .isolate_history(HistoryIsolation::Full);
        let applied = self.view.update_checked(
            transaction,
            self.active.clone(),
            |accepted| {
                if accepted.changes() != &changes
                    || accepted.selection().is_some()
                    || !accepted.effects().is_empty()
                    || accepted.scroll_into_view()
                    || accepted.user_event() != Some("rename")
                    || !accepted.add_to_history()
                    || accepted.history_isolation() != Some(HistoryIsolation::Full)
                {
                    return Err(EditError::InvalidRange);
                }
                Ok(())
            },
            |_| (None, Some(result.active_after)),
        );
        let Ok(active) = applied else {
            return false;
        };
        self.active = active;
        self.invalidate_edit_layout();
        self.message = None;
        cx.notify();
        true
    }

    fn source_lsp_rename_is_current(&self, result: &SourceLspRename) -> bool {
        self.source_lsp.as_ref().is_some_and(|lsp| {
            lsp.uri == result.plan.uri
                && Rc::ptr_eq(&lsp.rename.selection_epoch, &result.attachment)
                && lsp.rename.generation == result.generation
                && lsp.rename.stamp(self).as_ref() == Some(&result.stamp)
        })
    }
}

impl Rename {
    pub(super) fn attach(view: &mut EditorView<Option<Section>>) -> Self {
        let selection_epoch = Rc::new(Cell::new(0));
        let plugin = view.add_plugin(SelectionTracker(selection_epoch.clone()));
        Self {
            selection_epoch,
            plugin,
            generation: 0,
            wanted: None,
            pending: None,
            checked: None,
        }
    }

    pub(super) fn plugin(&self) -> PluginId {
        self.plugin
    }

    fn stamp(&self, editor: &Editor) -> Option<Stamp> {
        if !editor.reference_input_allowed() || editor.selecting {
            return None;
        }
        let body = editor.source_fence_with_unlabeled(true)?.body;
        if !allows_offset(editor.source(), &body, editor.cursor()) {
            return None;
        }
        Some(Stamp {
            revision: editor.view.revision(),
            selection_epoch: self.selection_epoch.get(),
            fence: editor.active.as_ref()?.source.clone(),
            body,
        })
    }

    pub(super) fn retain_current(&mut self, editor: &Editor) {
        if self.wanted.is_none() && self.pending.is_none() && self.checked.is_none() {
            return;
        }
        let current = self.stamp(editor);
        self.wanted
            .take_if(|target| Some(&target.stamp) != current.as_ref());
        if let Some(pending) = &mut self.pending {
            pending
                .target
                .take_if(|target| Some(&target.stamp) != current.as_ref());
        }
        self.checked
            .take_if(|result| Some(&result.stamp) != current.as_ref());
    }

    pub(super) fn discard_wanted(&mut self) {
        self.wanted = None;
    }

    pub(super) fn can_send(&self) -> bool {
        self.wanted.is_some() && self.pending.is_none()
    }

    pub(super) fn send(&mut self, client: &mut StdioClient, uri: &str) -> bool {
        send_request(
            client,
            &mut self.wanted,
            &mut self.pending,
            |workspace, target| rename_request(workspace, uri, target.offset, &target.new_name),
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

    pub(super) fn resolve(
        &mut self,
        client: &StdioClient,
        uri: &str,
        editor: &Editor,
        reply: RpcResolution,
    ) {
        let Some(pending) = self.pending.take_if(|pending| pending.id == reply.id) else {
            return;
        };
        let (Some(target), Ok(result)) = (pending.target, reply.outcome) else {
            return;
        };
        if reply.cancel_requested || self.stamp(editor).as_ref() != Some(&target.stamp) {
            return;
        }
        let Some(edits) = same_draft_edits(&result, uri) else {
            return;
        };
        let Some(mut plans) = client.with_workspace(|workspace| {
            let file = workspace.file(uri)?;
            // Do not use the core's versionless rebasing in the editor.
            if file.version() != pending.request.version()
                || file.has_pending_changes()
                || file.synced() != editor.view.state().text()
            {
                return None;
            }
            pending.request.resolve(workspace, &result).ok().flatten()
        }) else {
            return;
        };
        if plans.len() != 1 || plans[0].uri != uri {
            return;
        }
        let plan = plans.pop().expect("one plan");
        if let Some(active_after) = checked_fence(editor, &target.stamp, edits, &plan.changes) {
            self.checked = Some(SourceLspRename {
                attachment: self.selection_epoch.clone(),
                generation: self.generation,
                stamp: target.stamp,
                plan,
                active_after,
            });
        }
    }
}

fn allows_offset(source: &str, body: &Range<usize>, offset: usize) -> bool {
    (body.start..=body.end).contains(&offset)
        && source.is_char_boundary(offset)
        && !(source.as_bytes().get(offset) == Some(&b'\n')
            && source.as_bytes().get(offset.wrapping_sub(1)) == Some(&b'\r'))
}

// Do not pick one representation while overlooking an unsafe second batch.
fn same_draft_edits<'a>(result: &'a Value, uri: &str) -> Option<&'a [Value]> {
    let edits = match (result.get("changes"), result.get("documentChanges")) {
        (Some(changes), None) if changes.as_object()?.len() == 1 => changes.get(uri)?,
        (None, Some(documents)) => {
            let [document] = documents.as_array()?.as_slice() else {
                return None;
            };
            if document.get("textDocument")?.get("uri")?.as_str()? != uri {
                return None;
            }
            document.get("edits")?
        }
        _ => return None,
    }
    .as_array()?;
    (1..=MAX_EDITS).contains(&edits.len()).then_some(edits)
}

fn checked_fence(
    editor: &Editor,
    stamp: &Stamp,
    edits: &[Value],
    changes: &ChangeSet,
) -> Option<Section> {
    let original = editor.view.state().text();
    let positions = PositionIndex::new(original);
    let before = editor.markdown().sections();
    let is_fence = |section: &Section| section.source == stamp.fence && is_code(section);
    if !before.iter().any(is_fence) {
        return None;
    }
    // Check raw ranges too: the core drops empty no-ops when it builds a set.
    for edit in edits {
        let range = LspRange::try_from(edit.get("range")?).ok()?;
        let start = positions.offset(range.start).ok()?;
        let end = positions.offset(range.end).ok()?;
        if start < stamp.body.start
            || end > stamp.body.end
            || before.iter().any(|section| {
                section.kind == SectionKind::ReadOnly
                    && !is_fence(section)
                    && start < section.source.end
                    && section.source.start < end
            })
        {
            return None;
        }
    }
    let mut draft = original.clone();
    changes.apply_to(&mut draft).ok()?;
    if draft == *original {
        return None;
    }
    let body_end = changes.map_pos(stamp.body.end, Assoc::After);
    // An inserted CR must not join a protected LF separator.
    if original.as_str()[stamp.body.end..].starts_with('\n')
        && draft.as_str()[..body_end].ends_with('\r')
        && !original.as_str()[..stamp.body.end].ends_with('\r')
    {
        return None;
    }
    let after = Markdown::new(draft.as_str()).sections();
    if before.len() != after.len()
        || !before.iter().zip(&after).all(|(old, new)| {
            let fence = is_fence(old);
            let end = if fence { Assoc::After } else { Assoc::Before };
            old.kind == new.kind
                && (!fence || is_code(new))
                && new.source.start == changes.map_pos(old.source.start, Assoc::After)
                && new.source.end == changes.map_pos(old.source.end, end)
        })
    {
        return None;
    }
    let mut active = editor.active.clone()?;
    active.content.end = changes.map_pos(active.content.end, Assoc::After);
    active.source.end = changes.map_pos(active.source.end, Assoc::After);
    Some(active)
}

fn is_code(section: &Section) -> bool {
    section.kind == SectionKind::DraftCode
        || matches!(section.preview, Some(BlockPreview::Code { .. }))
}

#[cfg(test)]
mod tests;
