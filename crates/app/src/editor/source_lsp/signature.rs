//! Read-only signature help for the attached draft.

use std::{cell::Cell, io, rc::Rc};

use gpui::{App, Bounds, Context, KeyBinding, Pixels, Window, actions};
use gpui_editor_codemirror::{
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

const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_SIGNATURES: usize = 16;
const MAX_PARAMETERS: usize = 32;

#[derive(Default)]
struct Events {
    selection_epoch: Cell<u64>,
    typed: Cell<Option<char>>,
    inserted: Cell<bool>,
}

struct EventTracker(Rc<Events>);

impl ViewPlugin for EventTracker {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let events = &self.0;
        if update.selection_set || update.focus_changed {
            events
                .selection_epoch
                .set(events.selection_epoch.get().wrapping_add(1));
        }
        // Use the accepted changes, not only the text the input handler saw.
        // The caret can be inside a paired insertion such as `()`.
        let head = update.state.selection().main().head();
        if let Some(typed) = events.typed.get()
            && update.changes.changed_ranges().any(|(_, inserted)| {
                inserted.start < head
                    && head <= inserted.end
                    && head - inserted.start >= typed.len_utf8()
                    && update
                        .state
                        .text()
                        .slice(head - typed.len_utf8()..head)
                        .is_some_and(|text| text.ends_with(typed))
            })
        {
            events.inserted.set(true);
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Stamp {
    revision: u64,
    selection_epoch: u64,
    offset: usize,
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
    shown: Option<(Stamp, SignaturePlan)>,
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
        let Some(stamp) = lsp
            .signature
            .stamp(self)
            .filter(|stamp| self.signature_scope(stamp.offset))
        else {
            return false;
        };
        let active = prior.or_else(|| lsp.signature.current(stamp).cloned());
        let signature = &mut self.source_lsp.as_mut().expect("attached").signature;
        signature.queue(Target {
            stamp,
            trigger,
            active,
        });
        true
    }

    fn signature_scope(&self, offset: usize) -> bool {
        ReferenceScope::new(self).allows(&(offset..offset))
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
                signature_anchor(layout, self.source(), stamp.offset, self.scroll.bounds())?;
                lsp.signature.current(stamp).cloned()
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
        self.dismiss_source_lsp_hover()
            | self
                .source_lsp
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
        let (_, plan) = self
            .source_lsp
            .as_mut()
            .unwrap()
            .signature
            .shown
            .as_mut()
            .unwrap();
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
        let stamp = signature.stamp(self)?;
        let plan = signature.current(stamp)?;
        let anchor = signature_anchor(layout, self.source(), stamp.offset, viewport)?;
        if !self.signature_scope(stamp.offset) {
            return None;
        }
        Some(Overlay {
            text: signature_text(plan)?,
            anchor: Some(anchor),
            viewport,
            caret,
            prefer_above: false,
        })
    }
}

impl SignatureHelp {
    pub(super) fn attach(view: &mut EditorView<Option<Section>>) -> Self {
        let events = Rc::new(Events::default());
        let plugin = view.add_plugin(EventTracker(events.clone()));
        Self {
            events,
            plugin,
            wanted: None,
            pending: None,
            shown: None,
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
        (range.is_empty()
            && editor.reference_input_allowed()
            && editor.view.focused()
            && !editor.selecting)
            .then(|| Stamp {
                revision: editor.view.revision(),
                selection_epoch: self.events.selection_epoch.get(),
                offset: range.head(),
            })
    }

    fn current(&self, stamp: Stamp) -> Option<&SignaturePlan> {
        let (at, plan) = self.shown.as_ref()?;
        (*at == stamp).then_some(plan)
    }

    fn queue(&mut self, target: Target) {
        if let Some(pending) = &mut self.pending {
            pending.target = None;
        }
        self.wanted = Some(target);
    }

    pub(super) fn dismiss(&mut self) -> bool {
        self.wanted.take().is_some()
            | self.shown.take().is_some()
            | self
                .pending
                .as_mut()
                .and_then(|pending| pending.target.take())
                .is_some()
    }

    pub(super) fn retain_current(&mut self, editor: &Editor) {
        let current = self.stamp(editor);
        self.wanted.take_if(|target| Some(target.stamp) != current);
        if let Some(pending) = &mut self.pending {
            pending
                .target
                .take_if(|target| Some(target.stamp) != current);
        }
        self.shown.take_if(|(stamp, _)| Some(*stamp) != current);
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

    pub(super) fn resolve(&mut self, client: &StdioClient, uri: &str, reply: RpcResolution) {
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
        let effect = client.with_workspace(|workspace| {
            let file = workspace.file(uri)?;
            if file.version() != pending.request.version() || file.has_pending_changes() {
                return None;
            }
            match pending
                .request
                .resolve(workspace, &result, self.current(target.stamp))
                .ok()?
            {
                SignatureEffect::Show(plan)
                    if plan.uri() != uri || plan.anchor(workspace).ok()? != target.stamp.offset =>
                {
                    None
                }
                effect => Some(effect),
            }
        });
        match effect {
            Some(SignatureEffect::Show(plan)) => self.shown = Some((target.stamp, plan)),
            Some(SignatureEffect::Clear) => self.shown = None,
            Some(SignatureEffect::Keep) | None => {}
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
