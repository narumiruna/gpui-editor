#![cfg(unix)]

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{self, Command},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Transaction,
    lsp::{
        FormattingOptions, LspClient, LspError,
        features::{Documentation, FeatureError, FeatureRequest, Hover, hover_request},
        formatting_request, language_server_extensions,
        rpc::{FrameError, LspState, RpcError, RpcId, RpcMessage},
        transport::{
            Program, StdioClient, StdioClientError, StdioTransport, TransportError, TransportEvent,
        },
    },
    view::EditorView,
};
use serde_json::{Map, Value, json};

const WAIT: Duration = Duration::from_secs(5);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "codemirror-lsp-{}-{stamp}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let fixture = Self(directory);
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_stdio.rs");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let output = Command::new(rustc)
            .arg("--edition=2024")
            .arg(source)
            .arg("-o")
            .arg(fixture.executable())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fixture
    }

    fn executable(&self) -> PathBuf {
        self.0.join("fixture")
    }

    fn program(&self, mode: &str) -> Program {
        Program::new(self.executable())
            .arg(mode)
            .arg("空 白; $(literal)")
    }

    fn spawn(&self, mode: &str) -> StdioTransport {
        StdioTransport::spawn(&self.program(mode)).unwrap()
    }

    fn start(&self, mode: &str, timeout: Duration) -> StdioClient {
        let params = Map::from_iter([
            ("processId".into(), Value::Null),
            ("rootUri".into(), Value::Null),
            (
                "capabilities".into(),
                language_server_extensions().client_capabilities(),
            ),
        ]);
        StdioClient::start(&self.program(mode), params, timeout).unwrap()
    }

    fn ready(&self, mode: &str) -> StdioTransport {
        let mut connection = self.spawn(mode);
        assert_eq!(connection.state(), LspState::New);
        let id = connection.initialize(Map::new(), WAIT).unwrap();
        assert_eq!(connection.state(), LspState::Initializing(id));
        assert!(matches!(next(&mut connection), TransportEvent::Initialized));
        assert_eq!(connection.state(), LspState::Ready);
        assert_eq!(
            connection.server_capabilities(),
            Some(&json!({"hoverProvider":true}))
        );
        connection
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn next(connection: &mut StdioTransport) -> TransportEvent {
    connection
        .next_event(WAIT)
        .unwrap()
        .expect("no fixture event")
}

fn notification(connection: &mut StdioTransport, expected: &str) {
    let event = next(connection);
    assert!(
        matches!(&event, TransportEvent::Message(RpcMessage::Notification { method, .. }) if method == expected),
        "{event:?}"
    );
}

#[test]
fn stdio_exchanges_messages_and_settles_cancelled_and_timed_out_requests() {
    let fixture = Fixture::new();
    let mut connection = fixture.ready("exchange");
    let hold = connection.request("test/hold", None, WAIT).unwrap();
    let quick = connection.request("test/quick", None, WAIT).unwrap();
    assert!(matches!(
        next(&mut connection),
        TransportEvent::ProtocolError(RpcError::InvalidJson)
    ));
    let TransportEvent::Resolution(response) = next(&mut connection) else {
        panic!("expected Unicode response");
    };
    assert_eq!(response.id, quick);
    assert_eq!(response.outcome, Ok(json!("é😀雪")));
    let TransportEvent::Message(RpcMessage::Request { id, method, .. }) = next(&mut connection)
    else {
        panic!("expected server request");
    };
    assert_eq!(method, "workspace/configuration");
    assert_eq!(id, RpcId::String("fixture".into()));
    connection.reply(id, Ok(json!([3]))).unwrap();
    connection.cancel(hold).unwrap();
    connection.cancel(hold).unwrap();
    let TransportEvent::Resolution(response) = next(&mut connection) else {
        panic!("expected cancellation response");
    };
    assert_eq!(response.id, hold);
    assert!(response.cancel_requested);
    assert_eq!(response.outcome.unwrap_err().code, -32800);

    let unsent = connection
        .request("test/must-not-be-sent", None, Duration::ZERO)
        .unwrap();
    assert!(
        matches!(connection.next_event(Duration::ZERO).unwrap(), Some(TransportEvent::TimedOut(id)) if id == unsent)
    );
    let stale = connection
        .request("test/timeout", None, Duration::from_secs(1))
        .unwrap();
    notification(&mut connection, "test/received");
    assert!(matches!(next(&mut connection), TransportEvent::TimedOut(id) if id == stale));
    let after = connection
        .request("test/after-timeout", None, WAIT)
        .unwrap();
    assert!(
        matches!(next(&mut connection), TransportEvent::ProtocolError(RpcError::UnknownResponse(Some(id))) if id == stale.into())
    );
    let TransportEvent::Resolution(response) = next(&mut connection) else {
        panic!("expected following response");
    };
    assert_eq!(response.id, after);
    assert_eq!(response.outcome, Ok(Value::Bool(true)));
    assert!(connection.shutdown(WAIT).unwrap().success());
}

#[test]
fn stdio_delivers_attached_view_notifications_before_feature_traffic() {
    const URI: &str = "file:///editor.rs";
    let fixture = Fixture::new();
    let mut connection = fixture.spawn("editor");
    let mut client = LspClient::default();
    let mut view = EditorView::new(EditorState::new("let id"));
    client.attach(&mut view, URI, "rust").unwrap();
    let changes = ChangeSet::single(Change::new(4..6, "name")).unwrap();
    view.update(Transaction::new(changes), (), |_| (None, ()))
        .unwrap();
    connection.initialize(Map::new(), WAIT).unwrap();
    assert!(matches!(next(&mut connection), TransportEvent::Initialized));
    client
        .connected(connection.server_capabilities().unwrap())
        .unwrap();

    let flush = |client: &mut LspClient, connection: &mut StdioTransport| {
        for message in client.flush().unwrap() {
            let RpcMessage::Notification { method, params } =
                RpcMessage::try_from(message).unwrap()
            else {
                panic!("expected a document notification");
            };
            connection.notify(method, params).unwrap();
        }
    };
    flush(&mut client, &mut connection);
    let hover = client.with_workspace(|workspace| hover_request(workspace, URI, 5).unwrap());
    let id = connection
        .request(hover.method(), Some(hover.params().clone()), WAIT)
        .unwrap();
    let event = next(&mut connection);
    let TransportEvent::Resolution(reply) = event else {
        panic!("expected a hover response, got {event:?}");
    };
    assert_eq!(reply.id, id);
    let hover = client
        .with_workspace(|workspace| hover.resolve(workspace, &reply.outcome.unwrap()))
        .unwrap()
        .unwrap();
    assert_eq!(hover.contents, [Documentation::PlainText("fixture".into())]);
    drop(view);
    flush(&mut client, &mut connection);
    assert!(connection.shutdown(WAIT).unwrap().success());
}

#[test]
fn managed_client_orders_batches_and_feature_requests_under_backpressure() {
    const URI: &str = "file:///managed/0.rs";
    let fixture = Fixture::new();
    let mut client = fixture.start("managed", WAIT);
    let mut views: Vec<_> = (0..12)
        .map(|i| {
            let mut view = EditorView::new(EditorState::new("a😀b"));
            client
                .attach(&mut view, format!("file:///managed/{i}.rs"), "rust")
                .unwrap();
            change(&mut view, 5..6, format!("β{i}"));
            view
        })
        .collect();
    let mut transient = EditorView::<()>::new(EditorState::new("unopened"));
    client
        .attach(&mut transient, "file:///transient", "")
        .unwrap();
    drop(transient);
    assert_eq!(client.server_capabilities(), None);
    assert!(matches!(
        client.try_request::<FeatureRequest<()>>(WAIT, |_| panic!("early feature")),
        Err(StdioClientError::Transport(TransportError::Rpc(
            RpcError::InvalidSessionState
        )))
    ));
    let Some(TransportEvent::Message(RpcMessage::Request { id, method, .. })) =
        client.next_event(WAIT).unwrap()
    else {
        panic!("expected peer request during initialization");
    };
    assert_eq!(method, "workspace/configuration");
    client.reply(id, Ok(json!([]))).unwrap();
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Initialized)
    ));
    assert_eq!(client.state(), LspState::Ready);
    assert_eq!(client.server_capabilities().unwrap()["textDocumentSync"], 2);

    let queue_full = client.try_request::<FeatureRequest<()>>(WAIT, |_| panic!("no request slot"));
    assert!(matches!(
        queue_full,
        Err(StdioClientError::Transport(TransportError::QueueFull))
    ));
    change(&mut views[0], 8..8, "!");
    drop(views.pop().unwrap());

    let mut between = false;
    let start = Instant::now();
    let (id, hover) = loop {
        match client.try_request(WAIT, |workspace| hover_request(workspace, URI, 9)) {
            Ok(request) => break request,
            Err(StdioClientError::Transport(TransportError::QueueFull)) => {}
            Err(error) => panic!("request failed: {error}"),
        }
        if let Some(event) = client.next_event(Duration::from_millis(5)).unwrap() {
            assert_between(event, &mut between);
        }
        assert!(
            start.elapsed() < WAIT,
            "client did not drain the output queue"
        );
    };
    assert_eq!(hover.version(), 2);
    let mut event = client.next_event(WAIT).unwrap().unwrap();
    if !matches!(event, TransportEvent::Resolution(_)) {
        assert_between(event, &mut between);
        event = client.next_event(WAIT).unwrap().unwrap();
    }
    assert!(between);
    let TransportEvent::Resolution(reply) = event else {
        panic!("expected hover response, got {event:?}");
    };
    assert_eq!(reply.id, id);
    let hover = client
        .with_workspace(|workspace| hover.resolve(workspace, &reply.outcome.unwrap()))
        .unwrap()
        .unwrap();
    assert_eq!(hover.contents, [Documentation::PlainText("fixture".into())]);
    assert!(matches!(
        client.try_request(WAIT, |workspace| formatting_request(
            workspace,
            URI,
            FormattingOptions {
                tab_size: 4,
                insert_spaces: true
            }
        )),
        Err(StdioClientError::UnsupportedCapability(
            "documentFormattingProvider"
        ))
    ));

    for view in views {
        drop(view);
    }
    let mut last = EditorView::new(EditorState::new("a"));
    client
        .attach(&mut last, "file:///shutdown.rs", "rust")
        .unwrap();
    change(&mut last, 1..1, "b");
    assert!(client.shutdown(WAIT).unwrap().success());
}

