//! Read-only signature help for the attached draft.

use std::{
    cell::{Cell, Ref, RefCell},
    io,
    ops::Range,
    rc::Rc,
    time::Duration,
};

use gpui::{
    App, AsyncWindowContext, Bounds, Context, KeyBinding, Pixels, Task, WeakEntity, Window, actions,
};
use gpui_editor_codemirror::{
    Assoc, ChangeDesc, MapMode,
    lsp::signature::{
        SignatureEffect, SignaturePlan, SignatureRequest, SignatureTrigger, signature_request,
    },
    view::{EditorView, PluginId, ViewPlugin, ViewUpdate},
};

use super::{
    Documentation, Editor, Layout, Overlay, Pending, RequestId, RpcResolution, StdioClient, Value,
    display_text, hover_anchor, references::ReferenceScope, send_request,
};
use crate::document::Section;

actions!(
    source_lsp,
    [ShowSignatureHelp, PreviousSignature, NextSignature]
);

pub(in crate::editor) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-shift-space", ShowSignatureHelp, Some("Editor")),
        KeyBinding::new("cmd-shift-up", PreviousSignature, Some("Editor")),
        KeyBinding::new("cmd-shift-down", NextSignature, Some("Editor")),
    ]);
}

const CONTENT_CHANGE_DELAY: Duration = Duration::from_millis(250);
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_SIGNATURES: usize = 16;
const MAX_PARAMETERS: usize = 32;

#[derive(Default)]
struct Events {
    selection_epoch: Cell<u64>,
    typed: Cell<Option<char>>,
    inserted: Cell<bool>,
    shown: RefCell<Option<Shown>>,
    delayed: Cell<Option<Task<()>>>,
}

impl Events {
    fn close(&self) -> bool {
        self.delayed.take();
        self.shown.take().is_some()
    }
}

#[derive(Clone)]
pub(in crate::editor) struct SignatureTimer {
    window: AsyncWindowContext,
    editor: WeakEntity<Editor>,
}

impl SignatureTimer {
    pub(in crate::editor) fn new(window: &Window, cx: &Context<Editor>) -> Self {
        Self {
            window: window.to_async(cx),
            editor: cx.weak_entity(),
        }
    }

    fn start(&self) -> Task<()> {
        let timer = self
            .window
            .background_executor()
            .timer(CONTENT_CHANGE_DELAY);
        let editor = self.editor.clone();
        self.window.spawn(async move |cx| {
            timer.await;
            let _ = editor.update_in(cx, |editor, window, cx| {
                editor.retrigger_source_lsp_signature(window);
                cx.notify();
            });
        })
    }
}

struct EventTracker {
    events: Rc<Events>,
    timer: SignatureTimer,
}

impl ViewPlugin for EventTracker {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let events = &self.events;
        let invalidated =
            update.focus_changed || update.start_state.read_only() != update.state.read_only();
        if update.selection_set || invalidated {
            events
                .selection_epoch
                .set(events.selection_epoch.get().wrapping_add(1));
        }
        // Use the accepted changes, not only the text the input handler saw.
        // The caret can be inside a paired insertion such as `()`.
        let head = update.state.selection().main().head();
        let trigger_inserted = if let Some(typed) = events.typed.get()
            && update.changes.changed_ranges().any(|(_, inserted)| {
                inserted.start < head
                    && head <= inserted.end
                    && head - inserted.start >= typed.len_utf8()
                    && update
                        .state
                        .text()
                        .slice(head - typed.len_utf8()..head)
                        .is_some_and(|text| text.ends_with(typed))
            }) {
            events.inserted.set(true);
            true
        } else {
            false
        };
        if invalidated {
            events.close();
            return;
        }
        if !update.doc_changed() && !update.selection_set {
            return;
        }
        let mut shown = events.shown.borrow_mut();
        let Some(open) = shown.as_mut() else {
            return;
        };
        let same_fence = open.fence.as_mut().is_some_and(|fence| {
            fence.map(update.changes, &mut open.anchor)
                && fence.contains(head)
                && fence.contains(open.anchor)
        });
        if !same_fence || !matches!(update.state.selection().ranges(), [range] if range.is_empty())
        {
            *shown = None;
            events.delayed.take();
        } else if trigger_inserted {
            events.delayed.take();
        } else if update.selection_set {
            // A document change alone maps the tooltip without starting or
            // resetting the timer. Typing can also set the selection.
            events.delayed.set(Some(self.timer.start()));
        }
    }

    fn destroy(&mut self) {
        self.events.close();
    }
}

