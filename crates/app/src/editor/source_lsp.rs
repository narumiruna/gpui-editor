//! Opt-in LSP for one full editor document and one trusted stdio client.

use std::{collections::VecDeque, fmt, ops::Range, time::Duration};

use gpui::{Bounds, Pixels, Window, point, px, size};
use gpui_editor_codemirror::{
    lsp::{
        DocumentWorkspace, SyncKind,
        features::{Documentation, FeatureError, FeatureRequest, Hover, hover_request},
        rpc::{LspState, RequestId, ResponseError, RpcId, RpcMessage, RpcResolution},
        transport::{
            PreparedRequest, StdioClient, StdioClientError, TransportError, TransportEvent,
        },
    },
    view::PluginId,
};
use serde_json::Value;

use super::{Editor, ItemPaint, Layout, Overlay, SectionKind};

mod completion;
mod diagnostics;
mod formatting;
mod navigation;
mod reference_panel_ui;
mod references;
mod rename;
mod signature;

use completion::Completion;
pub use completion::{SourceLspCompletion, SourceLspCompletionItem};
use diagnostics::Diagnostics;
use formatting::Formatting;
pub use formatting::{FormattingOptions, SourceLspFormatting};
use navigation::Navigation;
pub use navigation::{NavigationKind, SourceLspNavigation};
pub(super) use reference_panel_ui::ReferencePanelUi;
use references::References;
pub use references::{ReferenceResults, ReferenceRow};
use rename::Rename;
pub use rename::SourceLspRename;
use signature::SignatureHelp;
pub(super) use signature::{NextSignature, PreviousSignature, ShowSignatureHelp, bind_keys};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_STEPS: usize = 8;
const MAX_PEER_REPLIES: usize = 8;
const MAX_PEER_ID_BYTES: usize = 256;
const MAX_HOVER_BYTES: usize = 16 * 1024;
const MAX_HOVER_ITEMS: usize = 32;
const MAX_DISPLAY_CHARS: usize = 512;
const MAX_DISPLAY_LINES: usize = 8;

/// An attachment could not be installed.
#[derive(Debug)]
pub enum AttachError {
    AlreadyAttached,
    ClientInUse,
    Client(StdioClientError),
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyAttached => f.write_str("the editor already has an LSP attachment"),
            Self::ClientInUse => f.write_str("the LSP client already has an open document"),
            Self::Client(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AttachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            _ => None,
        }
    }
}

/// The state after one host poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Detached,
    Initializing,
    Ready,
    /// This poll removed a failed or disconnected attachment.
    Disconnected,
}

/// Untrusted server text and its UTF-8 byte range in the current document.
/// Display `text` only as literal text. Do not interpret markup or activate links.
#[derive(Debug, Eq, PartialEq)]
pub struct HoverText {
    pub range: Range<usize>,
    pub text: String,
}

pub(super) struct SourceLsp {
    client: StdioClient,
    plugin: PluginId,
    uri: String,
    open_revision: u64,
    peer_replies: VecDeque<RpcId>,
    wanted: Option<Target>,
    pending: Option<Pending<Target, FeatureRequest<Hover>>>,
    result: Option<HoverResult>,
    references: References,
    signature: SignatureHelp,
    formatting: Formatting,
    navigation: Navigation,
    rename: Rename,
    completion: Completion,
    diagnostics: Diagnostics,
}

#[derive(Clone, Copy)]
struct Target {
    revision: u64,
    offset: usize,
    for_display: bool,
}

struct Pending<T, R> {
    target: Option<T>,
    id: RequestId,
    request: R,
}

struct HoverResult {
    target: Target,
    hover: HoverText,
    displayable: bool,
}