#[test]
fn deferred_client_waits_for_document_sync_and_admits_in_fifo_order() {
    const URI: &str = "file:///deferred.rs";
    let fixture = Fixture::new();
    let mut client = fixture.start("deferred", WAIT);
    let mut view = EditorView::new(EditorState::new("a😀b"));
    let original = client.attach(&mut view, URI, "rust").unwrap();
    let mapping = client.with_workspace(|workspace| workspace.workspace_mapping());
    let cancelled = client.defer_request::<FeatureRequest<Hover>>(WAIT).unwrap();
    let cancelled_notification = client.defer_notification().unwrap();
    let before_hover = client.defer_notification().unwrap();
    let replaced = client.defer_request(WAIT).unwrap();
    assert!(view.remove_plugin(original));
    client.attach(&mut view, URI, "rust").unwrap();
    let stale_selection = view.state().selection().main();
    let stale_host = client.defer_request(WAIT).unwrap();
    let unsupported = client.defer_request(WAIT).unwrap();
    let managed = client.defer_notification().unwrap();
    let expires = client.defer_request(Duration::ZERO).unwrap();
    let request_timeout = Duration::from_millis(500);
    let hover = client.defer_request(request_timeout).unwrap();
    let after_hover = client.defer_notification().unwrap();
    let _extra_views: Vec<_> = (0..8)
        .map(|i| {
            let mut extra = EditorView::<()>::new(EditorState::new("a"));
            client
                .attach(&mut extra, format!("file:///deferred/{i}.rs"), "rust")
                .unwrap();
            extra
        })
        .collect();
    change(&mut view, 5..6, "β");
    view.update(
        Transaction::new(ChangeSet::default()).with_selection(EditorSelection::single(7, 7)),
        (),
        |_| (None, ()),
    )
    .unwrap();

    // The fixture holds initialization until the host's reply. Poll for at
    // least the final feature's timeout without admitting any request.
    let Some(TransportEvent::Message(RpcMessage::Request { id, method, .. })) =
        client.next_event(WAIT).unwrap()
    else {
        panic!("expected the initialization gate");
    };
    assert_eq!(method, "workspace/configuration");
    assert!(
        client
            .try_admit_request(&cancelled, |_| panic!("prepared before initialization"))
            .unwrap()
            .is_none()
    );
    drop(cancelled_notification);
    assert!(
        !client
            .try_admit_notification(&before_hover, |_| panic!(
                "prepared a notification during initialization"
            ))
            .unwrap()
    );
    assert!(client.next_event(request_timeout).unwrap().is_none());
    client.reply(id, Ok(json!([]))).unwrap();
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Initialized)
    ));
    assert!(
        client
            .try_admit_request(&replaced, |_| panic!("skipped the FIFO head"))
            .unwrap()
            .is_none()
    );
    drop(cancelled);
    assert!(
        !client
            .try_admit_notification(&before_hover, |_| panic!(
                "prepared with a full output queue"
            ))
            .unwrap()
    );
    let start = Instant::now();
    while !client
        .try_admit_notification(&before_hover, |_| {
            Ok((
                "workspace/didChangeConfiguration",
                Some(json!({"settings":{"phase":"before"}})),
            ))
        })
        .unwrap()
    {
        assert!(
            client
                .next_event(Duration::from_millis(5))
                .unwrap()
                .is_none()
        );
        assert!(start.elapsed() < WAIT);
    }
    assert!(matches!(
        client.try_admit_notification(&before_hover, |_| panic!("reused a notification token")),
        Err(StdioClientError::InvalidDeferredNotification)
    ));
    assert!(
        !client
            .try_admit_notification(&after_hover, |_| panic!("jumped the FIFO"))
            .unwrap()
    );

    let start = Instant::now();
    loop {
        match client.try_admit_request(&replaced, |workspace| {
            if mapping.get_mapping(URI).is_none() {
                return Err(FeatureError::Unsupported("changed attachment"));
            }
            hover_request(workspace, URI, 0)
        }) {
            Err(StdioClientError::Feature(FeatureError::Unsupported("changed attachment"))) => {
                break;
            }
            Ok(None) => {}
            result => panic!("unexpected stale attachment admission: {result:?}"),
        }
        assert!(
            client
                .next_event(Duration::from_millis(5))
                .unwrap()
                .is_none()
        );
        assert!(start.elapsed() < WAIT);
    }
    assert!(matches!(
        client.try_admit_request(&stale_host, |workspace| {
            if view.state().selection().main() != stale_selection {
                return Err(FeatureError::Unsupported("changed selection"));
            }
            hover_request(workspace, URI, stale_selection.head())
        }),
        Err(StdioClientError::Feature(FeatureError::Unsupported(
            "changed selection"
        )))
    ));
    assert!(matches!(
        client.try_admit_request(&unsupported, |workspace| formatting_request(
            workspace,
            URI,
            FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
            }
        )),
        Err(StdioClientError::UnsupportedCapability(
            "documentFormattingProvider"
        ))
    ));
    assert!(matches!(
        client.try_admit_notification(&managed, |_| Ok(("textDocument/didClose", None))),
        Err(StdioClientError::ManagedNotification(
            "textDocument/didClose"
        ))
    ));

    let (expired, _) = client
        .try_admit_request(&expires, |workspace| hover_request(workspace, URI, 0))
        .unwrap()
        .unwrap();
    assert_eq!(expired.get(), 2, "local rejections must not allocate IDs");
    assert!(matches!(
        client.try_admit_request(&expires, |_| panic!("reused a token")),
        Err(StdioClientError::InvalidDeferredRequest)
    ));
    assert!(matches!(
        client.next_event(Duration::ZERO).unwrap(),
        Some(TransportEvent::TimedOut(id)) if id == expired
    ));
    let (id, hover) = client
        .try_admit_request(&hover, |workspace| {
            hover_request(workspace, URI, view.state().selection().main().head())
        })
        .unwrap()
        .unwrap();
    assert_eq!(hover.version(), 2);
    assert_eq!(id.get(), 3);
    assert!(
        client
            .try_admit_notification(&after_hover, |_| {
                Ok((
                    "workspace/didChangeConfiguration",
                    Some(json!({"settings":{"phase":"after"}})),
                ))
            })
            .unwrap()
    );
    let Some(TransportEvent::Resolution(reply)) = client.next_event(WAIT).unwrap() else {
        panic!("expected a hover response after admission");
    };
    assert_eq!(reply.id, id);
    let hover = client
        .with_workspace(|workspace| hover.resolve(workspace, &reply.outcome.unwrap()))
        .unwrap()
        .unwrap();
    assert_eq!(hover.contents, [Documentation::PlainText("fixture".into())]);
    assert!(client.shutdown(WAIT).unwrap().success());
}

