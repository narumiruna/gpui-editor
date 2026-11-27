use std::num::NonZeroUsize;

use gpui_editor_codemirror::lsp::{
    SyncKind,
    rpc::{
        FrameError, LspEvent, LspSession, LspState, PendingRequests, RequestId, ResponseError,
        RpcError, RpcId, RpcMessage, RpcResolution, StdioCodec,
    },
};
use serde_json::{Map, Value, json};

fn object(value: Value) -> RpcMessage {
    value.try_into().unwrap()
}

fn reply(id: RequestId, result: Value) -> RpcMessage {
    RpcMessage::Response {
        id: Some(id.into()),
        outcome: Ok(result),
    }
}

#[test]
fn headerless_messages_match_the_pinned_client_transport() {
    for text in [
        r#"{"jsonrpc":"2.0","id":"configuration/1","method":"workspace/configuration","params":{"items":[]}}"#,
        r#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"type":3,"message":"起動"}}"#,
        r#"{"jsonrpc":"2.0","method":"example","params":["positional"]}"#,
        r#"{"jsonrpc":"2.0","method":"example"}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error","data":null}}"#,
    ] {
        let decoded = RpcMessage::from_json(text.as_bytes()).unwrap();
        assert_eq!(
            Value::from(decoded),
            serde_json::from_str::<Value>(text).unwrap()
        );
    }
    assert_eq!(
        RpcMessage::from_json(
            br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#
        ),
        Ok(RpcMessage::Response {
            id: None,
            outcome: Err(ResponseError {
                code: -32700,
                message: "Parse error".into(),
                data: None
            }),
        })
    );
    for body in [b"".as_slice(), b"{", b"{} {}", b"\xff"] {
        assert_eq!(RpcMessage::from_json(body), Err(RpcError::InvalidJson));
    }
    for value in [
        json!([{"jsonrpc":"2.0","method":"example"}]),
        json!({"jsonrpc":"1.0","method":"example"}),
        json!({"jsonrpc":"2.0","method":""}),
        json!({"jsonrpc":"2.0","method":null}),
        json!({"jsonrpc":"2.0","method":"example","id":null}),
        json!({"jsonrpc":"2.0","method":"example","result":null}),
        json!({"jsonrpc":"2.0","method":"example","params":null}),
        json!({"jsonrpc":"2.0","method":"example","params":7}),
        json!({"jsonrpc":"2.0","result":null}),
        json!({"jsonrpc":"2.0","id":true,"result":null}),
        json!({"jsonrpc":"2.0","id":1.0,"result":null}),
        json!({"jsonrpc":"2.0","id":1}),
        json!({"jsonrpc":"2.0","id":1,"params":{},"result":null}),
        json!({"jsonrpc":"2.0","id":1,"result":null,"error":{"code":-32601,"message":"Missing"}}),
        json!({"jsonrpc":"2.0","id":1,"error":null}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601.0,"message":"Missing"}}),
    ] {
        assert!(matches!(
            RpcMessage::try_from(value),
            Err(RpcError::InvalidMessage(_))
        ));
    }
}

#[test]
fn stdio_length_counts_utf8_bytes_and_frames_survive_every_split() {
    let value = json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"message":"é😀雪"}});
    let body = value.to_string();
    assert!(body.len() > body.chars().count());
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut codec = StdioCodec::new(header.len(), body.len()).unwrap();
    let frame = codec.encode(&value).unwrap();
    assert_eq!(frame, [header.as_bytes(), body.as_bytes()].concat());
    for split in 0..frame.len() {
        let (head, tail) = frame.split_at(split);
        codec
            .feed(head, |_| panic!("incomplete frame at byte {split}"))
            .unwrap();
        let mut received = Vec::new();
        codec
            .feed(tail, |body| received.push(RpcMessage::from_json(body)))
            .unwrap();
        assert_eq!(received, [Ok(object(value.clone()))]);
        assert_eq!(codec.finish(), Ok(()));
    }
    let mut too_small = StdioCodec::new(header.len(), body.len() - 1).unwrap();
    assert_eq!(too_small.encode(&value), Err(FrameError::BodyTooLarge));
    assert_eq!(
        too_small.feed(header.as_bytes(), |_| unreachable!()),
        Err(FrameError::BodyTooLarge)
    );
}

