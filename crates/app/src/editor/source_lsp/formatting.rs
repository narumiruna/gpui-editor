//! Host-controlled formatting for the attached draft.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::Context;
use gpui_editor_codemirror::{
    Assoc, ChangeSet, HistoryIsolation, Transaction,
    lsp::{DocumentChanges, EditRequest, formatting_request},
    view::{EditorView, PluginId, ViewPlugin, ViewUpdate},
};

use super::{Editor, Pending, RequestId, RpcResolution, StdioClient, Value, send_request};
use crate::document::{BlockPreview, Markdown, Section, SectionKind};

pub use gpui_editor_codemirror::lsp::FormattingOptions;

const MAX_EDITS: usize = 256;

/// A checked, single-use format result for one editor and attachment.
/// The preview contains source text. Do not interpret it as markup or commands.
pub struct SourceLspFormatting {
    attachment: Rc<Cell<u64>>,
    generation: u64,
    stamp: Stamp,
    plan: DocumentChanges,
}

impl SourceLspFormatting {
    pub fn uri(&self) -> &str {
        &self.plan.uri
    }

    /// Simultaneous changes in raw UTF-8 byte coordinates.
    pub fn changes(&self) -> &ChangeSet {
        &self.plan.changes
    }
}

struct SelectionTracker(Rc<Cell<u64>>);

impl ViewPlugin for SelectionTracker {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        if update.selection_set || update.focus_changed {
            self.0.set(self.0.get().wrapping_add(1));
        }
    }
}

#[derive(Eq, PartialEq)]
struct Stamp {
    revision: u64,
    selection_epoch: u64,
    scope: Scope,
}

#[derive(Eq, PartialEq)]
enum Scope {
    Prose,
    Code {
        source: Range<usize>,
        body: Range<usize>,
    },
}

struct Target {
    stamp: Stamp,
    options: FormattingOptions,
}

pub(super) struct Formatting {
    selection_epoch: Rc<Cell<u64>>,
    plugin: PluginId,
    generation: u64,
    wanted: Option<Target>,
    pending: Option<Pending<Target, EditRequest<DocumentChanges>>>,
    checked: Option<SourceLspFormatting>,
}

impl Editor {
    /// Queue a whole-draft format request with host-supplied indentation options.
    /// Poll to send it and receive a result. A queued request can be discarded.
    /// A new request supersedes all earlier formatting results from this attachment.
    pub fn request_source_lsp_formatting(&mut self, options: FormattingOptions) -> bool {
        let Some(lsp) = &self.source_lsp else {
            return false;
        };
        if options.tab_size == 0
            || i32::try_from(options.tab_size).is_err()
            || lsp.client.server_capabilities().is_some_and(|caps| {
                !matches!(
                    caps.get("documentFormattingProvider"),
                    Some(Value::Bool(true) | Value::Object(_))
                )
            })
        {
            return false;
        }
        let Some(stamp) = lsp.formatting.stamp(self) else {
            return false;
        };
        let formatting = &mut self.source_lsp.as_mut().expect("attached").formatting;
        formatting.generation = formatting.generation.wrapping_add(1);
        formatting.wanted = Some(Target { stamp, options });
        if let Some(pending) = &mut formatting.pending {
            pending.target = None;
        }
        formatting.checked = None;
        true
    }

    /// Take a nonempty, checked result without changing the document.
    pub fn take_source_lsp_formatting(&mut self) -> Option<SourceLspFormatting> {
        let result = self.source_lsp.as_mut()?.formatting.checked.take()?;
        self.source_lsp_formatting_is_current(&result)
            .then_some(result)
    }

    /// Apply a checked result as one undo step. Recheck the snapshot and reject
    /// the whole batch if it is stale or the editor rejects the transaction.
    pub fn apply_source_lsp_formatting(
        &mut self,
        result: SourceLspFormatting,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.source_lsp_formatting_is_current(&result) {
            return false;
        }
        let transaction = Transaction::new(result.plan.changes)
            .with_user_event("format")
            .isolate_history(HistoryIsolation::Full);
        match result.stamp.scope {
            Scope::Prose => self.apply_prose_transaction(transaction, cx),
            Scope::Code { body, .. } => self.apply_source_transaction(transaction, body, cx),
        }
    }