#[derive(Clone, Eq, PartialEq)]
struct Fence {
    source: Range<usize>,
    body: Range<usize>,
}

impl Fence {
    fn at(editor: &Editor, offset: usize) -> Option<Self> {
        let fence = Self {
            source: editor.active.as_ref()?.source.clone(),
            body: editor.source_fence()?.body,
        };
        fence.contains(offset).then_some(fence)
    }

    fn contains(&self, offset: usize) -> bool {
        (self.body.start..=self.body.end).contains(&offset)
    }

    fn map(&mut self, changes: &ChangeDesc, anchor: &mut usize) -> bool {
        if !changes
            .changed_ranges()
            .all(|(old, _)| self.body.start <= old.start && old.end <= self.body.end)
        {
            return false;
        }
        let end = |pos| changes.map_pos(pos, Assoc::After, MapMode::Simple);
        let (Some(mapped), Some(body_end), Some(source_end)) = (
            changes.map_pos(*anchor, Assoc::Before, MapMode::TrackDel),
            end(self.body.end),
            end(self.source.end),
        ) else {
            return false;
        };
        self.body.end = body_end;
        self.source.end = source_end;
        *anchor = mapped;
        true
    }
}

// Requests require an exact revision and selection. Only the open tooltip maps.
#[derive(Eq, PartialEq)]
struct Stamp {
    revision: u64,
    selection_epoch: u64,
    offset: usize,
    fence: Option<Fence>,
}

struct Shown {
    // Equal signature labels can keep this anchor while the caret moves.
    anchor: usize,
    fence: Option<Fence>,
    plan: SignaturePlan,
}

struct Target {
    stamp: Stamp,
    trigger: SignatureTrigger,
    active: Option<SignaturePlan>,
}

pub(super) struct SignatureHelp {
    events: Rc<Events>,
    plugin: PluginId,
    wanted: Option<Target>,
    pending: Option<Pending<Target, SignatureRequest>>,
}

impl Editor {
    /// Queue signature help at the focused editor's single caret. Poll the
    /// attachment to send the request and notify a live editor after polling.
    pub fn request_source_lsp_signature_help(&mut self) -> bool {
        self.queue_signature_help(SignatureTrigger::Invoked, None)
    }

    fn queue_signature_help(
        &mut self,
        trigger: SignatureTrigger,
        prior: Option<SignaturePlan>,
    ) -> bool {
        let Some(lsp) = &self.source_lsp else {
            return false;
        };
        if lsp.client.server_capabilities().is_some_and(|caps| {
            !matches!(
                caps.get("signatureHelpProvider"),
                Some(Value::Bool(true) | Value::Object(_))
            )
        }) {
            return false;
        }
        let Some(stamp) = lsp.signature.stamp(self) else {
            return false;
        };
        let active = prior.or_else(|| {
            lsp.signature
                .current(self, &stamp)
                .map(|open| open.plan.clone())
        });
        if trigger == SignatureTrigger::ContentChange && (active.is_none() || stamp.fence.is_none())
        {
            return false;
        }
        let signature = &mut self.source_lsp.as_mut().expect("attached").signature;
        signature.queue(Target {
            stamp,
            trigger,
            active,
        });
        true
    }

    fn retrigger_source_lsp_signature(&mut self, window: &Window) {
        if !window.is_window_active()
            || !self.focus.is_focused(window)
            || !self.queue_signature_help(SignatureTrigger::ContentChange, None)
        {
            self.dismiss_source_lsp_signature();
        }
    }

    fn signature_scope(&self, offset: usize) -> bool {
        ReferenceScope::new(self).allows(&(offset..offset))
            && !self
                .source_fold_ranges()
                .iter()
                .any(|fold| fold.start < offset && offset < fold.end)
            && self
                .layout
                .as_ref()
                .is_none_or(|layout| !layout.fold_hides(offset))
    }