#[test]
fn deferred_client_bounds_staging_and_waits_for_a_pending_slot() {
    const URI: &str = "file:///deferred-pending.rs";
    let fixture = Fixture::new();
    let mut client = fixture.start("deferred-pending", WAIT);
    let mut view = EditorView::<()>::new(EditorState::new("a"));
    client.attach(&mut view, URI, "rust").unwrap();
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Initialized)
    ));
    let notice = client.defer_notification().unwrap();
    let token = client.defer_request(WAIT).unwrap();
    let mut rest: Vec<_> = (2..64)
        .map(|_| client.defer_request::<FeatureRequest<Hover>>(WAIT).unwrap())
        .collect();
    assert!(matches!(
        client.defer_request::<FeatureRequest<Hover>>(WAIT),
        Err(StdioClientError::DeferredQueueFull)
    ));
    assert!(matches!(
        client.defer_notification(),
        Err(StdioClientError::DeferredQueueFull)
    ));
    drop(rest.remove(1));
    rest.push(client.defer_request(WAIT).unwrap());

    // Immediate requests remain independent of staged requests. The fixture
    // receives all 64 but holds their responses to fill the pending limit.
    let start = Instant::now();
    for _ in 0..64 {
        loop {
            match client.try_request(WAIT, |workspace| hover_request(workspace, URI, 0)) {
                Ok(_) => break,
                Err(StdioClientError::Transport(TransportError::QueueFull)) => {}
                Err(error) => panic!("{error}"),
            }
            assert!(
                client
                    .next_event(Duration::from_millis(2))
                    .unwrap()
                    .is_none()
            );
            assert!(start.elapsed() < WAIT);
        }
    }
    let Some(TransportEvent::Message(RpcMessage::Request { id, .. })) =
        client.next_event(WAIT).unwrap()
    else {
        panic!("expected the pending request gate");
    };
    assert!(
        client
            .try_admit_notification(&notice, |_| {
                Ok((
                    "workspace/didChangeConfiguration",
                    Some(json!({"settings":{"pending":64}})),
                ))
            })
            .unwrap()
    );
    assert!(
        client
            .try_admit_request(&token, |_| panic!("prepared without a pending slot"))
            .unwrap()
            .is_none()
    );
    client.reply(id, Ok(json!([]))).unwrap();
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Resolution(reply)) if reply.id.get() == 2
    ));
    let (id, _) = client
        .try_admit_request(&token, |workspace| hover_request(workspace, URI, 0))
        .unwrap()
        .unwrap();
    assert_eq!(id.get(), 66);
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Resolution(reply)) if reply.id == id
    ));
    // Leave the other tokens staged: shutdown must not send them.
    assert!(client.shutdown(WAIT).unwrap().success());
}