    fn source_lsp_formatting_is_current(&self, result: &SourceLspFormatting) -> bool {
        self.source_lsp.as_ref().is_some_and(|lsp| {
            lsp.uri == result.plan.uri
                && Rc::ptr_eq(&lsp.formatting.selection_epoch, &result.attachment)
                && lsp.formatting.generation == result.generation
                && lsp.formatting.stamp(self).as_ref() == Some(&result.stamp)
        })
    }
}

impl Formatting {
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
        let scope = match &editor.active {
            Some(active) => Scope::Code {
                source: active.source.clone(),
                body: editor.source_fence()?.body,
            },
            None => Scope::Prose,
        };
        Some(Stamp {
            revision: editor.view.revision(),
            selection_epoch: self.selection_epoch.get(),
            scope,
        })
    }

    pub(super) fn retain_current(&mut self, editor: &Editor) {
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
            |workspace, target| formatting_request(workspace, uri, target.options),
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
        if reply.cancel_requested
            || self.stamp(editor).as_ref() != Some(&target.stamp)
            || !result
                .as_array()
                .is_some_and(|edits| (1..=MAX_EDITS).contains(&edits.len()))
        {
            return;
        }
        let plan = client.with_workspace(|workspace| pending.request.resolve(workspace, &result));
        if let Ok(Some(plan)) = plan
            && plan.uri == uri
            && preserves_markdown(editor, &target.stamp.scope, &plan.changes)
        {
            self.checked = Some(SourceLspFormatting {
                attachment: self.selection_epoch.clone(),
                generation: self.generation,
                stamp: target.stamp,
                plan,
            });
        }
    }
}

fn preserves_markdown(editor: &Editor, scope: &Scope, changes: &ChangeSet) -> bool {
    let before = editor.markdown().sections();
    if let Scope::Code { source, body } = scope
        && (!before.iter().any(|section| is_active_code(section, source))
            || !changes.changes().all(|(range, _)| contains(body, range)))
    {
        return false;
    }
    if !before.iter().all(|section| {
        let content = match scope {
            Scope::Code { source, body } if is_active_code(section, source) => Some(body),
            Scope::Prose
                if matches!(
                    section.kind,
                    SectionKind::Heading
                        | SectionKind::Bullet
                        | SectionKind::Strong
                        | SectionKind::Emphasis
                ) =>
            {
                Some(&section.content)
            }
            _ => None,
        };
        changes.changes().all(|(range, _)| {
            !intersects(&section.source, range)
                || content.is_some_and(|content| contains(content, range))
        })
    }) {
        return false;
    }

    // Edits at a boundary can hide a section or introduce a new delimiter.
    // Parse the entire edited draft and require the same sections at the mapped boundaries.
    let original = editor.view.state().text();
    let mut draft = original.clone();
    if changes.apply_to(&mut draft).is_err() || draft == *original {
        return false;
    }
    let after = Markdown::new(draft.as_str()).sections();
    before.len() == after.len()
        && before.iter().zip(&after).all(|(old, new)| {
            let end = if matches!(scope, Scope::Code { source, body } if is_active_code(old, source) && body.end == source.end) {
                // An unclosed draft ends at the end of its editable body.
                Assoc::After
            } else {
                Assoc::Before
            };
            old.kind == new.kind
                && new.source.start == changes.map_pos(old.source.start, Assoc::After)
                && new.source.end == changes.map_pos(old.source.end, end)
        })
}

fn is_active_code(section: &Section, source: &Range<usize>) -> bool {
    section.source == *source
        && (section.kind == SectionKind::DraftCode
            || matches!(section.preview, Some(BlockPreview::Code { .. })))
}

fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn intersects(section: &Range<usize>, range: &Range<usize>) -> bool {
    if range.is_empty() {
        section.contains(&range.start)
    } else {
        range.start < section.end && section.start < range.end
    }
}

#[cfg(test)]
mod tests;
