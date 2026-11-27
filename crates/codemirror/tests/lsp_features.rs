use std::{num::NonZeroUsize, ops::Range};

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    lsp::{
        DocumentWorkspace, LspError, Position, SyncKind,
        features::{
            CompletionEditMode, CompletionTrigger, Documentation, FeatureError, FeatureRequest,
            InsertTextFormat, completion_request, definition_request, hover_request,
        },
        rpc::{LspEvent, LspSession, RequestId, RpcMessage, StdioCodec},
    },
};
use serde_json::{Map, Value, json};

const URI: &str = "file:///home/holly/src/test.js";
const TARGET: &str = "file:///home/holly/src/target.js";

fn open(text: &str) -> (DocumentWorkspace, Value) {
    let mut workspace = DocumentWorkspace::default();
    let notification = workspace
        .open_file(URI, "javascript", Text::new(text))
        .unwrap();
    (workspace, notification)
}

fn change(workspace: &mut DocumentWorkspace, uri: &str, range: Range<usize>, text: &str) {
    let edits = ChangeSet::single(Change::new(range, text)).unwrap();
    workspace.record_change(uri, &edits).unwrap();
}

fn wire_range(line: u32, start: u32, end: u32) -> Value {
    json!({"start":{"line":line, "character":start},"end":{"line":line, "character":end}})
}

// Response data from @codemirror/lsp-client 6.3.1 test/server.ts. The tarball
// digest is pinned in docs/codemirror-upstream.json. The fixture keeps the
// server's item order, snippet escapes, relative range, and added edits.
fn pinned_completion(line: u32, column: u32) -> Value {
    let range = wire_range(line, column - 1, column);
    json!({
        "isIncomplete": true,
        "itemDefaults": {"editRange": range},
        "items": [
            {"label":"one", "kind":14, "commitCharacters":["."], "textEdit":{"newText":"one!", "range":range}},
            {"label":"ookay", "kind":7, "documentation":"`code` stuff", "insertText":"okay"},
            {"label":"fn", "kind":3, "insertText":"fn(${1:\\$arg})$0", "insertTextFormat":2},
            {"label":"Bar", "kind":7, "insertText":"Bar", "additionalTextEdits":[
                {"range":wire_range(0, 0, 0), "newText":"use Bar;\n"}
            ]},
            {"label":"Zap", "kind":7, "insertText":"Zap", "additionalTextEdits":[
                {"range":wire_range(2, 0, 0), "newText":"MID\n"}
            ]}
        ]
    })
}

fn exchange(value: Value) -> RpcMessage {
    let mut codec = StdioCodec::new(128, 8192).unwrap();
    let frame = codec.encode(&value).unwrap();
    let mut message = None;
    for chunk in frame.chunks(5) {
        codec
            .feed(chunk, |body| {
                assert!(message.is_none());
                message = Some(RpcMessage::from_json(body).unwrap());
            })
            .unwrap();
    }
    codec.finish().unwrap();
    message.unwrap()
}

fn submit<T>(session: &mut LspSession, request: &FeatureRequest<T>, params: Value) -> RequestId {
    let (id, message) = session
        .request(request.method(), Some(request.params().clone()))
        .unwrap();
    assert_eq!(
        Value::from(exchange(message.into())),
        json!({
            "jsonrpc":"2.0", "id":id.get(), "method":request.method(), "params":params
        })
    );
    id
}

fn resolve<T>(
    session: &mut LspSession,
    workspace: &DocumentWorkspace,
    request: &FeatureRequest<T>,
    id: RequestId,
    response: Value,
) -> T {
    let LspEvent::Resolution(reply) = session
        .receive(exchange(json!({
            "jsonrpc":"2.0", "id":id.get(), "result":response
        })))
        .unwrap()
    else {
        panic!("expected a feature reply")
    };
    assert_eq!(reply.id, id);
    assert!(!reply.cancel_requested);
    request
        .resolve(workspace, &reply.outcome.unwrap())
        .unwrap()
        .unwrap()
}