fn change(view: &mut EditorView<()>, range: std::ops::Range<usize>, text: impl Into<String>) {
    let changes = ChangeSet::single(Change::new(range, text)).unwrap();
    view.update(Transaction::new(changes), (), |_| (None, ()))
        .unwrap();
}

fn assert_between(event: TransportEvent, seen: &mut bool) {
    assert!(
        matches!(&event, TransportEvent::Message(RpcMessage::Notification { method, .. }) if method == "test/between"),
        "{event:?}"
    );
    assert!(!std::mem::replace(seen, true), "duplicate fixture event");
}

#[test]
fn managed_client_does_not_activate_after_failed_initialization() {
    let fixture = Fixture::new();
    assert!(matches!(
        StdioClient::start(&Program::new("relative"), Map::new(), WAIT),
        Err(TransportError::InvalidProgram)
    ));
    for (mode, timeout) in [
        ("reject-initialize", WAIT),
        ("invalid-initialize", WAIT),
        ("invalid-initialize", Duration::ZERO),
        ("unsupported-encoding", WAIT),
    ] {
        let mut client = fixture.start(mode, timeout);
        let LspState::Initializing(initialize) = client.state() else {
            panic!("expected initialization to be pending");
        };
        let mut view = EditorView::new(EditorState::new("a"));
        client
            .attach(&mut view, "file:///pending.rs", "rust")
            .unwrap();
        change(&mut view, 1..1, "b");
        let token = client.defer_request::<FeatureRequest<Hover>>(WAIT).unwrap();
        let mapping = client.with_workspace(|workspace| workspace.workspace_mapping());
        assert!(mapping.get_mapping("file:///pending.rs").is_some());
        assert!(
            client
                .try_admit_request(&token, |_| panic!("prepared during initialization"))
                .unwrap()
                .is_none()
        );
        let event = client.next_event(WAIT);
        match (mode, timeout, event) {
            ("reject-initialize", _, Ok(Some(TransportEvent::Resolution(response)))) => {
                assert_eq!(response.id, initialize);
                assert_eq!(response.outcome.unwrap_err().code, -32002);
            }
            (_, Duration::ZERO, Ok(Some(TransportEvent::TimedOut(id)))) => {
                assert_eq!(id, initialize)
            }
            (
                "invalid-initialize",
                _,
                Ok(Some(TransportEvent::ProtocolError(RpcError::InvalidInitializeResult))),
            ) => {}
            (
                "unsupported-encoding",
                _,
                Err(StdioClientError::Workspace(LspError::UnsupportedPositionEncoding(encoding))),
            ) => assert_eq!(encoding, "utf-8"),
            (_, _, event) => panic!("unexpected init result in {mode}: {event:?}"),
        }
        assert_eq!(
            client.state(),
            if mode == "unsupported-encoding" {
                LspState::Closed
            } else {
                LspState::Failed
            }
        );
        assert_eq!(client.server_capabilities(), None);
        assert!(mapping.get_mapping("file:///pending.rs").is_none());
        assert!(matches!(
            client.try_admit_request(&token, |_| panic!("prepared after failure")),
            Err(StdioClientError::InvalidDeferredRequest)
        ));
        assert!(matches!(
            client.defer_request::<FeatureRequest<Hover>>(WAIT),
            Err(StdioClientError::Transport(TransportError::Rpc(
                RpcError::InvalidSessionState
            )))
        ));
        if mode == "reject-initialize" {
            let mut replacement = fixture.start(mode, timeout);
            let own = replacement
                .defer_request::<FeatureRequest<Hover>>(WAIT)
                .unwrap();
            assert!(matches!(
                replacement.try_admit_request(&token, |_| panic!("replayed on another client")),
                Err(StdioClientError::InvalidDeferredRequest)
            ));
            assert!(
                replacement
                    .try_admit_request(&own, |_| panic!("prepared during initialization"))
                    .unwrap()
                    .is_none()
            );
        }
        assert!(matches!(
            client.try_flush(),
            Err(StdioClientError::Transport(TransportError::Rpc(
                RpcError::InvalidSessionState
            )))
        ));
        client.with_workspace(|workspace| {
            let file = workspace.file("file:///pending.rs").unwrap();
            assert_eq!(file.version(), 0);
            assert_eq!(file.draft().as_str(), "ab");
        });
    }
}