impl Editor {
    /// Attach an already-started, dedicated, trusted client to the entire draft.
    /// The caller supplies the URI and language ID; neither is used to open a file.
    /// This transfers client ownership, including on error. Poll on the owning
    /// thread to advance initialization, synchronization, and requests.
    pub fn attach_source_lsp(
        &mut self,
        mut client: StdioClient,
        uri: &str,
        language_id: &str,
    ) -> Result<(), AttachError> {
        if self.source_lsp.is_some() {
            return Err(AttachError::AlreadyAttached);
        }
        if !client.with_workspace(|workspace| workspace.files().is_empty()) {
            return Err(AttachError::ClientInUse);
        }
        let plugin = client
            .attach(&mut self.view, uri, language_id)
            .map_err(AttachError::Client)?;
        let open_revision = self.view.revision();
        let open_version = client
            .with_workspace(|workspace| workspace.file(uri).expect("attached document").version());
        self.source_lsp = Some(SourceLsp {
            client,
            plugin,
            uri: uri.into(),
            open_revision,
            peer_replies: VecDeque::new(),
            wanted: None,
            pending: None,
            result: None,
            references: References::default(),
            signature: SignatureHelp::attach(&mut self.view),
            formatting: Formatting::attach(&mut self.view),
            navigation: Navigation::attach(&mut self.view),
            rename: Rename::attach(&mut self.view),
            completion: Completion::attach(&mut self.view),
            diagnostics: Diagnostics::new(open_revision, open_version),
        });
        Ok(())
    }

    /// Drive a bounded number of nonblocking I/O steps, including when idle.
    /// No GPUI timer is installed. Call `cx.notify()` after polling a live editor
    /// entity to update its display. On terminal failure the attachment is removed;
    /// the native transport may kill and reap its direct child during cleanup.
    pub fn poll_source_lsp(&mut self) -> Status {
        let Some(mut lsp) = self.source_lsp.take() else {
            return Status::Detached;
        };
        let status = lsp.poll(self);
        self.source_lsp = Some(lsp);
        if status == Status::Disconnected {
            drop(self.detach_source_lsp());
        }
        status
    }

    /// Queue hover at an explicit byte offset. A valid new target supersedes
    /// the previous one, even during initialization. The host must poll to send
    /// it; a queued target does not guarantee a result. Call `cx.notify()` after
    /// a successful request to clear the previous tooltip in a live editor.
    /// Requests made without editor focus or during composition are data-only.
    pub fn request_source_lsp_hover(&mut self, offset: usize) -> bool {
        let source = self.source().as_bytes();
        if !self.source().is_char_boundary(offset)
            || (source.get(offset) == Some(&b'\n') && offset > 0 && source[offset - 1] == b'\r')
        {
            return false;
        }
        let for_display = self.input_enabled && self.view.focused() && !self.is_composing();
        let Some(lsp) = &mut self.source_lsp else {
            return false;
        };
        lsp.wanted = Some(Target {
            revision: self.view.revision(),
            offset,
            for_display,
        });
        lsp.result = None;
        true
    }

    /// Read the latest result, if its document revision is still current.
    pub fn source_lsp_hover(&self) -> Option<&HoverText> {
        Some(&self.current_source_lsp_hover()?.hover)
    }

    fn current_source_lsp_hover(&self) -> Option<&HoverResult> {
        let result = self.source_lsp.as_ref()?.result.as_ref()?;
        (result.target.revision == self.view.revision()).then_some(result)
    }

    /// Clear the hover and retire any queued or in-flight hover. Polling still
    /// drains the transport. Call `cx.notify()` when this returns true.
    pub fn dismiss_source_lsp_hover(&mut self) -> bool {
        let Some(lsp) = &mut self.source_lsp else {
            return false;
        };
        lsp.wanted.take().is_some()
            | lsp.result.take().is_some()
            | lsp.pending.as_mut().and_then(|p| p.target.take()).is_some()
    }

    pub(super) fn source_lsp_tooltip(
        &self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        window: &Window,
        caret: Option<Bounds<Pixels>>,
    ) -> Option<Overlay> {
        if !self.input_enabled
            || self.view.state().read_only()
            || self.selecting
            || self.is_composing()
            || !self.focus.is_focused(window)
            || !window.is_window_active()
        {
            return None;
        }
        if let Some(signature) = self.source_lsp_signature_tooltip(layout, viewport, caret) {
            return Some(signature);
        }
        let result = self.current_source_lsp_hover().filter(|r| r.displayable)?;
        let range = &result.hover.range;
        if let Some(active) = &self.active
            && range.start < active.source.end
            && (range.end > active.source.start || range.start == active.source.start)
        {
            let content = if active.kind == SectionKind::DraftCode {
                self.source_fence()?.body
            } else {
                active.content.clone()
            };
            if range.start < content.start || range.end > content.end {
                return None;
            }
        }
        let anchor = hover_anchor(layout, self.source(), range, result.target.offset, viewport)?;
        let text = display_text(&result.hover.text)?;
        Some(Overlay {
            text,
            anchor: Some(anchor),
            viewport,
            caret,
            prefer_above: false,
        })
    }

