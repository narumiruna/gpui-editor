use std::{
    collections::VecDeque,
    fmt,
    process::ExitStatus,
    time::{Duration, Instant},
};

use serde_json::{Map, Value};

use crate::{
    lsp::{
        DocumentWorkspace, EditRequest, LspClient, LspError,
        features::{FeatureError, FeatureRequest},
        signature::SignatureRequest,
    },
    view::{EditorView, PluginId},
};

use super::{
    Deadline, IDLE_INTERVAL, LspState, MAX_PENDING, Program, RequestId, ResponseError, RpcError,
    RpcId, RpcMessage, StdioTransport, TransportError, TransportEvent,
};

#[derive(Debug)]
pub enum StdioClientError {
    Transport(TransportError),
    Workspace(LspError),
    Feature(FeatureError),
    UnsupportedCapability(&'static str),
}

impl fmt::Display for StdioClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::Workspace(error) => error.fmt(f),
            Self::Feature(error) => error.fmt(f),
            Self::UnsupportedCapability(name) => write!(f, "LSP server does not support {name}"),
        }
    }
}

impl std::error::Error for StdioClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Workspace(error) => Some(error),
            Self::Feature(error) => Some(error),
            Self::UnsupportedCapability(_) => None,
        }
    }
}

impl From<TransportError> for StdioClientError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

/// The wire fields of a prepared native feature request. The driver returns the
/// original request to the host, which owns response decoding and application.
pub trait PreparedRequest {
    /// Return the method, capability key, and params, in that order.
    fn parts(&self) -> (&'static str, &'static str, &Value);
}

impl<T> PreparedRequest for FeatureRequest<T> {
    fn parts(&self) -> (&'static str, &'static str, &Value) {
        (self.method(), self.capability(), self.params())
    }
}

impl<T> PreparedRequest for EditRequest<T> {
    fn parts(&self) -> (&'static str, &'static str, &Value) {
        (self.method(), self.capability(), self.params())
    }
}

impl PreparedRequest for SignatureRequest {
    fn parts(&self) -> (&'static str, &'static str, &Value) {
        (self.method(), self.capability(), self.params())
    }
}

/// Drive one trusted subprocess with one editor workspace. Poll [`Self::next_event`]
/// to drive I/O, including when no request is pending. Editor callbacks only
/// record changes. The host supplies initialize params, handles peer messages,
/// and chooses when to flush document edits between feature requests.
///
/// A connection cannot be reused after a terminal error or shutdown. Dropping
/// the client terminates the direct child without sending `shutdown` or `exit`.
pub struct StdioClient {
    editor: LspClient,
    transport: Option<StdioTransport>,
    outbox: VecDeque<Value>,
}

impl StdioClient {
    /// Start the specified program and queue `initialize`. Attachments may be
    /// added during initialization; feature requests require `Initialized`.
    pub fn start(
        program: &Program,
        params: Map<String, Value>,
        timeout: Duration,
    ) -> Result<Self, TransportError> {
        let mut transport = StdioTransport::spawn(program)?;
        transport.initialize(params, timeout)?;
        Ok(Self {
            editor: LspClient::default(),
            transport: Some(transport),
            outbox: VecDeque::new(),
        })
    }

    pub fn state(&self) -> LspState {
        self.transport
            .as_ref()
            .map_or(LspState::Closed, StdioTransport::state)
    }

    pub fn server_capabilities(&self) -> Option<&Value> {
        self.transport.as_ref()?.server_capabilities()
    }

    /// Attach one editor to a URI. An attachment never starts a process or
    /// loads a file. Attachments can only be added during an active connection.
    pub fn attach<M: Clone>(
        &mut self,
        view: &mut EditorView<M>,
        uri: impl Into<String>,
        language_id: impl Into<String>,
    ) -> Result<PluginId, StdioClientError> {
        self.transport_mut()?;
        self.editor
            .attach(view, uri, language_id)
            .map_err(StdioClientError::Workspace)
    }

    /// Inspect the workspace, for example to resolve a feature response.
    /// Snapshots can include notifications queued locally but not yet sent.
    /// Do not edit or destroy an attached view in the callback.
    pub fn with_workspace<T>(&self, inspect: impl FnOnce(&DocumentWorkspace) -> T) -> T {
        self.editor.with_workspace(inspect)
    }

    /// Queue pending document notifications. `QueueFull` leaves the unsent
    /// suffix in order. Drive I/O and retry to flush any newer edits too.
    pub fn try_flush(&mut self) -> Result<(), StdioClientError> {
        self.require_ready()?;
        self.send_outbox()?;
        self.outbox
            .extend(self.editor.flush().map_err(StdioClientError::Workspace)?);
        self.send_outbox()?;
        Ok(())
    }

    /// Flush before preparing a feature request. On `QueueFull`, the callback
    /// does not run and no request is queued. Poll and retry with the current
    /// editor position. A successful call starts the request timeout and returns
    /// the ID and original request. Match the ID before resolving a response.
    pub fn try_request<R: PreparedRequest>(
        &mut self,
        timeout: Duration,
        prepare: impl FnOnce(&DocumentWorkspace) -> Result<R, FeatureError>,
    ) -> Result<(RequestId, R), StdioClientError> {
        self.try_flush()?;
        self.transport_mut()?.check_queue()?;
        let request = self
            .with_workspace(prepare)
            .map_err(StdioClientError::Feature)?;
        let (method, capability, params) = request.parts();
        if !matches!(
            self.server_capabilities()
                .and_then(|caps| caps.get(capability)),
            Some(Value::Bool(true) | Value::Object(_))
        ) {
            return Err(StdioClientError::UnsupportedCapability(capability));
        }
        let id = self
            .transport_mut()?
            .request(method, Some(params.clone()), timeout)?;
        Ok((id, request))
    }

