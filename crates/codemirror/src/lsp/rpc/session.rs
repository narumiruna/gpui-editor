use std::{collections::BTreeMap, num::NonZeroUsize};

use serde_json::{Map, Value, json};

use super::{ResponseError, RpcError, RpcId, RpcMessage};

/// A locally assigned request ID in the positive LSP integer range.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestId(i32);

impl RequestId {
    pub fn get(self) -> i32 {
        self.0
    }
}

impl From<RequestId> for RpcId {
    fn from(id: RequestId) -> Self {
        Self::Number(i64::from(id.get()))
    }
}

/// The terminal reply to one local request.
#[derive(Debug, Eq, PartialEq)]
pub struct RpcResolution {
    pub id: RequestId,
    pub cancel_requested: bool,
    pub outcome: Result<Value, ResponseError>,
}

/// Bounded tracking for requests sent over one connection.
///
/// IDs start at one, increase monotonically, and are never reused by this
/// tracker. The caller sends returned messages and calls [`Self::abort`] on
/// send failure or timeout. The tracker has no clock or event queue.
#[derive(Debug)]
pub struct PendingRequests {
    next_id: Option<i32>,
    max_pending: NonZeroUsize,
    pending: BTreeMap<RequestId, bool>,
}

impl PendingRequests {
    pub fn new(max_pending: NonZeroUsize) -> Self {
        Self {
            next_id: Some(1),
            max_pending,
            pending: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn request(
        &mut self,
        method: impl Into<String>,
        params: Option<Value>,
    ) -> Result<(RequestId, RpcMessage), RpcError> {
        if self.len() == self.max_pending.get() {
            return Err(RpcError::PendingLimit);
        }
        let id = RequestId(self.next_id.ok_or(RpcError::RequestIdsExhausted)?);
        let message = RpcMessage::request(id.into(), method, params)?;
        self.next_id = id.get().checked_add(1);
        self.pending.insert(id, false);
        Ok((id, message))
    }

    /// Settle a request only if its numeric ID matches exactly. Unknown,
    /// duplicate, and late responses do not change the pending set.
    pub fn resolve(
        &mut self,
        id: Option<RpcId>,
        outcome: Result<Value, ResponseError>,
    ) -> Result<RpcResolution, RpcError> {
        if let Some(RpcId::Number(number)) = &id
            && let Ok(number) = i32::try_from(*number)
            && let Some(cancel_requested) = self.pending.remove(&RequestId(number))
        {
            return Ok(RpcResolution {
                id: RequestId(number),
                cancel_requested,
                outcome,
            });
        }
        Err(RpcError::UnknownResponse(id))
    }

    /// Request LSP cancellation by ID. As with the pinned client, this does not
    /// settle the request. Send the returned `$/cancelRequest` notification;
    /// repeated calls for the same pending ID return `None`.
    pub fn cancel(&mut self, id: RequestId) -> Result<Option<RpcMessage>, RpcError> {
        let cancelled = self
            .pending
            .get_mut(&id)
            .ok_or(RpcError::UnknownRequest(id))?;
        if *cancelled {
            return Ok(None);
        }
        *cancelled = true;
        Ok(Some(RpcMessage::Notification {
            method: "$/cancelRequest".into(),
            params: Some(json!({"id": id.get()})),
        }))
    }

    /// Stop tracking one request locally. This does not send a message.
    pub fn abort(&mut self, id: RequestId) -> Result<(), RpcError> {
        self.pending
            .remove(&id)
            .map(|_| ())
            .ok_or(RpcError::UnknownRequest(id))
    }

    /// Stop tracking all requests and return their IDs in issuance order.
    pub fn abort_all(&mut self) -> Vec<RequestId> {
        std::mem::take(&mut self.pending).into_keys().collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LspState {
    New,
    Initializing(RequestId),
    Ready,
    Failed,
    Closed,
}

#[derive(Debug, Eq, PartialEq)]
pub enum LspEvent {
    /// Send this `initialized` notification before sending further traffic.
    Initialized(RpcMessage),
    /// A feature response or a peer error in response to `initialize`.
    Resolution(RpcResolution),
    /// A peer request or notification for the caller to handle.
    Message(RpcMessage),
}

/// Client-side initialization and request tracking for a single LSP connection.
///
/// Like `@codemirror/lsp-client` 6.3.1, this sends `initialize` first, retains
/// server capabilities, and produces `initialized` with empty params before
/// normal traffic. Calls before readiness return an error instead of queuing.
/// The caller supplies its own initialize params and capabilities. This type
/// does not implement feature dispatch, server request replies, or shutdown.
#[derive(Debug)]
pub struct LspSession {
    pending: PendingRequests,
    state: LspState,
    server_capabilities: Option<Value>,
}

impl LspSession {
    pub fn new(max_pending: NonZeroUsize) -> Self {
        Self {
            pending: PendingRequests::new(max_pending),
            state: LspState::New,
            server_capabilities: None,
        }
    }

    pub fn state(&self) -> LspState {
        self.state
    }

    pub fn server_capabilities(&self) -> Option<&Value> {
        self.server_capabilities.as_ref()
    }

    pub fn initialize(
        &mut self,
        params: Map<String, Value>,
    ) -> Result<(RequestId, RpcMessage), RpcError> {
        if self.state != LspState::New {
            return Err(RpcError::InvalidSessionState);
        }
        let (id, message) = self
            .pending
            .request("initialize", Some(Value::Object(params)))?;
        self.state = LspState::Initializing(id);
        Ok((id, message))
    }

    pub fn request(
        &mut self,
        method: impl Into<String>,
        params: Option<Value>,
    ) -> Result<(RequestId, RpcMessage), RpcError> {
        self.require_ready()?;
        self.pending.request(method, params)
    }

    pub fn notification(
        &self,
        method: impl Into<String>,
        params: Option<Value>,
    ) -> Result<RpcMessage, RpcError> {
        self.require_ready()?;
        RpcMessage::notification(method, params)
    }

    pub fn cancel(&mut self, id: RequestId) -> Result<Option<RpcMessage>, RpcError> {
        self.require_ready()?;
        self.pending.cancel(id)
    }

    /// Stop tracking a request locally, including an initialization that could
    /// not be sent or that reached its deadline.
    pub fn abort(&mut self, id: RequestId) -> Result<(), RpcError> {
        self.pending.abort(id)?;
        if self.state == LspState::Initializing(id) {
            self.state = LspState::Failed;
        }
        Ok(())
    }

    /// Release pending requests on disconnect. Use a new session to reconnect.
    pub fn close(&mut self) -> Vec<RequestId> {
        self.state = LspState::Closed;
        self.server_capabilities = None;
        self.pending.abort_all()
    }

    /// Handle one validated message from the peer.
    pub fn receive(&mut self, message: RpcMessage) -> Result<LspEvent, RpcError> {
        if !matches!(self.state, LspState::Initializing(_) | LspState::Ready) {
            return Err(RpcError::InvalidSessionState);
        }
        let RpcMessage::Response { id, outcome } = message else {
            return Ok(LspEvent::Message(message));
        };
        let resolution = self.pending.resolve(id, outcome)?;
        if self.state != LspState::Initializing(resolution.id) {
            return Ok(LspEvent::Resolution(resolution));
        }
        self.state = LspState::Failed;
        let RpcResolution {
            outcome: Ok(result),
            ..
        } = resolution
        else {
            return Ok(LspEvent::Resolution(resolution));
        };
        let Value::Object(mut result) = result else {
            return Err(RpcError::InvalidInitializeResult);
        };
        let Some(capabilities @ Value::Object(_)) = result.remove("capabilities") else {
            return Err(RpcError::InvalidInitializeResult);
        };
        self.server_capabilities = Some(capabilities);
        self.state = LspState::Ready;
        Ok(LspEvent::Initialized(RpcMessage::Notification {
            method: "initialized".into(),
            params: Some(json!({})),
        }))
    }

    fn require_ready(&self) -> Result<(), RpcError> {
        if self.state == LspState::Ready {
            Ok(())
        } else {
            Err(RpcError::InvalidSessionState)
        }
    }
}