#[test]
fn completion_reply_and_plan_stay_with_the_request_workspace() {
    let (workspace, _) = open("foo");
    let (other, _) = open("bar");
    let request = completion_request(&workspace, URI, 3, CompletionTrigger::Invoked).unwrap();
    let reply = json!([{"label": "example"}]);
    assert!(matches!(
        request.resolve(&other, &reply),
        Err(FeatureError::StaleResponse { .. })
    ));
    let list = request.resolve(&workspace, &reply).unwrap().unwrap();
    assert!(matches!(
        list.plan(&other, 0, CompletionEditMode::Replace),
        Err(FeatureError::StaleResponse { .. })
    ));
    assert!(
        list.plan(&workspace, 0, CompletionEditMode::Replace)
            .is_ok()
    );
}

#[test]
fn pinned_hover_and_completion_fixtures_round_trip_in_local_framed_rpc() {
    let mut session = LspSession::new(NonZeroUsize::new(3).unwrap());
    let (init, outbound) = session.initialize(Map::new()).unwrap();
    assert_eq!(
        Value::from(exchange(outbound.into()))["method"],
        "initialize"
    );
    let capabilities = json!({
        "textDocumentSync":{"openClose":true,"change":2},
        "renameProvider":true,"documentFormattingProvider":true,
        "completionProvider":{"triggerCharacters":[","]},"hoverProvider":true,
        "signatureHelpProvider":{}
    });
    let peer = exchange(json!({"jsonrpc":"2.0", "id":init.get(), "result":{
        "capabilities":capabilities,"serverInfo":{"name":"Dummy server"}
    }}));
    let LspEvent::Initialized(initialized) = session.receive(peer).unwrap() else {
        panic!("expected initialization notification");
    };
    assert_eq!(
        Value::from(exchange(initialized.into())),
        json!({
            "jsonrpc":"2.0","method":"initialized","params":{}
        })
    );

    let source = "speedwell\r\n😀..o";
    let (mut workspace, opened) = open(source);
    assert_eq!(
        Value::from(exchange(opened))["params"]["textDocument"]["text"],
        source
    );
    let hover = hover_request(&workspace, URI, 1).unwrap();
    let trigger =
        CompletionTrigger::automatic('o', session.server_capabilities().unwrap()).unwrap();
    let completion = completion_request(&workspace, URI, source.len(), trigger).unwrap();
    assert_eq!(hover.method(), "textDocument/hover");
    assert_eq!(hover.capability(), "hoverProvider");
    assert_eq!(hover.version(), 0);
    assert_eq!(completion.method(), "textDocument/completion");
    assert_eq!(completion.capability(), "completionProvider");
    let hover_id = submit(
        &mut session,
        &hover,
        json!({
            "textDocument":{"uri":URI},"position":{"line":0,"character":1}
        }),
    );
    let completion_id = submit(
        &mut session,
        &completion,
        json!({
            "textDocument":{"uri":URI},"position":{"line":1,"character":5},"context":{"triggerKind":1}
        }),
    );
    let list = resolve(
        &mut session,
        &workspace,
        &completion,
        completion_id,
        pinned_completion(1, 5),
    );
    let tooltip = resolve(
        &mut session,
        &workspace,
        &hover,
        hover_id,
        json!({
            "range": wire_range(0, 1, 1), "contents":{"language":"javascript","value":"'hover'"}
        }),
    );
    assert_eq!(tooltip.range, 1..1);
    assert_eq!(
        tooltip.contents,
        [Documentation::Code {
            language: "javascript".into(),
            value: "'hover'".into()
        }]
    );
    assert_eq!(list.range, source.len() - 1..source.len());
    assert!(list.is_incomplete);
    assert_eq!(
        list.items()
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["one", "ookay", "fn", "Bar", "Zap"]
    );
    assert_eq!(list.items()[0].kind, Some(14));
    assert_eq!(list.items()[0].commit_characters, Some(vec![".".into()]));
    assert_eq!(
        list.items()[1].documentation,
        Some(Documentation::PlainText("`code` stuff".into()))
    );
    assert_eq!(list.items()[1].insert_text, "okay");
    assert_eq!(list.items()[2].insert_text, "fn(${1:\\$arg})$0");
    assert_eq!(
        list.items()[2].insert_text_format,
        InsertTextFormat::Snippet
    );
    assert_eq!(
        list.plan(&workspace, 2, CompletionEditMode::Insert),
        Err(FeatureError::Unsupported("completion snippet expansion"))
    );
    assert_eq!(
        list.plan(&workspace, 4, CompletionEditMode::Insert)
            .unwrap()
            .invalid_additional_edits,
        1
    );
    let mut imported = Text::new(source);
    list.plan(&workspace, 3, CompletionEditMode::Insert)
        .unwrap()
        .changes
        .apply_to(&mut imported)
        .unwrap();
    assert_eq!(imported.as_str(), "use Bar;\nspeedwell\r\n😀..Bar");
    let plan = list
        .plan(&workspace, 0, CompletionEditMode::Insert)
        .unwrap();
    workspace.record_change(URI, &plan.changes).unwrap();
    assert_eq!(
        workspace.file(URI).unwrap().draft().as_str(),
        "speedwell\r\n😀..one!"
    );
}

