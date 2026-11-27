//! JSON-RPC objects, optional LSP stdio framing, and request tracking.
//!
//! `@codemirror/lsp-client` 6.3.1 sends and receives complete JSON strings
//! without LSP headers. Use [`RpcMessage`] directly for such a transport.
//! [`StdioCodec`] adds `Content-Length` framing for byte streams. Neither API
//! runs I/O. [`LspSession`] adds the client initialization sequence.

mod frame;
mod session;

pub use frame::{FrameError, StdioCodec};
pub use session::{LspEvent, LspSession, LspState, PendingRequests, RequestId, RpcResolution};

use std::fmt;

use serde_json::{Map, Value};

/// A JSON-RPC ID. Numeric strings are distinct from integers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcId {
    Number(i64),
    String(String),
}

impl RpcId {
    fn parse(value: Value) -> Result<Self, RpcError> {
        match value {
            Value::Number(number) => number.as_i64().map(Self::Number),
            Value::String(string) => Some(Self::String(string)),
            _ => None,
        }
        .ok_or(RpcError::InvalidMessage("id"))
    }
}

impl From<RpcId> for Value {
    fn from(id: RpcId) -> Self {
        match id {
            RpcId::Number(number) => Self::from(number),
            RpcId::String(string) => Self::String(string),
        }
    }
}

/// An error in a peer's response, including optional error data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

impl ResponseError {
    fn parse(value: Value) -> Result<Self, RpcError> {
        let Value::Object(mut fields) = value else {
            return Err(RpcError::InvalidMessage("error"));
        };
        let code = fields
            .get("code")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok())
            .ok_or(RpcError::InvalidMessage("error.code"))?;
        let Some(Value::String(message)) = fields.remove("message") else {
            return Err(RpcError::InvalidMessage("error.message"));
        };
        Ok(Self {
            code,
            message,
            data: fields.remove("data"),
        })
    }
}

impl From<ResponseError> for Value {
    fn from(error: ResponseError) -> Self {
        let mut fields = Map::from_iter([
            ("code".into(), Value::from(error.code)),
            ("message".into(), Value::String(error.message)),
        ]);
        if let Some(data) = error.data {
            fields.insert("data".into(), data);
        }
        Self::Object(fields)
    }
}

/// One JSON-RPC 2.0 object. Batches are not supported by this LSP codec.
///
/// Use [`Self::request`] and [`Self::notification`] to check outbound methods
/// and params. A response always has one of `result` or `error`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcMessage {
    Request {
        id: RpcId,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
    Response {
        /// `None` is an explicit JSON `null`, not a missing field.
        id: Option<RpcId>,
        outcome: Result<Value, ResponseError>,
    },
}

impl RpcMessage {
    /// Decode a complete UTF-8 JSON body with no framing headers. The
    /// transport must enforce the body size limit before calling this method.
    pub fn from_json(body: &[u8]) -> Result<Self, RpcError> {
        let value: Value = serde_json::from_slice(body).map_err(|_| RpcError::InvalidJson)?;
        value.try_into()
    }

    pub fn request(
        id: RpcId,
        method: impl Into<String>,
        params: Option<Value>,
    ) -> Result<Self, RpcError> {
        let method = method.into();
        check_call(&method, params.as_ref())?;
        Ok(Self::Request { id, method, params })
    }

    pub fn notification(
        method: impl Into<String>,
        params: Option<Value>,
    ) -> Result<Self, RpcError> {
        let method = method.into();
        check_call(&method, params.as_ref())?;
        Ok(Self::Notification { method, params })
    }
}

fn check_call(method: &str, params: Option<&Value>) -> Result<(), RpcError> {
    if method.is_empty() {
        return Err(RpcError::InvalidMessage("method"));
    }
    if params.is_some_and(|value| !value.is_object() && !value.is_array()) {
        return Err(RpcError::InvalidMessage("params"));
    }
    Ok(())
}

impl TryFrom<Value> for RpcMessage {
    type Error = RpcError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let Value::Object(mut fields) = value else {
            return Err(RpcError::InvalidMessage("expected an object"));
        };
        if fields.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(RpcError::InvalidMessage("jsonrpc"));
        }
        let id = fields.remove("id");
        let params = fields.remove("params");
        let result = fields.remove("result");
        let error = fields.remove("error");
        if let Some(method) = fields.remove("method") {
            let Value::String(method) = method else {
                return Err(RpcError::InvalidMessage("method"));
            };
            if result.is_some() || error.is_some() {
                return Err(RpcError::InvalidMessage("call has response fields"));
            }
            return match id {
                Some(id) => Self::request(RpcId::parse(id)?, method, params),
                None => Self::notification(method, params),
            };
        }
        if params.is_some() {
            return Err(RpcError::InvalidMessage("response has params"));
        }
        let id = match id {
            Some(Value::Null) => None,
            Some(id) => Some(RpcId::parse(id)?),
            None => return Err(RpcError::InvalidMessage("response has no id")),
        };
        let outcome = match (result, error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(ResponseError::parse(error)?),
            _ => return Err(RpcError::InvalidMessage("expected either result or error")),
        };
        Ok(Self::Response { id, outcome })
    }
}

impl From<RpcMessage> for Value {
    fn from(message: RpcMessage) -> Self {
        let mut fields = Map::new();
        fields.insert("jsonrpc".into(), Value::from("2.0"));
        let (method, params) = match message {
            RpcMessage::Request { id, method, params } => {
                fields.insert("id".into(), id.into());
                (method, params)
            }
            RpcMessage::Notification { method, params } => (method, params),
            RpcMessage::Response { id, outcome } => {
                fields.insert("id".into(), id.map_or(Value::Null, Value::from));
                match outcome {
                    Ok(result) => fields.insert("result".into(), result),
                    Err(error) => fields.insert("error".into(), error.into()),
                };
                return Self::Object(fields);
            }
        };
        fields.insert("method".into(), Value::String(method));
        if let Some(params) = params {
            fields.insert("params".into(), params);
        }
        Self::Object(fields)
    }
}

/// A local protocol or request tracking error, not a peer response error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcError {
    InvalidJson,
    InvalidMessage(&'static str),
    PendingLimit,
    RequestIdsExhausted,
    UnknownRequest(RequestId),
    UnknownResponse(Option<RpcId>),
    InvalidSessionState,
    InvalidInitializeResult,
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson => f.write_str("invalid JSON-RPC body"),
            Self::InvalidMessage(reason) => write!(f, "invalid JSON-RPC message: {reason}"),
            Self::PendingLimit => f.write_str("too many pending JSON-RPC requests"),
            Self::RequestIdsExhausted => f.write_str("no JSON-RPC request IDs remain"),
            Self::UnknownRequest(id) => write!(f, "no pending JSON-RPC request for {}", id.get()),
            Self::UnknownResponse(id) => write!(f, "unmatched JSON-RPC response ID: {id:?}"),
            Self::InvalidSessionState => f.write_str("operation not allowed in this LSP state"),
            Self::InvalidInitializeResult => {
                f.write_str("LSP initialize result has no capabilities object")
            }
        }
    }
}

impl std::error::Error for RpcError {}