#[test]
fn managed_client_invalidates_feature_mappings_on_eof() {
    const URI: &str = "file:///eof.rs";
    let fixture = Fixture::new();
    let mut client = fixture.start("managed-eof", WAIT);
    let mut view = EditorView::<()>::new(EditorState::new("a"));
    client.attach(&mut view, URI, "rust").unwrap();
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Initialized)
    ));
    let token = client.defer_request::<FeatureRequest<Hover>>(WAIT).unwrap();
    let (id, hover) = client
        .try_request(WAIT, |workspace| hover_request(workspace, URI, 0))
        .unwrap();
    assert!(
        matches!(client.next_event(WAIT).unwrap(), Some(TransportEvent::Disconnected { reason: TransportError::Eof(_), pending }) if pending == [id])
    );
    assert_eq!(client.state(), LspState::Closed);
    assert!(matches!(
        client.try_admit_request(&token, |_| panic!("prepared after EOF")),
        Err(StdioClientError::InvalidDeferredRequest)
    ));
    let stale = client
        .with_workspace(|workspace| hover.resolve(workspace, &json!({"contents":"not usable"})));
    assert!(matches!(stale, Err(FeatureError::StaleResponse { .. })));
    change(&mut view, 1..1, "b");
    assert_eq!(
        client.with_workspace(|workspace| workspace.file(URI).unwrap().draft().as_str().to_owned()),
        "ab"
    );
}

