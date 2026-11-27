use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Transaction,
    change_filter,
    lsp::{
        LspClient, LspError,
        features::{FeatureError, definition_request},
        rpc::{LspEvent, LspSession, RpcMessage},
    },
    view::{EditorView, Viewport},
};
use serde_json::{Map, Value, json};

const URI: &str = "file:///view.rs";
const OTHER: &str = "file:///other.rs";

fn doc_message(method: &str, text_document: Value) -> Value {
    json!({"jsonrpc":"2.0", "method":method, "params":{"textDocument":text_document}})
}

fn open(uri: &str, text: &str, version: i32) -> Value {
    doc_message(
        "textDocument/didOpen",
        json!({"uri":uri, "languageId":"rust", "text":text, "version":version}),
    )
}

fn close(uri: &str) -> Value {
    doc_message("textDocument/didClose", json!({"uri":uri}))
}

fn change(text: &str, version: i32) -> Value {
    json!({"jsonrpc":"2.0", "method":"textDocument/didChange", "params":{
        "textDocument":{"uri":URI, "version":version}, "contentChanges":[{"text":text}]
    }})
}

fn edit(
    view: &mut EditorView<()>,
    range: std::ops::Range<usize>,
    insert: &str,
) -> Result<(), EditError> {
    let tx = Transaction::new(ChangeSet::single(Change::new(range, insert))?);
    view.update(tx, (), |_| (None, ()))
}

#[test]
fn flush_uses_only_accepted_view_changes_and_history_replays() {
    let mut client = LspClient::default();
    let source = "a字bc\r\n";
    let state = EditorState::with_extensions(
        source,
        change_filter(|_, _| ChangePermission::Protect(std::iter::once(1..4).collect())),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    client.attach(&mut view, URI, "rust").unwrap();
    assert_eq!(
        edit(&mut view, 2..3, "invalid"),
        Err(EditError::InvalidRange)
    );
    edit(&mut view, 0..5, "🦊").unwrap();
    assert_eq!(view.state().text().as_str(), "🦊字c\r\n");
    assert!(client.flush().unwrap().is_empty());
    client.with_workspace(|workspace| {
        let file = workspace.file(URI).unwrap();
        assert_eq!(file.synced().as_str(), source);
        assert_eq!(file.draft(), view.state().text());
        assert_eq!(file.version(), 0);
    });

    client
        .connected(&json!({"textDocumentSync":{"change":2}}))
        .unwrap();
    assert_eq!(
        client.flush().unwrap(),
        [open(URI, source, 0), change("🦊字c\r\n", 1)]
    );
    view.set_selection(EditorSelection::single(4, 4)).unwrap();
    view.set_viewport(Viewport::complete(view.state().text()), true)
        .unwrap();
    assert!(client.flush().unwrap().is_empty());
    assert!(view.undo().is_some());
    assert_eq!(client.flush().unwrap(), [change(source, 2)]);
    assert!(view.redo().is_some());
    assert_eq!(client.flush().unwrap(), [change("🦊字c\r\n", 3)]);
    assert!(client.flush().unwrap().is_empty());
}

#[test]
fn incremental_flush_preserves_disjoint_view_edits_and_utf16_ranges() {
    let source = "a".repeat(1024) + "😀x\r\nword";
    let mut client = LspClient::default();
    let mut view = EditorView::new(EditorState::new(&source));
    client.attach(&mut view, URI, "rust").unwrap();
    client.connected(&json!({"textDocumentSync":2})).unwrap();
    assert_eq!(client.flush().unwrap(), [open(URI, &source, 0)]);
    let changes = ChangeSet::new(vec![
        Change::new(0..0, "start "),
        Change::new(1028..1029, "y"),
        Change::new(1031..1035, "字"),
    ])
    .unwrap();
    view.update(Transaction::new(changes), (), |_| (None, ()))
        .unwrap();
    let messages = client.flush().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0]["params"],
        json!({"textDocument":{"uri":URI,"version":1},"contentChanges":[
            {"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":4}},"text":"字"},
            {"range":{"start":{"line":0,"character":1026},"end":{"line":0,"character":1027}},"text":"y"},
            {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"text":"start "}
        ]})
    );
    client.with_workspace(|workspace| {
        assert_eq!(workspace.file(URI).unwrap().synced(), view.state().text());
    });
}

#[test]
fn lifecycle_is_ordered_and_duplicate_uri_leaves_both_views_unchanged() {
    let mut client = LspClient::default();
    client.connected(&json!({"textDocumentSync":1})).unwrap();
    let mut first = EditorView::new(EditorState::new("first"));
    let mut second = EditorView::new(EditorState::new("second"));
    let plugin = client.attach(&mut first, URI, "rust").unwrap();
    assert_eq!(
        client.attach(&mut second, URI, "ignored"),
        Err(LspError::DocumentAlreadyOpen(URI.into()))
    );
    edit(&mut second, 6..6, "!").unwrap();
    client.attach(&mut second, OTHER, "rust").unwrap();
    edit(&mut first, 0..0, "discarded").unwrap();
    assert!(first.remove_plugin(plugin));
    assert!(!first.remove_plugin(plugin));
    edit(&mut first, 0.."discardedfirst".len(), "reopened").unwrap();
    client.attach(&mut first, URI, "rust").unwrap();
    drop(second);
    assert_eq!(
        client.flush().unwrap(),
        [
            open(URI, "first", 0),
            open(OTHER, "second!", 0),
            close(URI),
            open(URI, "reopened", 1),
            close(OTHER),
        ]
    );
    drop(first);
    assert_eq!(client.flush().unwrap(), [close(URI)]);
    assert!(client.with_workspace(|workspace| workspace.files().is_empty()));
}

