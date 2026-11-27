use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    lsp::{
        DocumentChanges, DocumentSnapshot, DocumentWorkspace, FormattingOptions, LspError,
        SyncKind, WorkspaceEdit, features::FeatureError, formatting_request, rename_request,
        workspace_edit,
    },
};
use serde_json::{Value, json};

const A: &str = "untouched:%2Fa";
const B: &str = "file:///2";
const UNKNOWN: &str = "untouched:/a";
const TABS: FormattingOptions = FormattingOptions {
    tab_size: 8,
    insert_spaces: false,
};

fn open(workspace: &mut DocumentWorkspace, uri: &str, text: &str) {
    workspace
        .open_file(uri, "javascript", Text::new(text))
        .unwrap();
}

fn change(workspace: &mut DocumentWorkspace, uri: &str, at: usize, text: &str) {
    workspace
        .record_change(uri, &ChangeSet::single(Change::new(at..at, text)).unwrap())
        .unwrap();
}

fn wire_edit(from: u32, to: u32, new_text: &str) -> Value {
    json!({
        "range": { "start": {"line": 0, "character": from}, "end": {"line": 0, "character": to} },
        "newText": new_text,
    })
}

fn batch(uri: &str, version: Option<i32>, edits: &[Value]) -> Value {
    json!({"textDocument": {"uri": uri, "version": version}, "edits": edits})
}

fn planned_text(workspace: &DocumentWorkspace, edit: &DocumentChanges) -> Text {
    let mut text = workspace.file(&edit.uri).unwrap().draft().clone();
    edit.changes.apply_to(&mut text).unwrap();
    text
}

// Pinned 6.3.1 dist/index.js with test/server.ts, executed in jsdom. The
// upstream command gets an indentation width of 8 from two tabs of width 4.
#[test]
fn edit_replies_cannot_plan_edits_in_a_different_workspace() {
    let mut workspace = DocumentWorkspace::default();
    let mut other = DocumentWorkspace::default();
    open(&mut workspace, A, "foo");
    open(&mut other, A, "bar");
    let formatting = formatting_request(&workspace, A, TABS).unwrap();
    let rename = rename_request(&workspace, A, 1, "new").unwrap();
    let edit = wire_edit(0, 3, "new");
    assert!(matches!(
        formatting.resolve(&other, &json!([edit])),
        Err(FeatureError::StaleResponse { .. })
    ));
    assert!(matches!(
        rename.resolve(&other, &json!({"changes": {A: [edit]}})),
        Err(FeatureError::StaleResponse { .. })
    ));
    assert_eq!(other.file(A).unwrap().draft().as_str(), "bar");
}

#[test]
fn formatting_matches_upstream_request_and_replacement() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "hawthorn");
    let request = formatting_request(&workspace, A, TABS).unwrap();
    assert_eq!(request.version(), 0);
    assert_eq!(request.method(), "textDocument/formatting");
    assert_eq!(request.capability(), "documentFormattingProvider");
    assert_eq!(
        *request.params(),
        json!({"textDocument": {"uri": A}, "options": {"tabSize": 8, "insertSpaces": false}})
    );
    assert_eq!(request.resolve(&workspace, &Value::Null), Ok(None));
    assert!(
        request
            .resolve(&workspace, &json!([]))
            .unwrap()
            .unwrap()
            .changes
            .is_empty()
    );
    assert_eq!(
        request.resolve(&workspace, &json!({"edits": []})),
        Err(LspError::InvalidFormat("TextEdit[]").into())
    );
    let edit = request
        .resolve(&workspace, &json!([wire_edit(8, 8, "\n// formatted!")]))
        .unwrap()
        .unwrap();
    assert_eq!(
        planned_text(&workspace, &edit).as_str(),
        "hawthorn\n// formatted!"
    );
    assert_eq!(workspace.file(A).unwrap().draft().as_str(), "hawthorn");
}