#[test]
fn stdio_reports_bad_headers_and_truncation_without_guessing_the_next_boundary() {
    for (header, body) in [(0, 1), (20, 1), (32, 0), (usize::MAX, 1), (64, usize::MAX)] {
        assert_eq!(
            StdioCodec::new(header, body).unwrap_err(),
            FrameError::InvalidLimits
        );
    }
    assert_eq!(
        StdioCodec::new(21, 16).unwrap().encode(&json!("12345678")),
        Err(FrameError::HeaderTooLarge)
    );
    for (input, expected) in [
        (
            b"X-Test: x\r\n\r\n".as_slice(),
            FrameError::InvalidHeader("missing Content-Length"),
        ),
        (
            b"Content-Length: 2\r\ncontent-length: 2\r\n\r\n",
            FrameError::InvalidHeader("duplicate Content-Length"),
        ),
        (
            b"Content-Length: +2\r\n\r\n",
            FrameError::InvalidHeader("invalid Content-Length"),
        ),
        (
            b"Content-Length: -2\r\n\r\n",
            FrameError::InvalidHeader("invalid Content-Length"),
        ),
        (
            b"Content-Length: 2 0\r\n\r\n",
            FrameError::InvalidHeader("invalid Content-Length"),
        ),
        (
            b"Content-Length: 2\nX-Test: 0\r\n\r\n",
            FrameError::InvalidHeader("invalid field name or value"),
        ),
        (
            b"Content-Length: \xff\r\n\r\n",
            FrameError::InvalidHeader("non-ASCII bytes"),
        ),
        (b"Content-Length: 17\r\n\r\n", FrameError::BodyTooLarge),
        (
            b"Content-Length: 184467440737095516160\r\n\r\n",
            FrameError::BodyTooLarge,
        ),
        (
            b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            FrameError::HeaderTooLarge,
        ),
    ] {
        let mut codec = StdioCodec::new(64, 16).unwrap();
        assert_eq!(codec.feed(input, |_| unreachable!()), Err(expected));
        assert_eq!(
            codec.feed(b"Content-Length: 2\r\n\r\n{}", |_| unreachable!()),
            Err(FrameError::Poisoned)
        );
    }
    for partial in [b"Content-Len".as_slice(), b"Content-Length: 2\r\n\r\n{"] {
        let mut codec = StdioCodec::new(64, 16).unwrap();
        assert_eq!(codec.feed(partial, |_| unreachable!()), Ok(()));
        assert_eq!(codec.finish(), Err(FrameError::Truncated));
        assert_eq!(codec.finish(), Err(FrameError::Poisoned));
    }
}

#[test]
fn stdio_recovers_at_known_boundaries_without_consuming_pending_requests_on_bad_json() {
    let mut codec = StdioCodec::new(128, 128).unwrap();
    let mut pending = PendingRequests::new(NonZeroUsize::new(1).unwrap());
    let (id, _) = pending.request("hover", None).unwrap();
    let invalid = codec
        .encode(&json!({"jsonrpc":"2.0","id":1,"result":null,"error":null}))
        .unwrap();
    let valid = codec.encode(&reply(id, Value::Null).into()).unwrap();
    let wire = [
        b"content-length:\t0\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n"
            .as_slice(),
        b"Content-Length: 1\r\n\r\n\xff",
        &invalid,
        &valid,
        b"X-Test: missing-length\r\n\r\n",
    ]
    .concat();
    let mut delivered = Vec::new();
    assert_eq!(
        codec.feed(&wire, |body| delivered.push(RpcMessage::from_json(body))),
        Err(FrameError::InvalidHeader("missing Content-Length"))
    );
    assert_eq!(
        delivered,
        [
            Err(RpcError::InvalidJson),
            Err(RpcError::InvalidJson),
            Err(RpcError::InvalidMessage("expected either result or error")),
            Ok(reply(id, Value::Null)),
        ]
    );
    assert_eq!(pending.len(), 1);
    for message in delivered.into_iter().flatten() {
        let RpcMessage::Response {
            id: wire_id,
            outcome,
        } = message
        else {
            unreachable!()
        };
        assert_eq!(
            pending.resolve(wire_id, outcome),
            Ok(RpcResolution {
                id,
                cancel_requested: false,
                outcome: Ok(Value::Null)
            })
        );
    }
    assert!(pending.is_empty());
}

