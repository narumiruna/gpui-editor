//! Explicit navigation in the active code fence of the attached draft.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::Context;
use gpui_editor_codemirror::{
    EditError, EditorSelection, Transaction,
    language::fold::{folded_ranges, unfold_effect},
    lsp::features::{
        DefinitionTarget, FeatureRequest, declaration_request, definition_request,
        implementation_request, type_definition_request,
    },
    view::{EditorView, PluginId, ViewPlugin, ViewUpdate},
};

use super::{Editor, Pending, RequestId, RpcResolution, StdioClient, Value, send_request};
use crate::document::Section;

const MAX_LOCATIONS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationKind {
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
}

/// A checked, single-use jump for one attachment and editor snapshot.
pub struct SourceLspNavigation {
    attachment: Rc<Cell<u64>>,
    generation: u64,
    target: Target,
    range: Range<usize>,
}

impl SourceLspNavigation {
    pub fn kind(&self) -> NavigationKind {
        self.target.kind
    }

    /// The full target range in draft UTF-8 bytes. Apply moves to its start.
    pub fn range(&self) -> &Range<usize> {
        &self.range
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
    head: usize,
    fence: Range<usize>,
    body: Range<usize>,
}

struct Target {
    stamp: Stamp,
    kind: NavigationKind,
}

pub(super) struct Navigation {
    selection_epoch: Rc<Cell<u64>>,
    plugin: PluginId,
    generation: u64,
    wanted: Option<Target>,
    pending: Option<Pending<Target, FeatureRequest<Vec<DefinitionTarget>>>>,
    checked: Option<SourceLspNavigation>,
}

impl Editor {
    /// Queue a jump query at the primary selection head in the editable fence.
    /// A valid new query supersedes earlier navigation results. Poll to check
    /// the server capability, sync the draft, and receive a result.
    pub fn request_source_lsp_navigation(&mut self, kind: NavigationKind) -> bool {
        let Some(stamp) = self
            .source_lsp
            .as_ref()
            .and_then(|lsp| lsp.navigation.stamp(self))
        else {
            return false;
        };
        let navigation = &mut self.source_lsp.as_mut().expect("attached").navigation;
        navigation.generation = navigation.generation.wrapping_add(1);
        navigation.wanted = Some(Target { stamp, kind });
        if let Some(pending) = &mut navigation.pending {
            pending.target = None;
        }
        navigation.checked = None;
        true
    }

    /// Take the first checked location without changing the draft or selection.
    pub fn take_source_lsp_navigation(&mut self) -> Option<SourceLspNavigation> {
        let result = self.source_lsp.as_mut()?.navigation.checked.take()?;
        self.source_lsp_navigation_is_current(&result)
            .then_some(result)
    }

    /// Move the caret and reveal the target without editing or recording history.
    /// Reject a stale result or a transaction changed by an extension.
    pub fn apply_source_lsp_navigation(
        &mut self,
        result: SourceLspNavigation,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.source_lsp_navigation_is_current(&result) {
            return false;
        }
        let offset = result.range.start;
        let selection = EditorSelection::single(offset, offset);
        let body = result.target.stamp.body;
        // Unfold as part of the selection transaction. A filter must not be
        // able to redirect the jump after the fold has already opened.
        let unfolds: Vec<_> = folded_ranges(self.view.state())
            .iter(offset)
            .take_while(|fold| fold.from < offset)
            .filter_map(|fold| {
                (body.start <= fold.from && offset < fold.to && fold.to <= body.end)
                    .then_some(fold.from..fold.to)
            })
            .collect();
        let transaction = Transaction::default()
            .with_selection(selection.clone())
            .with_effects(
                unfolds
                    .iter()
                    .cloned()
                    .map(|range| unfold_effect().of(range)),
            )
            .with_user_event("select.definition")
            .with_add_to_history(false);
        let active = self.active.clone();
        if self
            .view
            .update_checked(
                transaction,
                active.clone(),
                |accepted| {
                    if !accepted.changes().is_empty()
                        || accepted.add_to_history()
                        || accepted.history_isolation().is_some()
                        || accepted.scroll_into_view()
                        || !accepted.selection().is_some_and(|actual| {
                            actual.eq_with_assoc(&selection) && actual.main().bidi_level().is_none()
                        })
                        || !accepted
                            .effects()
                            .iter()
                            .map(|effect| effect.get(unfold_effect()))
                            .eq(unfolds.iter().map(Some))
                    {
                        return Err(EditError::InvalidSelection);
                    }
                    Ok(())
                },
                |_| (None, active),
            )
            .is_err()
        {
            return false;
        }
        if !unfolds.is_empty() {
            self.layout = None;
        }
        self.completion = None;
        self.rectangle = None;
        self.preferred_x = None;
        self.upstream = false;
        self.reveal_caret = false;
        self.view.take_scroll_target();
        self.reveal_source(offset, cx);
        true
    }

    fn source_lsp_navigation_is_current(&self, result: &SourceLspNavigation) -> bool {
        self.source_lsp.as_ref().is_some_and(|lsp| {
            Rc::ptr_eq(&lsp.navigation.selection_epoch, &result.attachment)
                && lsp.navigation.generation == result.generation
                && lsp.navigation.stamp(self).as_ref() == Some(&result.target.stamp)
        })
    }
}

impl Navigation {
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
        let head = editor.cursor();
        if !allows(editor.source(), &body, &(head..head)) {
            return None;
        }
        Some(Stamp {
            revision: editor.view.revision(),
            selection_epoch: self.selection_epoch.get(),
            head,
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
            .take_if(|result| Some(&result.target.stamp) != current.as_ref());
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
            |workspace, target| {
                let prepare = match target.kind {
                    NavigationKind::Definition => definition_request,
                    NavigationKind::Declaration => declaration_request,
                    NavigationKind::TypeDefinition => type_definition_request,
                    NavigationKind::Implementation => implementation_request,
                };
                prepare(workspace, uri, target.stamp.head)
            },
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
        let items = result
            .as_array()
            .map_or(std::slice::from_ref(&result), Vec::as_slice);
        if !(1..=MAX_LOCATIONS).contains(&items.len())
            || items
                .iter()
                .any(|item| item.get("uri").and_then(Value::as_str) != Some(uri))
        {
            return;
        }
        let locations =
            client.with_workspace(|workspace| pending.request.resolve(workspace, &result));
        let Ok(Some(locations)) = locations else {
            return;
        };
        // Do not search for another target if the server's first choice is not
        // safe. Reject the entire batch if any location is invalid or outside.
        if locations.iter().any(|location| {
            location
                .byte_range
                .as_ref()
                .is_none_or(|range| !allows(editor.source(), &target.stamp.body, range))
        }) {
            return;
        }
        if let Some(range) = locations.into_iter().next().and_then(|loc| loc.byte_range) {
            self.checked = Some(SourceLspNavigation {
                attachment: self.selection_epoch.clone(),
                generation: self.generation,
                target,
                range,
            });
        }
    }
}

fn allows(source: &str, body: &Range<usize>, range: &Range<usize>) -> bool {
    let valid = |offset: usize| {
        source.is_char_boundary(offset)
            && !(source.as_bytes().get(offset) == Some(&b'\n')
                && offset > 0
                && source.as_bytes()[offset - 1] == b'\r')
    };
    body.start <= range.start
        && range.start <= range.end
        && range.end <= body.end
        && valid(range.start)
        && valid(range.end)
}

#[cfg(test)]
mod tests;