    /// Queue a reply to a peer request, including during initialization. On
    /// `QueueFull`, poll and retry. This does not capture new document edits.
    pub fn reply(
        &mut self,
        id: RpcId,
        outcome: Result<Value, ResponseError>,
    ) -> Result<(), TransportError> {
        self.send_outbox()?;
        self.transport_mut()?.reply(id, outcome)
    }

    /// Queue cancellation of a pending request. On `QueueFull`, poll and retry.
    pub fn cancel(&mut self, id: RequestId) -> Result<(), TransportError> {
        self.send_outbox()?;
        self.transport_mut()?.cancel(id)
    }

    /// Drive I/O and return the next transport event in order. Once ready, this
    /// also queues document opens and closes and retries captured notifications.
    /// It does not capture new edits. A zero wait makes one nonblocking pass.
    /// Initialization rejection, timeout, and malformed results follow the
    /// transport event contract. Unsupported sync or failure to encode a
    /// document notification closes the connection and invalidates all mappings.
    pub fn next_event(
        &mut self,
        wait: Duration,
    ) -> Result<Option<TransportEvent>, StdioClientError> {
        let deadline = Deadline::new(wait);
        loop {
            let blocked = if self.state() == LspState::Ready {
                match self.send_lifecycle() {
                    Ok(()) => false,
                    Err(TransportError::QueueFull) => true,
                    Err(error) => return Err(error.into()),
                }
            } else {
                false
            };
            let remaining = deadline.remaining(Instant::now());
            let event = self.drive(if blocked {
                remaining.min(IDLE_INTERVAL)
            } else {
                remaining
            })?;
            if !blocked || event.is_some() || deadline.remaining(Instant::now()).is_zero() {
                return Ok(event);
            }
        }
    }

    /// Flush document traffic, then send `shutdown` and `exit` under one
    /// deadline. This requires readiness and discards unrelated events. It
    /// consumes the client, including on failure.
    pub fn shutdown(mut self, timeout: Duration) -> Result<ExitStatus, StdioClientError> {
        self.require_ready()?;
        let deadline = Deadline::new(timeout);
        loop {
            let flushed = match self.try_flush() {
                Ok(()) => true,
                Err(StdioClientError::Transport(TransportError::QueueFull)) => false,
                Err(error) => return Err(error),
            };
            let remaining = deadline.remaining(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::ShutdownTimedOut.into());
            }
            let transport = self.transport_mut()?;
            if flushed
                && transport.check_queue().is_ok()
                && transport.deadlines.len() < MAX_PENDING.get()
            {
                return Ok(self.transport.take().unwrap().shutdown(remaining)?);
            }
            if let Some(TransportEvent::Disconnected { reason, .. }) =
                self.drive(remaining.min(IDLE_INTERVAL))?
            {
                return Err(reason.into());
            }
        }
    }

    fn drive(&mut self, wait: Duration) -> Result<Option<TransportEvent>, StdioClientError> {
        let event = match self.transport_mut()?.next_event(wait) {
            Ok(event) => event,
            Err(error) => {
                self.disconnect();
                return Err(error.into());
            }
        };
        if matches!(event, Some(TransportEvent::Initialized)) {
            let capabilities = self
                .transport
                .as_ref()
                .and_then(StdioTransport::server_capabilities)
                .expect("initialized transport");
            // The transport queued `initialized` ahead of the editor traffic.
            if let Err(error) = self.editor.connected(capabilities) {
                self.disconnect();
                return Err(StdioClientError::Workspace(error));
            }
        } else if matches!(event, Some(TransportEvent::Disconnected { .. })) {
            self.disconnect();
        } else if self.state() == LspState::Failed {
            self.editor.disconnected();
            self.outbox.clear();
        }
        Ok(event)
    }

    fn send_lifecycle(&mut self) -> Result<(), TransportError> {
        self.send_outbox()?;
        self.outbox.extend(self.editor.take_lifecycle());
        self.send_outbox()
    }

    fn send_outbox(&mut self) -> Result<(), TransportError> {
        while !self.outbox.is_empty() {
            self.transport_mut()?.check_queue()?;
            let message = self.outbox.pop_front().expect("nonempty outbox");
            if let Err(error) = self.send_notification(message) {
                // The workspace advanced when the batch was captured. Do not
                // send later traffic against a snapshot the server cannot use.
                self.disconnect();
                return Err(error);
            }
        }
        Ok(())
    }

    fn send_notification(&mut self, message: Value) -> Result<(), TransportError> {
        let RpcMessage::Notification { method, params } = RpcMessage::try_from(message)? else {
            return Err(RpcError::InvalidMessage("document notification").into());
        };
        self.transport_mut()?.notify(method, params)
    }

    fn require_ready(&self) -> Result<(), TransportError> {
        if self.state() == LspState::Ready {
            Ok(())
        } else {
            Err(RpcError::InvalidSessionState.into())
        }
    }

    fn transport_mut(&mut self) -> Result<&mut StdioTransport, TransportError> {
        self.transport
            .as_mut()
            .filter(|transport| !matches!(transport.state(), LspState::Closed | LspState::Failed))
            .ok_or_else(|| RpcError::InvalidSessionState.into())
    }

    fn disconnect(&mut self) {
        self.editor.disconnected();
        self.outbox.clear();
        self.transport.take();
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        self.editor.disconnected();
    }
}