#[test]
fn request_tracking_is_bounded_and_does_not_coerce_or_reuse_ids() {
    let mut pending = PendingRequests::new(NonZeroUsize::new(1).unwrap());
    assert_eq!(
        pending.request("", None),
        Err(RpcError::InvalidMessage("method"))
    );
    let (first, _) = pending.request("first", None).unwrap();
    assert_eq!(first.get(), 1);
    assert_eq!(
        pending.request("would overflow", None),
        Err(RpcError::PendingLimit)
    );
    for id in [
        None,
        Some(RpcId::Number(-1)),
        Some(RpcId::Number(4_294_967_297)),
        Some(RpcId::String("1".into())),
    ] {
        assert_eq!(
            pending.resolve(id.clone(), Ok(Value::Null)),
            Err(RpcError::UnknownResponse(id))
        );
    }
    assert!(pending.cancel(first).unwrap().is_some());
    assert_eq!(pending.cancel(first), Ok(None));
    assert_eq!(
        pending.request("still full", None),
        Err(RpcError::PendingLimit)
    );
    assert_eq!(
        pending.resolve(
            Some(first.into()),
            Ok(json!("completed despite cancellation"))
        ),
        Ok(RpcResolution {
            id: first,
            cancel_requested: true,
            outcome: Ok(json!("completed despite cancellation")),
        })
    );
    let (second, _) = pending.request("second", None).unwrap();
    assert_eq!(second.get(), 2);
    assert_eq!(pending.abort(second), Ok(()));
    assert_eq!(pending.abort(second), Err(RpcError::UnknownRequest(second)));
    for id in [first, second] {
        assert_eq!(
            pending.resolve(Some(id.into()), Ok(Value::Null)),
            Err(RpcError::UnknownResponse(Some(id.into())))
        );
        assert_eq!(pending.cancel(id), Err(RpcError::UnknownRequest(id)));
    }
    let (third, _) = pending.request("third", None).unwrap();
    assert_eq!(third.get(), 3);
}

struct Endpoint {
    codec: StdioCodec,
    wire: Vec<u8>,
}

impl Endpoint {
    fn new() -> Self {
        Self {
            codec: StdioCodec::new(256, 4096).unwrap(),
            wire: Vec::new(),
        }
    }

    fn send(&mut self, message: RpcMessage) {
        self.wire
            .extend(self.codec.encode(&message.into()).unwrap());
    }

    fn read_from(&mut self, other: &mut Self, chunk_bytes: usize) -> Vec<RpcMessage> {
        let mut messages = Vec::new();
        for chunk in std::mem::take(&mut other.wire).chunks(chunk_bytes) {
            self.codec
                .feed(chunk, |body| {
                    messages.push(RpcMessage::from_json(body).unwrap())
                })
                .unwrap();
        }
        messages
    }
}