#[test]
fn managed_client_stops_when_a_document_frame_cannot_be_queued() {
    let fixture = Fixture::new();
    let mut client = fixture.start("editor", WAIT);
    let mut view = EditorView::<()>::new(EditorState::new("let id"));
    client
        .attach(&mut view, "file:///editor.rs", "rust")
        .unwrap();
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Initialized)
    ));
    client.try_flush().unwrap();
    let mapping = client.with_workspace(|workspace| workspace.workspace_mapping());
    change(&mut view, 6..6, "x".repeat(8 * 1024 * 1024));
    assert!(matches!(
        client.try_flush(),
        Err(StdioClientError::Transport(TransportError::Frame(
            FrameError::BodyTooLarge
        )))
    ));
    assert_eq!(client.state(), LspState::Closed);
    assert!(mapping.get_mapping("file:///editor.rs").is_none());
}

#[test]
fn managed_client_bounds_shutdown_when_document_traffic_is_blocked() {
    let fixture = Fixture::new();
    let mut client = fixture.start("unread", WAIT);
    assert!(matches!(
        client.next_event(WAIT).unwrap(),
        Some(TransportEvent::Initialized)
    ));
    assert!(
        matches!(client.next_event(WAIT).unwrap(), Some(TransportEvent::Message(RpcMessage::Notification { method, .. })) if method == "test/not-reading")
    );
    let mut large = EditorView::<()>::new(EditorState::new("x".repeat(4 * 1024 * 1024)));
    client.attach(&mut large, "file:///large", "").unwrap();
    let views: Vec<_> = (0..8)
        .map(|i| {
            let mut view = EditorView::<()>::new(EditorState::new("a"));
            client
                .attach(&mut view, format!("file:///small/{i}"), "")
                .unwrap();
            view
        })
        .collect();
    assert!(matches!(
        client.try_flush(),
        Err(StdioClientError::Transport(TransportError::QueueFull))
    ));
    let start = Instant::now();
    assert!(matches!(
        client.shutdown(Duration::from_millis(100)),
        Err(StdioClientError::Transport(
            TransportError::ShutdownTimedOut
        ))
    ));
    assert!(start.elapsed() < WAIT);
    drop(views);
}