    /// Remove the plugin and return the client. Call `cx.notify()` to remove
    /// its tooltip from a live editor. A ready attachment queues a close.
    /// Edits not yet captured for sync are discarded by the plugin.
    /// This does not wait for the server. The caller can drive the client or call
    /// `StdioClient::shutdown` with a finite timeout after readiness, outside
    /// responsive UI callbacks. Drop terminates the direct child instead.
    pub fn detach_source_lsp(&mut self) -> Option<StdioClient> {
        self.dismiss_source_lsp_references();
        let lsp = self.source_lsp.take()?;
        self.view.remove_plugin(lsp.plugin);
        self.view.remove_plugin(lsp.signature.plugin());
        self.view.remove_plugin(lsp.formatting.plugin());
        self.view.remove_plugin(lsp.navigation.plugin());
        self.view.remove_plugin(lsp.rename.plugin());
        self.view.remove_plugin(lsp.completion.plugin());
        Some(lsp.client)
    }
}

impl SourceLsp {
    fn poll(&mut self, editor: &mut Editor) -> Status {
        let revision = editor.view.revision();
        if self.references.panel_is_stale(revision) {
            self.references.close(editor);
        }
        self.signature.retain_current(editor);
        self.formatting.retain_current(editor);
        self.navigation.retain_current(editor);
        self.rename.retain_current(editor);
        self.completion.retain_current(editor);
        let diagnostic_scope = self
            .diagnostics
            .enabled()
            .then(|| editor.source_lsp_diagnostic_scope())
            .flatten();
        self.diagnostics.retain(diagnostic_scope.as_ref());
        for _ in 0..POLL_STEPS {
            if !self.try_peer_reply() {
                return Status::Disconnected;
            }
            match self.client.next_event(Duration::ZERO) {
                Ok(Some(TransportEvent::Message(RpcMessage::Request { id, .. }))) => {
                    if matches!(&id, RpcId::String(id) if id.len() > MAX_PEER_ID_BYTES)
                        || self.peer_replies.len() == MAX_PEER_REPLIES
                    {
                        return Status::Disconnected;
                    }
                    self.peer_replies.push_back(id);
                }
                Ok(Some(TransportEvent::Message(message @ RpcMessage::Notification { .. }))) => {
                    if let Some(scope) = &diagnostic_scope {
                        self.diagnostics
                            .receive(&self.client, &self.uri, scope, &message);
                    }
                }
                Ok(Some(TransportEvent::Resolution(reply))) => {
                    if self.references.is_pending(reply.id) {
                        self.references
                            .resolve(&self.client, &self.uri, editor, reply);
                    } else if self.signature.is_pending(reply.id) {
                        self.signature.resolve(&self.client, &self.uri, reply);
                    } else if self.formatting.is_pending(reply.id) {
                        self.formatting
                            .resolve(&self.client, &self.uri, editor, reply);
                    } else if self.navigation.is_pending(reply.id) {
                        self.navigation
                            .resolve(&self.client, &self.uri, editor, reply);
                    } else if self.rename.is_pending(reply.id) {
                        self.rename.resolve(&self.client, &self.uri, editor, reply);
                    } else if self.completion.is_pending(reply.id) {
                        self.completion
                            .resolve(&self.client, &self.uri, editor, reply);
                    } else {
                        self.resolve_hover(reply, revision, |range| {
                            // Check only on receipt. The raw result is still
                            // available to the host for the whole draft.
                            editor
                                .markdown()
                                .plan_replacement(range, "", editor.active.as_ref())
                                .is_ok()
                        });
                    }
                }
                Ok(Some(TransportEvent::TimedOut(id))) => {
                    if self
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.id == id)
                    {
                        self.pending = None;
                    }
                    self.references.timed_out(id);
                    self.signature.timed_out(id);
                    self.formatting.timed_out(id);
                    self.navigation.timed_out(id);
                    self.rename.timed_out(id);
                    self.completion.timed_out(id);
                }
                Err(_) | Ok(Some(TransportEvent::Disconnected { .. })) => {
                    return Status::Disconnected;
                }
                Ok(None) => break,
                Ok(Some(_)) => {}
            }
        }