#[test]
fn completion_maps_the_pinned_additional_edit_before_and_after_response() {
    let (mut workspace, _) = open("L0\nL1\nL2\n..z");
    let request = completion_request(&workspace, URI, 12, CompletionTrigger::Invoked).unwrap();
    change(&mut workspace, URI, 0..0, "HEADER\n");
    workspace.sync(SyncKind::Incremental).unwrap();
    let list = request
        .resolve(&workspace, &pinned_completion(3, 3))
        .unwrap()
        .unwrap();
    assert_eq!(list.range, 18..19);
    change(&mut workspace, URI, 0..0, "😀");
    let plan = list
        .plan(&workspace, 4, CompletionEditMode::Insert)
        .unwrap();
    assert_eq!(
        plan.changes.changes().collect::<Vec<_>>(),
        [(&(17..17), "MID\n"), (&(22..23), "Zap")]
    );
    let mut accepted = workspace.file(URI).unwrap().draft().clone();
    plan.changes.apply_to(&mut accepted).unwrap();
    assert_eq!(accepted.as_str(), "😀HEADER\nL0\nL1\nMID\nL2\n..Zap");

    change(&mut workspace, URI, 17..18, "M");
    let plan = list
        .plan(&workspace, 4, CompletionEditMode::Insert)
        .unwrap();
    assert_eq!(plan.stale_additional_edits, 1);
    assert_eq!(
        plan.changes.changes().collect::<Vec<_>>(),
        [(&(22..23), "Zap")]
    );
    change(&mut workspace, URI, 22..23, "x");
    let stale = FeatureError::StaleResponse {
        uri: URI.into(),
        requested: 0,
        current: Some(1),
    };
    assert_eq!(
        list.plan(&workspace, 4, CompletionEditMode::Insert),
        Err(stale.clone())
    );
    assert_eq!(
        request
            .resolve(&workspace, &pinned_completion(3, 3))
            .unwrap_err(),
        stale
    );
}