#[test]
fn duplex_session_initializes_then_handles_requests_notifications_and_cancellation() {
    let (mut client, mut server) = (Endpoint::new(), Endpoint::new());
    let mut session = LspSession::new(NonZeroUsize::new(2).unwrap());
    let initialize = json!({"processId":null,"clientInfo":{"name":"test"},"rootUri":"file:///test","capabilities":{}});
    let (init, request) = session
        .initialize(initialize.as_object().unwrap().clone())
        .unwrap();
    assert_eq!(init.get(), 1);
    assert_eq!(session.state(), LspState::Initializing(init));
    assert_eq!(session.server_capabilities(), None);
    assert_eq!(
        session.initialize(Map::new()),
        Err(RpcError::InvalidSessionState)
    );
    assert_eq!(
        session.request("textDocument/hover", None),
        Err(RpcError::InvalidSessionState)
    );
    assert_eq!(
        session.notification("textDocument/didOpen", None),
        Err(RpcError::InvalidSessionState)
    );
    client.send(request);
    assert_eq!(
        server.read_from(&mut client, 1),
        [object(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":initialize})
        )]
    );

    let capabilities =
        json!({"textDocumentSync":{"openClose":true,"change":2},"hoverProvider":true});
    server.send(reply(
        init,
        json!({"capabilities":capabilities,"serverInfo":{"name":"simulated"}}),
    ));
    let LspEvent::Initialized(initialized) = session
        .receive(client.read_from(&mut server, 7).pop().unwrap())
        .unwrap()
    else {
        panic!("expected initialized notification");
    };
    assert_eq!(session.state(), LspState::Ready);
    assert_eq!(session.server_capabilities(), Some(&capabilities));
    assert_eq!(
        SyncKind::from_server_capabilities(session.server_capabilities().unwrap()),
        Ok(SyncKind::Incremental)
    );
    client.send(initialized);
    let did_open = json!({"textDocument":{"uri":"file:///test/main.rs","languageId":"rust","version":1,"text":"// 😀\n"}});
    client.send(
        session
            .notification("textDocument/didOpen", Some(did_open.clone()))
            .unwrap(),
    );
    let params =
        json!({"textDocument":{"uri":"file:///test/main.rs"},"position":{"line":0,"character":0}});
    let (hover, hover_msg) = session
        .request("textDocument/hover", Some(params.clone()))
        .unwrap();
    let (definition, definition_msg) = session
        .request("textDocument/definition", Some(params.clone()))
        .unwrap();
    client.send(hover_msg);
    client.send(definition_msg);
    client.send(session.cancel(hover).unwrap().unwrap());
    assert_eq!(
        server.read_from(&mut client, 4096),
        [
            object(json!({"jsonrpc":"2.0","method":"initialized","params":{}})),
            object(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":did_open})),
            object(
                json!({"jsonrpc":"2.0","id":hover.get(),"method":"textDocument/hover","params":params})
            ),
            object(
                json!({"jsonrpc":"2.0","id":definition.get(),"method":"textDocument/definition","params":params})
            ),
            object(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":hover.get()}})),
        ]
    );

    let diagnostics = object(
        json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///test/main.rs","diagnostics":[]}}),
    );
    let server_request = object(
        json!({"jsonrpc":"2.0","id":"configuration/1","method":"workspace/configuration","params":{"items":[]}}),
    );
    let cancellation = ResponseError {
        code: -32800,
        message: "Request cancelled".into(),
        data: Some(json!({"detail":"client request"})),
    };
    server.send(reply(definition, Value::Null));
    server.send(diagnostics.clone());
    server.send(server_request.clone());
    server.send(RpcMessage::Response {
        id: Some(hover.into()),
        outcome: Err(cancellation.clone()),
    });
    let events = client
        .read_from(&mut server, 17)
        .into_iter()
        .map(|message| session.receive(message))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        events,
        [
            LspEvent::Resolution(RpcResolution {
                id: definition,
                cancel_requested: false,
                outcome: Ok(Value::Null)
            }),
            LspEvent::Message(diagnostics),
            LspEvent::Message(server_request),
            LspEvent::Resolution(RpcResolution {
                id: hover,
                cancel_requested: true,
                outcome: Err(cancellation)
            }),
        ]
    );
    assert_eq!(
        session.receive(reply(hover, Value::Null)),
        Err(RpcError::UnknownResponse(Some(hover.into())))
    );
    let (first, _) = session.request("textDocument/hover", None).unwrap();
    let (second, _) = session.request("textDocument/references", None).unwrap();
    assert_eq!(session.close(), [first, second]);
    assert_eq!(session.close(), []);
    assert_eq!(session.state(), LspState::Closed);
    assert_eq!(session.server_capabilities(), None);
}

#[test]
fn initialization_fails_on_bad_capabilities_error_or_local_abort() {
    for value in [Value::Null, json!({}), json!({"capabilities":[]})] {
        let mut session = LspSession::new(NonZeroUsize::new(1).unwrap());
        let (id, _) = session.initialize(Map::new()).unwrap();
        assert_eq!(
            session.receive(reply(id, value)),
            Err(RpcError::InvalidInitializeResult)
        );
        assert_eq!(session.state(), LspState::Failed);
        assert_eq!(session.server_capabilities(), None);
        assert_eq!(session.close(), []);
    }
    let mut session = LspSession::new(NonZeroUsize::new(1).unwrap());
    let (id, _) = session.initialize(Map::new()).unwrap();
    let error = ResponseError {
        code: -32603,
        message: "Failed to start".into(),
        data: None,
    };
    assert_eq!(
        session.receive(RpcMessage::Response {
            id: Some(id.into()),
            outcome: Err(error.clone())
        }),
        Ok(LspEvent::Resolution(RpcResolution {
            id,
            cancel_requested: false,
            outcome: Err(error)
        }))
    );
    assert_eq!(session.state(), LspState::Failed);

    let mut session = LspSession::new(NonZeroUsize::new(1).unwrap());
    let (id, _) = session.initialize(Map::new()).unwrap();
    assert_eq!(session.abort(id), Ok(()));
    assert_eq!(session.state(), LspState::Failed);
    assert_eq!(session.close(), []);
}