#[test]
fn formatting_maps_once_and_rejects_touched_batches_or_closed_sources() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "abcdefg");
    let request = formatting_request(&workspace, A, TABS).unwrap();
    change(&mut workspace, A, 0, "!");
    assert!(matches!(
        formatting_request(&workspace, A, TABS),
        Err(FeatureError::UnsyncedDocument(uri)) if uri == A
    ));
    let response = json!([wire_edit(2, 4, "XY")]);
    let edit = request.resolve(&workspace, &response).unwrap().unwrap();
    // The pinned command maps twice and gets "!abcXYfg" in this case.
    assert_eq!(planned_text(&workspace, &edit).as_str(), "!abXYefg");

    change(&mut workspace, A, 4, "?");
    let result = json!([wire_edit(5, 7, "OK"), wire_edit(2, 4, "XY")]);
    assert_eq!(
        request.resolve(&workspace, &result),
        Err(FeatureError::StaleResponse {
            uri: A.into(),
            requested: 0,
            current: Some(0)
        })
    );
    assert_eq!(workspace.file(A).unwrap().draft().as_str(), "!abc?defg");
    workspace.close_file(A);
    open(&mut workspace, A, "abcdefg");
    assert_eq!(
        request.resolve(&workspace, &response),
        Err(FeatureError::StaleResponse {
            uri: A.into(),
            requested: 0,
            current: Some(1)
        })
    );
}

// Pinned 6.3.1 webtest-client.ts rename with the UTF-16 target and delayed
// source prefix also exercised by the reference process.
#[test]
fn rename_matches_upstream_wire_and_maps_unrelated_changes() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "let foo = 1; console.log(foo)");
    open(&mut workspace, B, "😀foo?");
    let request = rename_request(&workspace, A, 4, "bar").unwrap();
    assert_eq!(request.version(), 0);
    assert_eq!(request.method(), "textDocument/rename");
    assert_eq!(request.capability(), "renameProvider");
    assert_eq!(
        *request.params(),
        json!({"textDocument": {"uri": A}, "position": {"line": 0, "character": 4}, "newName": "bar"})
    );
    assert_eq!(request.resolve(&workspace, &Value::Null), Ok(None));
    change(&mut workspace, A, 0, "  ");
    workspace.sync(SyncKind::Incremental).unwrap();
    let response = json!({"changes": {
        (A): [wire_edit(4, 7, "bar"), wire_edit(25, 28, "bar")],
        (B): [wire_edit(2, 5, "bar")],
    }});
    let plan = request.resolve(&workspace, &response).unwrap().unwrap();
    for (uri, expected) in [(A, "  let bar = 1; console.log(bar)"), (B, "😀bar?")] {
        let edit = plan.iter().find(|edit| edit.uri == uri).unwrap();
        assert_eq!(planned_text(&workspace, edit).as_str(), expected);
    }
    assert_eq!(workspace.file(B).unwrap().draft().as_str(), "😀foo?");
}

#[test]
fn rename_request_uses_host_position_and_checks_utf8_and_sync() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "😀α\r\nfoo");
    let request = rename_request(&workspace, A, 4, "new").unwrap();
    assert_eq!(
        request.params()["position"],
        json!({"line":0,"character":2})
    );
    let request = rename_request(&workspace, A, 9, "new").unwrap();
    assert_eq!(
        request.params()["position"],
        json!({"line":1,"character":1})
    );
    for byte in [2, 7] {
        assert!(matches!(
            rename_request(&workspace, A, byte, "new"),
            Err(FeatureError::Protocol(LspError::InvalidByteOffset(at))) if at == byte
        ));
    }
    change(&mut workspace, A, 0, "!");
    assert!(matches!(
        rename_request(&workspace, A, 0, "new"),
        Err(FeatureError::UnsyncedDocument(uri)) if uri == A
    ));
}

#[test]
fn rename_precedence_versions_and_versionless_rebasing() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "foo");
    open(&mut workspace, B, "xyz foo");
    let request = rename_request(&workspace, A, 0, "bar").unwrap();
    let mut response = json!({
        "changes": { (UNKNOWN): [wire_edit(0, 3, "ignored")] },
        "documentChanges": [
            batch(B, Some(0), &[wire_edit(4, 7, "bar")]),
            batch(A, None, &[wire_edit(0, 3, "bar")]),
        ],
    });
    let plan = request.resolve(&workspace, &response).unwrap().unwrap();
    assert_eq!(
        plan.iter()
            .map(|entry| entry.uri.as_str())
            .collect::<Vec<_>>(),
        [B, A]
    );
    assert_eq!(planned_text(&workspace, &plan[0]).as_str(), "xyz bar");
    assert_eq!(planned_text(&workspace, &plan[1]).as_str(), "bar");

    change(&mut workspace, B, 0, "!");
    assert_eq!(
        request.resolve(&workspace, &response),
        Err(FeatureError::StaleResponse {
            uri: B.into(),
            requested: 0,
            current: Some(0)
        })
    );
    workspace.sync(SyncKind::Full).unwrap();
    let id = |version| json!({"uri": B, "version": version});
    assert_eq!(
        request.resolve(&workspace, &response),
        Err(LspError::VersionMismatch {
            uri: B.into(),
            requested: 0,
            snapshot: Some(1)
        }
        .into())
    );
    response["documentChanges"][0]["textDocument"] = id(Some(1));
    assert_eq!(
        request.resolve(&workspace, &response),
        Err(LspError::VersionMismatch {
            uri: B.into(),
            requested: 1,
            snapshot: Some(0)
        }
        .into())
    );
    response["documentChanges"][0]["textDocument"] = id(None);
    let plan = request.resolve(&workspace, &response).unwrap().unwrap();
    assert_eq!(planned_text(&workspace, &plan[0]).as_str(), "!xyz bar");
}