#[test]
fn hover_treats_documentation_as_data_and_rejects_stale_or_inexact_ranges() {
    let (mut workspace, _) = open("a😀b\r\nalpha\rβ");
    for offset in [2, 7, 100] {
        assert_eq!(
            hover_request(&workspace, URI, offset).unwrap_err(),
            LspError::InvalidByteOffset(offset).into()
        );
    }
    let hover = hover_request(&workspace, URI, 14).unwrap();
    assert_eq!(hover.params()["position"], json!({"line":2,"character":0}));
    let markdown = "<img src=x onerror=bad()> [x](javascript:bad())";
    let response = json!({"contents":[
        markdown,
        {"kind":"plaintext","value":"<literal>"},
        {"kind":"markdown","value":"# heading"},
        {"language":"js","value":"<code>"}
    ]});
    let result = hover.resolve(&workspace, &response).unwrap().unwrap();
    assert_eq!(result.range, 14..14);
    assert_eq!(
        result.contents,
        [
            Documentation::Markdown(markdown.into()),
            Documentation::PlainText("<literal>".into()),
            Documentation::Markdown("# heading".into()),
            Documentation::Code {
                language: "js".into(),
                value: "<code>".into()
            }
        ]
    );
    assert_eq!(hover.resolve(&workspace, &Value::Null), Ok(None));
    let spanning = json!({"contents":"text","range":{
        "start":{"line":0,"character":1},"end":{"line":1,"character":2}
    }});
    assert_eq!(
        hover.resolve(&workspace, &spanning).unwrap().unwrap().range,
        1..10
    );
    assert_eq!(
        hover
            .resolve(
                &workspace,
                &json!({"contents":"foo","range":wire_range(0,2,3)})
            )
            .unwrap_err(),
        LspError::InvalidPosition(Position {
            line: 0,
            character: 2
        })
        .into()
    );
    assert!(
        hover
            .resolve(
                &workspace,
                &json!({"contents":{"kind":"html","value":"<script>"}})
            )
            .is_err()
    );

    change(&mut workspace, URI, 0..0, "x");
    assert_eq!(
        hover_request(&workspace, URI, 15).unwrap_err(),
        FeatureError::UnsyncedDocument(URI.into())
    );
    assert_eq!(
        hover.resolve(&workspace, &response).unwrap_err(),
        FeatureError::StaleResponse {
            uri: URI.into(),
            requested: 0,
            current: Some(0)
        }
    );
    workspace.sync(SyncKind::Full).unwrap();
    let fresh = hover_request(&workspace, URI, 15).unwrap();
    assert_eq!(fresh.version(), 1);
    workspace.close_file(URI).unwrap();
    workspace
        .open_file(URI, "javascript", Text::new("xa😀b\r\nalpha\rβ"))
        .unwrap();
    assert_eq!(
        fresh.resolve(&workspace, &response).unwrap_err(),
        FeatureError::StaleResponse {
            uri: URI.into(),
            requested: 1,
            current: Some(2)
        }
    );
}

#[test]
fn definition_maps_open_snapshots_but_leaves_unopened_uris_inert() {
    let (mut workspace, _) = open("head\r\n😀name");
    workspace
        .open_file(TARGET, "javascript", Text::new("// doc\r\n😀let name"))
        .unwrap();
    let request = definition_request(&workspace, URI, 11).unwrap();
    assert_eq!(request.method(), "textDocument/definition");
    assert_eq!(request.capability(), "definitionProvider");
    assert_eq!(
        request.params(),
        &json!({"textDocument":{"uri":URI},"position":{"line":1,"character":3}})
    );
    let one = json!({"uri":TARGET,"range":wire_range(1,6,10)});
    let closed_uri = "command:never-run";
    let other = json!({"uri":closed_uri,"range":wire_range(10,3,5)});
    assert_eq!(
        request.resolve(&workspace, &one).unwrap().unwrap()[0].byte_range,
        Some(16..20)
    );
    change(&mut workspace, URI, 0..0, "\n");
    change(&mut workspace, TARGET, 0..0, "😀\r\n");
    workspace.sync(SyncKind::Full).unwrap();
    change(&mut workspace, TARGET, 6..6, "!");
    let response = request
        .resolve(&workspace, &json!([one, other]))
        .unwrap()
        .unwrap();
    assert_eq!(response[0].uri, TARGET);
    assert_eq!(
        response[0].range.start,
        Position {
            line: 1,
            character: 6
        }
    );
    assert_eq!(response[0].byte_range, Some(23..27));
    assert_eq!(response[1].uri, closed_uri);
    assert_eq!(response[1].byte_range, None);
    assert!(workspace.file(closed_uri).is_none());
    assert!(
        request
            .resolve(
                &workspace,
                &json!({"uri":closed_uri,"range":wire_range(0,4,1)})
            )
            .is_err()
    );
    assert!(
        request
            .resolve(
                &workspace,
                &json!({"targetUri":TARGET,"targetRange":wire_range(1,6,10)})
            )
            .is_err()
    );
    change(&mut workspace, TARGET, 24..25, "X");
    assert_eq!(
        request.resolve(&workspace, &one).unwrap_err(),
        FeatureError::StaleResponse {
            uri: TARGET.into(),
            requested: 0,
            current: Some(1)
        }
    );
    workspace.close_file(TARGET).unwrap();
    workspace
        .open_file(TARGET, "javascript", Text::new("// doc\r\n😀let name"))
        .unwrap();
    assert_eq!(
        request.resolve(&workspace, &one).unwrap_err(),
        FeatureError::StaleResponse {
            uri: TARGET.into(),
            requested: 0,
            current: Some(2)
        }
    );
    change(&mut workspace, URI, 11..12, "X");
    assert_eq!(
        request.resolve(&workspace, &other).unwrap_err(),
        FeatureError::StaleResponse {
            uri: URI.into(),
            requested: 0,
            current: Some(1)
        }
    );
}