#[test]
fn reconnect_keeps_pending_edits_and_uses_the_new_sync_policy() {
    let mut client = LspClient::default();
    let mut view = EditorView::new(EditorState::new("name"));
    client.attach(&mut view, URI, "rust").unwrap();
    client.connected(&json!({"textDocumentSync":1})).unwrap();
    assert_eq!(client.flush().unwrap(), [open(URI, "name", 0)]);
    let mut temporary = EditorView::<()>::new(EditorState::new("temporary"));
    client.attach(&mut temporary, OTHER, "rust").unwrap();
    drop(temporary);
    edit(&mut view, 0..0, "my ").unwrap();
    client.disconnected();
    edit(&mut view, 0..2, "a").unwrap();
    assert!(client.flush().unwrap().is_empty());

    client.connected(&json!({"textDocumentSync":0})).unwrap();
    assert_eq!(client.flush().unwrap(), [open(URI, "name", 0)]);
    client.with_workspace(|workspace| {
        let file = workspace.file(URI).unwrap();
        assert_eq!(file.synced().as_str(), "a name");
        assert_eq!(file.version(), 1);
        assert!(!file.has_pending_changes());
    });
    client.disconnected();
    client.connected(&json!({"textDocumentSync":1})).unwrap();
    assert_eq!(client.flush().unwrap(), [open(URI, "a name", 1)]);
}

#[test]
fn late_rpc_result_maps_pending_edits_but_not_a_new_attachment_or_session() {
    let mut rpc = LspSession::new(NonZeroUsize::new(4).unwrap());
    let (initialize, _) = rpc.initialize(Map::new()).unwrap();
    let capabilities = json!({"textDocumentSync":1, "definitionProvider":true});
    let response = |id, value| RpcMessage::Response {
        id: Some(id),
        outcome: Ok(value),
    };
    assert!(matches!(
        rpc.receive(response(
            initialize.into(),
            json!({"capabilities":capabilities})
        )),
        Ok(LspEvent::Initialized(_))
    ));
    let mut client = LspClient::default();
    let mut view = EditorView::new(EditorState::new("abc word"));
    let plugin = client.attach(&mut view, URI, "rust").unwrap();
    client
        .connected(rpc.server_capabilities().unwrap())
        .unwrap();
    assert_eq!(client.flush().unwrap(), [open(URI, "abc word", 0)]);
    let definition =
        client.with_workspace(|workspace| definition_request(workspace, URI, 5).unwrap());
    let (id, _) = rpc
        .request(definition.method(), Some(definition.params().clone()))
        .unwrap();
    edit(&mut view, 0..0, "😀 ").unwrap();
    assert!(matches!(
        client.with_workspace(|workspace| definition_request(workspace, URI, 10)),
        Err(FeatureError::UnsyncedDocument(_))
    ));
    let result = json!({"uri":URI, "range":{
        "start":{"line":0,"character":4}, "end":{"line":0,"character":8}
    }});
    let LspEvent::Resolution(reply) = rpc.receive(response(id.into(), result.clone())).unwrap()
    else {
        panic!("expected the definition response");
    };
    assert_eq!(reply.id, id);
    let result = reply.outcome.unwrap();
    assert_eq!(
        client
            .with_workspace(|workspace| definition.resolve(workspace, &result))
            .unwrap()
            .unwrap()[0]
            .byte_range,
        Some(9..13)
    );
    assert!(view.remove_plugin(plugin));
    client.attach(&mut view, URI, "rust").unwrap();
    assert_eq!(
        client.flush().unwrap(),
        [close(URI), open(URI, "😀 abc word", 1)]
    );
    assert!(matches!(
        client.with_workspace(|workspace| definition.resolve(workspace, &result)),
        Err(FeatureError::StaleResponse {
            requested: 0,
            current: Some(1),
            ..
        })
    ));

    let before_reconnect =
        client.with_workspace(|workspace| definition_request(workspace, URI, 10).unwrap());
    client.disconnected();
    assert_eq!(
        client.connected(&json!({"textDocumentSync":99})),
        Err(LspError::InvalidFormat("textDocumentSync"))
    );
    assert!(client.flush().unwrap().is_empty());
    client.connected(&capabilities).unwrap();
    assert_eq!(
        client.connected(&capabilities),
        Err(LspError::AlreadyConnected)
    );
    assert_eq!(client.flush().unwrap(), [open(URI, "😀 abc word", 1)]);
    assert!(matches!(
        client.with_workspace(|workspace| before_reconnect.resolve(workspace, &result)),
        Err(FeatureError::StaleResponse {
            requested: 1,
            current: Some(1),
            ..
        })
    ));
}