#[test]
fn rename_rejects_unknown_late_and_stale_targets_as_one_batch() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "foo");
    open(&mut workspace, B, "foo");
    let request = rename_request(&workspace, A, 0, "bar").unwrap();
    let response = json!({"changes": {
        (A): [wire_edit(0, 3, "bar")],
        (UNKNOWN): [],
    }});
    let reject_unknown = |workspace: &DocumentWorkspace| {
        assert_eq!(
            request.resolve(workspace, &response),
            Err(LspError::MissingDocument(UNKNOWN.into()).into())
        );
    };
    reject_unknown(&workspace);
    open(&mut workspace, UNKNOWN, "foo");
    reject_unknown(&workspace);
    let response =
        json!({"changes": { (A): [wire_edit(0, 3, "bar")], (B): [wire_edit(0, 3, "bar")] }});
    change(&mut workspace, B, 1, "!");
    assert_eq!(
        request.resolve(&workspace, &response),
        Err(FeatureError::StaleResponse {
            uri: B.into(),
            requested: 0,
            current: Some(0)
        })
    );
    workspace.close_file(B);
    assert_eq!(
        request.resolve(&workspace, &response),
        Err(FeatureError::StaleResponse {
            uri: B.into(),
            requested: 0,
            current: None
        })
    );
    assert_eq!(workspace.file(A).unwrap().draft().as_str(), "foo");
}

#[test]
fn rename_rejects_a_touched_source_word_even_when_not_edited_by_reply() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, A, "foo");
    open(&mut workspace, B, "foo");
    let request = rename_request(&workspace, A, 0, "bar").unwrap();
    change(&mut workspace, A, 2, "!");
    assert_eq!(
        request.resolve(
            &workspace,
            &json!({"changes": {(B): [wire_edit(0, 3, "bar")]}})
        ),
        Err(FeatureError::StaleResponse {
            uri: A.into(),
            requested: 0,
            current: Some(0)
        })
    );
}

#[test]
fn workspace_edit_decodes_the_whole_batch_before_snapshot_lookup() {
    let known = batch(A, None, &[wire_edit(0, 1, "y")]);
    let mut annotated = wire_edit(0, 1, "y");
    annotated["annotationId"] = json!("confirmation");
    for (bad, error) in [
        (
            json!({"kind": "create", "uri": B}),
            LspError::UnsupportedWorkspaceEdit("resource operations"),
        ),
        (
            json!({"kind": "rename", "oldUri": A, "newUri": B}),
            LspError::UnsupportedWorkspaceEdit("resource operations"),
        ),
        (
            json!({"kind": "delete", "uri": B}),
            LspError::UnsupportedWorkspaceEdit("resource operations"),
        ),
        (
            batch(B, None, &[annotated]),
            LspError::UnsupportedWorkspaceEdit("annotated text edits"),
        ),
        (known.clone(), LspError::RepeatedDocument(A.into())),
    ] {
        let edit = json!({"documentChanges": [known, bad]});
        assert_eq!(
            workspace_edit(&edit, |_| panic!("unexpected snapshot lookup")),
            Err(error)
        );
    }
    let source = Text::new("x");
    let edit = WorkspaceEdit::try_from(
        &json!({"documentChanges": [batch(A, Some(1), &[wire_edit(0, 1, "y")])]}),
    )
    .unwrap();
    assert_eq!(
        edit.plan(|_| Some(DocumentSnapshot {
            text: &source,
            version: None
        })),
        Err(LspError::VersionMismatch {
            uri: A.into(),
            requested: 1,
            snapshot: None
        })
    );
    assert_eq!(
        workspace_edit(&json!({"documentChanges": null, "changes": {}}), |_| None),
        Err(LspError::InvalidFormat("WorkspaceEdit"))
    );
}