#[test]
fn completion_uses_exact_item_ranges_and_rejects_overlapping_edits() {
    let source = "x\r\n😀word";
    let (workspace, _) = open(source);
    let triggers = json!({"completionProvider":{"triggerCharacters":[",", "}"]}});
    assert_eq!(
        CompletionTrigger::automatic(',', &triggers),
        Some(CompletionTrigger::Character(','))
    );
    assert_eq!(
        CompletionTrigger::automatic('_', &triggers),
        Some(CompletionTrigger::Invoked)
    );
    assert_eq!(CompletionTrigger::automatic('7', &triggers), None);
    let request =
        completion_request(&workspace, URI, 9, CompletionTrigger::Character(',')).unwrap();
    assert_eq!(
        request.params()["context"],
        json!({"triggerKind":2,"triggerCharacter":","})
    );
    assert_eq!(
        completion_request(&workspace, URI, 9, CompletionTrigger::Incomplete)
            .unwrap()
            .params()["context"],
        json!({"triggerKind":3})
    );
    let response = json!({"isIncomplete":false,"itemDefaults":{
        "editRange":{"insert":wire_range(1,2,4),"replace":wire_range(1,2,6)},
        "commitCharacters":[";"],"insertTextFormat":1
    }, "items":[
        {"label":"Display","filterText":"match","sortText":"001","detail":"metadata","textEditText":"物","insertText":"unused","command":{"command":"never-run","title":"ignored"}},
        {"label":"delete","textEdit":{"range":wire_range(1,3,5),"newText":""}},
        {"label":"conflict","additionalTextEdits":[{"range":wire_range(1,2,4),"newText":"invalid"}]}
    ]});
    let list = request.resolve(&workspace, &response).unwrap().unwrap();
    assert!(!list.is_incomplete);
    assert_eq!(list.range, 7..9);
    assert_eq!(list.commit_characters, Some(vec![";".into()]));
    assert_eq!(list.items()[0].filter_text.as_deref(), Some("match"));
    assert_eq!(list.items()[0].sort_text.as_deref(), Some("001"));
    assert_eq!(list.items()[0].detail.as_deref(), Some("metadata"));
    for (item, mode, expected) in [
        (0, CompletionEditMode::Insert, "x\r\n😀物rd"),
        (0, CompletionEditMode::Replace, "x\r\n😀物"),
        (1, CompletionEditMode::Replace, "x\r\n😀wd"),
    ] {
        let mut applied = Text::new(source);
        list.plan(&workspace, item, mode)
            .unwrap()
            .changes
            .apply_to(&mut applied)
            .unwrap();
        assert_eq!(applied.as_str(), expected);
    }
    assert_eq!(
        list.plan(&workspace, 2, CompletionEditMode::Insert),
        Err(LspError::OverlappingEdits.into())
    );
    let inside_surrogate =
        json!([{"label":"bad","textEdit":{"range":wire_range(1,1,4),"newText":"bad"}}]);
    assert_eq!(
        request.resolve(&workspace, &inside_surrogate).unwrap_err(),
        LspError::InvalidPosition(Position {
            line: 1,
            character: 1
        })
        .into()
    );
    let array = request
        .resolve(&workspace, &json!([{"label":"word"}]))
        .unwrap()
        .unwrap();
    assert_eq!(array.range, 7..11);
    assert!(request.resolve(&workspace, &Value::Null).unwrap().is_none());
}