#[test]
fn stdio_reports_setup_and_initialization_failures() {
    assert!(matches!(
        StdioTransport::spawn(&Program::new("relative")),
        Err(TransportError::InvalidProgram)
    ));
    let fixture = Fixture::new();
    assert!(
        matches!(StdioTransport::spawn(&Program::new(fixture.0.join("missing"))), Err(TransportError::Io { operation: "start LSP process", error }) if error.kind() == io::ErrorKind::NotFound)
    );
    let mut connection = fixture.spawn("invalid-initialize");
    assert!(matches!(
        connection.request("test/early", None, WAIT),
        Err(TransportError::Rpc(RpcError::InvalidSessionState))
    ));
    connection.initialize(Map::new(), WAIT).unwrap();
    assert!(matches!(
        next(&mut connection),
        TransportEvent::ProtocolError(RpcError::InvalidInitializeResult)
    ));
    assert_eq!(connection.state(), LspState::Failed);
    assert_eq!(connection.server_capabilities(), None);

    let mut connection = fixture.spawn("invalid-initialize");
    let id = connection.initialize(Map::new(), Duration::ZERO).unwrap();
    assert!(matches!(next(&mut connection), TransportEvent::TimedOut(expired) if expired == id));
    assert_eq!(connection.state(), LspState::Failed);
}