    pub(in crate::editor) fn type_with_source_lsp_signature(
        &mut self,
        text: &str,
        window: &Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Self, &mut Context<Self>) -> bool,
    ) {
        let capture = self.source_lsp.as_ref().and_then(|lsp| {
            let mut chars = text.chars();
            let typed = chars.next()?;
            if chars.next().is_some()
                || !window.is_window_active()
                || !self.focus.is_focused(window)
            {
                return None;
            }
            let stamp = lsp.signature.stamp(self)?;
            let active = self.layout.as_ref().and_then(|layout| {
                let open = lsp.signature.current(self, &stamp)?;
                signature_anchor(layout, self.source(), open.anchor, self.scroll.bounds())?;
                Some(open.plan.clone())
            });
            let trigger = SignatureTrigger::automatic(
                [text],
                active.is_some(),
                lsp.client.server_capabilities()?,
            )?;
            let events = lsp.signature.events.clone();
            events.typed.set(Some(typed));
            Some((events, trigger, active))
        });
        self.edit_with_source_completion("input.type", edit, cx);
        if let Some((events, trigger, active)) = capture {
            events.typed.set(None);
            if events.inserted.replace(false) {
                self.queue_signature_help(trigger, active);
            }
        }
    }

    pub(in crate::editor) fn dismiss_source_lsp_tooltips(&mut self) -> bool {
        self.dismiss_source_lsp_hover() | self.dismiss_source_lsp_signature()
    }

    pub(in crate::editor) fn dismiss_source_lsp_signature(&mut self) -> bool {
        self.source_lsp
            .as_mut()
            .is_some_and(|lsp| lsp.signature.dismiss())
    }

    pub(in crate::editor) fn cycle_source_lsp_signature(
        &mut self,
        forward: bool,
        window: &Window,
    ) -> bool {
        let Some(layout) = &self.layout else {
            return false;
        };
        if !window.is_window_active()
            || !self.focus.is_focused(window)
            || self
                .source_lsp_signature_tooltip(layout, self.scroll.bounds(), None)
                .is_none()
        {
            return false;
        }
        let mut shown = self
            .source_lsp
            .as_ref()
            .unwrap()
            .signature
            .events
            .shown
            .borrow_mut();
        let plan = &mut shown.as_mut().unwrap().plan;
        *plan = if forward {
            plan.next_signature()
        } else {
            plan.prev_signature()
        };
        true
    }

    pub(super) fn source_lsp_signature_tooltip(
        &self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        caret: Option<Bounds<Pixels>>,
    ) -> Option<Overlay> {
        let signature = &self.source_lsp.as_ref()?.signature;
        signature.events.shown.borrow().as_ref()?;
        let stamp = signature.stamp(self)?;
        let shown = signature.current(self, &stamp)?;
        let anchor = signature_anchor(layout, self.source(), shown.anchor, viewport)?;
        Some(Overlay {
            text: signature_text(&shown.plan)?,
            anchor: Some(anchor),
            viewport,
            caret,
            prefer_above: false,
        })
    }
}

impl SignatureHelp {
    pub(super) fn attach(view: &mut EditorView<Option<Section>>, timer: SignatureTimer) -> Self {
        let events = Rc::new(Events::default());
        let plugin = view.add_plugin(EventTracker {
            events: events.clone(),
            timer,
        });
        Self {
            events,
            plugin,
            wanted: None,
            pending: None,
        }
    }

    pub(super) fn plugin(&self) -> PluginId {
        self.plugin
    }

    fn stamp(&self, editor: &Editor) -> Option<Stamp> {
        let selection = editor.view.state().selection();
        let [range] = selection.ranges() else {
            return None;
        };
        let offset = range.head();
        (range.is_empty()
            && editor.reference_input_allowed()
            && editor.view.focused()
            && !editor.selecting
            && !editor.is_reviewing_changes()
            && editor.signature_scope(offset))
        .then(|| Stamp {
            revision: editor.view.revision(),
            selection_epoch: self.events.selection_epoch.get(),
            offset,
            fence: Fence::at(editor, offset),
        })
    }