        if !self
            .client
            .with_workspace(|workspace| workspace.file(&self.uri).is_some())
        {
            return Status::Disconnected;
        }
        match self.client.state() {
            LspState::Initializing(_) => Status::Initializing,
            LspState::Ready => {
                let can_sync = self.can_sync(revision);
                if !self.sync_and_request(revision, can_sync, editor.reference_input_allowed()) {
                    return Status::Disconnected;
                }
                if can_sync {
                    self.diagnostics
                        .observe_sync(&self.client, &self.uri, editor);
                }
                Status::Ready
            }
            _ => Status::Disconnected,
        }
    }

    fn try_peer_reply(&mut self) -> bool {
        let Some(id) = self.peer_replies.front() else {
            return true;
        };
        // Do not use the method or params, even for familiar requests.
        let unsupported = ResponseError {
            code: -32601,
            message: "Method not implemented".into(),
            data: None,
        };
        match self.client.reply(id.clone(), Err(unsupported)) {
            Ok(()) => {
                self.peer_replies.pop_front();
                true
            }
            Err(TransportError::QueueFull) => true,
            Err(_) => false,
        }
    }

    fn can_sync(&self, revision: u64) -> bool {
        // Without change sync, the server only has the opening snapshot.
        revision == self.open_revision
            || self
                .client
                .server_capabilities()
                .and_then(|caps| SyncKind::from_server_capabilities(caps).ok())
                .is_some_and(|kind| kind != SyncKind::None)
    }

    fn sync_and_request(&mut self, revision: u64, synced: bool, references_allowed: bool) -> bool {
        self.wanted = self
            .wanted
            .filter(|target| synced && target.revision == revision);
        self.references
            .retain_wanted(synced && references_allowed, revision);
        if !synced {
            self.signature.dismiss();
            self.formatting.discard_wanted();
            self.navigation.discard_wanted();
            self.rename.discard_wanted();
            self.completion.discard_wanted();
        }
        if (self.wanted.is_none() || self.pending.is_some())
            && !self.references.can_send()
            && !self.signature.can_send()
            && !self.formatting.can_send()
            && !self.navigation.can_send()
            && !self.rename.can_send()
            && !self.completion.can_send()
        {
            return matches!(
                self.client.try_flush(),
                Ok(()) | Err(StdioClientError::Transport(TransportError::QueueFull))
            );
        }
        self.references.send(&mut self.client, &self.uri)
            && self.signature.send(&mut self.client, &self.uri)
            && self.formatting.send(&mut self.client, &self.uri)
            && self.navigation.send(&mut self.client, &self.uri)
            && self.rename.send(&mut self.client, &self.uri)
            && self.completion.send(&mut self.client, &self.uri)
            && send_request(
                &mut self.client,
                &mut self.wanted,
                &mut self.pending,
                |workspace, target| hover_request(workspace, &self.uri, target.offset),
            )
    }

    fn resolve_hover(
        &mut self,
        reply: RpcResolution,
        revision: u64,
        can_display: impl FnOnce(Range<usize>) -> bool,
    ) {
        let Some(pending) = self.pending.take_if(|pending| pending.id == reply.id) else {
            return;
        };
        let Some(target) = pending.target else {
            return;
        };
        if target.revision != revision || self.wanted.is_some() || reply.cancel_requested {
            return;
        }
        let Ok(result) = reply.outcome else {
            return;
        };
        if !hover_fits(&result) {
            return;
        }
        let hover = self
            .client
            .with_workspace(|workspace| pending.request.resolve(workspace, &result));
        if let Ok(Some(hover)) = hover
            && (hover.range.start..=hover.range.end).contains(&target.offset)
        {
            let text = hover
                .contents
                .into_iter()
                .map(|content| match content {
                    Documentation::PlainText(text)
                    | Documentation::Markdown(text)
                    | Documentation::Code { value: text, .. } => text,
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.result = Some(HoverResult {
                target,
                displayable: target.for_display && can_display(hover.range.clone()),
                hover: HoverText {
                    range: hover.range,
                    text,
                },
            });
        }
    }
}

// try_request flushes accepted edits first and retains their order if the
// output queue fills. Each feature has one queued target and one wire request.
fn send_request<T, R: PreparedRequest>(
    client: &mut StdioClient,
    wanted: &mut Option<T>,
    pending: &mut Option<Pending<T, R>>,
    prepare: impl FnOnce(&DocumentWorkspace, &T) -> Result<R, FeatureError>,
) -> bool {
    let Some(target) = wanted.as_ref().filter(|_| pending.is_none()) else {
        return true;
    };
    match client.try_request(REQUEST_TIMEOUT, |workspace| prepare(workspace, target)) {
        Ok((id, request)) => {
            *pending = Some(Pending {
                target: wanted.take(),
                id,
                request,
            });
        }
        Err(StdioClientError::Transport(TransportError::QueueFull)) => {}
        Err(StdioClientError::UnsupportedCapability(_) | StdioClientError::Feature(_)) => {
            *wanted = None;
        }
        Err(_) => return false,
    }
    true
}

fn hover_anchor(
    layout: &Layout,
    source: &str,
    range: &Range<usize>,
    offset: usize,
    viewport: Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
    // At the end of a non-empty range, use the preceding glyph's row rather
    // than the next row at a wrap boundary.
    let offset = if !range.is_empty() && offset == range.end {
        offset
            - source
                .get(range.start..offset)?
                .chars()
                .next_back()?
                .len_utf8()
    } else {
        offset
    };
    let position = layout.bounds_at(offset, false)?;
    if !viewport.intersects(&position) {
        return None;
    }
    let row = layout.row_at(position.center());
    let items = row.items.iter().filter(|item| match &item.paint {
        ItemPaint::Text(line) => {
            item.target.is_none() && source.get(item.source.clone()) == Some(line.text.as_ref())
        }
        #[cfg(target_os = "macos")]
        ItemPaint::Isolated(_) => true,
        _ => false,
    });
    if range.is_empty() {
        // No range in the response: the request point must belong to a real
        // text item, not to a preview, fold placeholder, or hidden delimiter.
        return items
            .into_iter()
            .any(|item| item.source.start <= offset && offset <= item.source.end)
            .then_some(position);
    }

    // Do not project a range across unpainted source or through another row.
    // For bidi text, anchor to the visible span at the request, not its
    // logical start on the far side of the row.
    let mut covered = range.start;
    let mut anchor = None;
    for item in items {
        if item.source.end <= covered {
            continue;
        }
        if item.source.start > covered {
            return None;
        }
        if item.source.contains(&offset) {
            anchor = item.x_spans(range.clone()).find_map(|span| {
                let bounds = Bounds::new(
                    layout.origin + point(span.start, row.y),
                    size(span.end - span.start, row.height),
                );
                (span.start < span.end
                    && viewport.intersects(&bounds)
                    && bounds.inset(px(-1.)).contains(&position.center()))
                .then_some(bounds)
            });
        }
        covered = item.source.end;
        if covered >= range.end {
            return anchor;
        }
    }
    None
}

fn display_text(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    let mut preview = String::new();
    let mut lines = 1;
    for (index, ch) in text.chars().enumerate() {
        if index == MAX_DISPLAY_CHARS || (ch == '\n' && lines == MAX_DISPLAY_LINES) {
            preview.push('…');
            break;
        }
        preview.push(match ch {
            '\n' => {
                lines += 1;
                '\n'
            }
            '\t' => ' ',
            // Do not pass terminal controls or explicit direction controls
            // from the peer into the text shaper.
            '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'
            | '\u{2029}' => '�',
            ch if ch.is_control() => '�',
            ch => ch,
        });
    }
    Some(preview)
}

fn hover_fits(result: &Value) -> bool {
    let Some(contents) = result.get("contents") else {
        return false;
    };
    let items = contents
        .as_array()
        .map_or_else(|| std::slice::from_ref(contents), Vec::as_slice);
    if !(1..=MAX_HOVER_ITEMS).contains(&items.len()) {
        return false;
    }
    let mut remaining = MAX_HOVER_BYTES - (items.len() - 1);
    items.iter().all(|item| {
        let text = item
            .as_str()
            .or_else(|| item.get("value").and_then(Value::as_str));
        let language_bytes = item
            .get("language")
            .and_then(Value::as_str)
            .map_or(0, str::len);
        let Some(left) = text.and_then(|text| remaining.checked_sub(text.len() + language_bytes))
        else {
            return false;
        };
        remaining = left;
        true
    })
}

#[cfg(test)]
mod tests;