#[test]
fn stdio_reports_pending_requests_on_eof_or_bad_framing() {
    let fixture = Fixture::new();
    for mode in ["eof", "truncated", "oversized"] {
        let mut connection = fixture.ready(mode);
        let id = connection.request("test/disconnect", None, WAIT).unwrap();
        notification(&mut connection, "test/last");
        let TransportEvent::Disconnected { reason, pending } = next(&mut connection) else {
            panic!("expected disconnect in {mode}");
        };
        assert_eq!(pending, [id]);
        match mode {
            "eof" => assert!(matches!(reason, TransportError::Eof(_)), "{reason:?}"),
            "truncated" => assert!(
                matches!(reason, TransportError::Frame(FrameError::Truncated)),
                "{reason:?}"
            ),
            "oversized" => assert!(
                matches!(reason, TransportError::Frame(FrameError::BodyTooLarge)),
                "{reason:?}"
            ),
            _ => unreachable!(),
        }
        assert_eq!(connection.state(), LspState::Closed);
        assert!(matches!(
            connection.next_event(Duration::ZERO),
            Err(TransportError::Rpc(RpcError::InvalidSessionState))
        ));
    }
}

#[test]
fn stdio_bounds_blocked_writes_and_shutdown() {
    let fixture = Fixture::new();
    let mut connection = fixture.ready("unread");
    notification(&mut connection, "test/not-reading");
    let start = Instant::now();
    let large = json!({"text":"x".repeat(4 * 1024 * 1024)});
    let id = connection
        .request("test/backpressure", Some(large), Duration::from_millis(100))
        .unwrap();
    for _ in 0..7 {
        connection.notify("test/queued", None).unwrap();
    }
    assert!(matches!(
        connection.notify("test/over-queue-limit", None),
        Err(TransportError::QueueFull)
    ));
    assert!(matches!(next(&mut connection), TransportEvent::TimedOut(expired) if expired == id));
    drop(connection);
    assert!(start.elapsed() < WAIT);

    for mode in ["shutdown-timeout", "exit-timeout"] {
        let connection = fixture.ready(mode);
        let start = Instant::now();
        assert!(matches!(
            connection.shutdown(Duration::from_millis(100)),
            Err(TransportError::ShutdownTimedOut)
        ));
        assert!(start.elapsed() < WAIT);
    }
}