    fn current(&self, editor: &Editor, stamp: &Stamp) -> Option<Ref<'_, Shown>> {
        Ref::filter_map(self.events.shown.borrow(), |shown| {
            shown.as_ref().filter(|open| {
                open.fence == stamp.fence
                    && (open.anchor == stamp.offset
                        || open.fence.is_some() && editor.signature_scope(open.anchor))
            })
        })
        .ok()
    }

    fn queue(&mut self, target: Target) {
        self.events.delayed.take();
        if let Some(pending) = &mut self.pending {
            pending.target = None;
        }
        self.wanted = Some(target);
    }

    pub(super) fn dismiss(&mut self) -> bool {
        self.wanted.take().is_some()
            | self.events.close()
            | self
                .pending
                .as_mut()
                .and_then(|pending| pending.target.take())
                .is_some()
    }

    pub(super) fn retain_current(&mut self, editor: &Editor) {
        if self.wanted.is_none() && self.pending.is_none() && self.events.shown.borrow().is_none() {
            return;
        }
        let current = self.stamp(editor);
        let open = current
            .as_ref()
            .and_then(|stamp| self.current(editor, stamp))
            .is_some();
        if !open {
            self.events.close();
        }
        let stale = |target: &mut Target| {
            Some(&target.stamp) != current.as_ref()
                || target.trigger == SignatureTrigger::ContentChange && !open
        };
        self.wanted.take_if(stale);
        if let Some(pending) = &mut self.pending {
            pending.target.take_if(stale);
        }
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.wanted.is_none() && self.pending.is_none()
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
                signature_request(
                    workspace,
                    uri,
                    target.stamp.offset,
                    target.trigger.clone(),
                    target.active.as_ref(),
                )
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
        let Some(pending) = self.pending.take() else {
            return;
        };
        let Some(target) = pending.target.filter(|_| !reply.cancel_requested) else {
            return;
        };
        let Ok(result) = reply.outcome else {
            return;
        };
        if !response_fits(&result) {
            return;
        }
        let update = client.with_workspace(|workspace| {
            let file = workspace.file(uri)?;
            if file.version() != pending.request.version() || file.has_pending_changes() {
                return None;
            }
            let current = self.current(editor, &target.stamp);
            match pending
                .request
                .resolve(workspace, &result, current.as_ref().map(|open| &open.plan))
                .ok()?
            {
                SignatureEffect::Show(plan) => {
                    let anchor = plan.anchor(workspace).ok()?;
                    (plan.uri() == uri
                        && (anchor == target.stamp.offset
                            || current.as_ref().is_some_and(|open| anchor == open.anchor)))
                    .then_some(Some(Shown {
                        anchor,
                        fence: target.stamp.fence,
                        plan,
                    }))
                }
                SignatureEffect::Clear => Some(None),
                SignatureEffect::Keep => None,
            }
        });
        match update {
            Some(Some(shown)) => *self.events.shown.borrow_mut() = Some(shown),
            Some(None) => {
                self.events.close();
            }
            None => {}
        }
    }
}

fn signature_anchor(
    layout: &Layout,
    source: &str,
    offset: usize,
    viewport: Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
    hover_anchor(layout, source, &(offset..offset), offset, viewport)
}

fn signature_text(plan: &SignaturePlan) -> Option<String> {
    let signature = plan.signature();
    let mut text = signature.label.clone();
    if plan.signatures().len() > 1 {
        text = format!(
            "{}/{}  {text}",
            plan.active_index() + 1,
            plan.signatures().len()
        );
    }
    if let Some(span) = plan.highlight() {
        text.push_str("\nParameter: ");
        text.push_str(&signature.label[span]);
    }
    if let Some(docs) = &signature.documentation {
        let docs = match docs {
            Documentation::PlainText(text)
            | Documentation::Markdown(text)
            | Documentation::Code { value: text, .. } => text,
        };
        text.push('\n');
        text.push_str(docs);
    }
    display_text(&text)
}

fn response_fits(response: &Value) -> bool {
    if response.is_null() {
        return true;
    }
    response
        .get("signatures")
        .and_then(Value::as_array)
        .is_some_and(|signatures| {
            signatures.len() <= MAX_SIGNATURES
                && signatures.iter().all(|signature| {
                    signature
                        .get("parameters")
                        .and_then(Value::as_array)
                        .is_none_or(|parameters| parameters.len() <= MAX_PARAMETERS)
                })
        })
        && serde_json::to_writer(ResponseBudget(MAX_RESPONSE_BYTES), response).is_ok()
}

// Bound the wire data retained by the core plan, including fields not displayed.
struct ResponseBudget(usize);

impl io::Write for ResponseBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or(io::ErrorKind::WriteZero)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn rejects_overlarge_signature_help_before_resolving() {
    use serde_json::json;

    assert!(response_fits(&Value::Null));
    assert!(response_fits(&json!({"signatures": []})));
    assert!(!response_fits(
        &json!({"signatures": vec![json!({"label": "f(x)"}); MAX_SIGNATURES + 1]})
    ));
    assert!(!response_fits(&json!({"signatures": [{
        "label": "f(x)", "parameters": vec![json!({"label": [2, 3]}); MAX_PARAMETERS + 1]
    }]})));
    let large =
        json!({"signatures": [{"label": "f(x)", "documentation": "x".repeat(MAX_RESPONSE_BYTES)}]});
    assert!(!response_fits(&large));
}
