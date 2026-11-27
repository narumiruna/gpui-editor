use std::{num::NonZeroUsize, time::Duration};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Text,
    lint::{LintState, Severity, diagnostic_count, set_diagnostics_for},
    lsp::{
        DocumentWorkspace, LanguageServerCommand as Command, LspError, SyncKind,
        features::{
            FeatureError, declaration_request, definition_request, implementation_request,
            type_definition_request,
        },
        language_server_extensions, language_server_support,
        rpc::{LspEvent, LspSession, RpcId, RpcMessage},
        server_diagnostics,
        signature::SignatureCommand,
    },
};
use serde_json::{Map, Value, json};

const PUBLISH: &str = "textDocument/publishDiagnostics";

// The fixture comes from the manifest-verified dist/index.js. A scratch jsdom
// editor and an in-memory server exercised the exported bundles and commands.
fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/lsp_support.json")).unwrap()
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn notification(params: Value) -> RpcMessage {
    RpcMessage::notification(PUBLISH, Some(params)).unwrap()
}

fn change(workspace: &mut DocumentWorkspace, uri: &str, change: Change) {
    workspace
        .record_change(uri, &ChangeSet::single(change).unwrap())
        .unwrap();
}

#[test]
fn bundles_supply_pinned_keys_and_native_initialize_capabilities() {
    let oracle = oracle();
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let exports: Value = serde_json::from_str(include_str!(
        "../../../docs/codemirror-upstream-exports.json"
    ))
    .unwrap();
    let package = |data: Value| {
        data["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "@codemirror/lsp-client")
            .unwrap()
            .clone()
    };
    let pin = package(manifest);
    assert_eq!(
        oracle["package"],
        format!("@codemirror/lsp-client@{}", string(&pin["version"]))
    );
    assert_eq!(oracle["integrity"], pin["integrity"]);
    assert_eq!(oracle["exports"], package(exports)["modules"][0]["exports"]);

    let composition = &oracle["composition"];
    let mut workspace = DocumentWorkspace::default();
    let source = string(&oracle["source"]);
    let opened = &composition["opened"];
    let uri = string(&opened["textDocument"]["uri"]);
    let language_id = string(&opened["textDocument"]["languageId"]);
    let (legacy, did_open) =
        language_server_support(&mut workspace, uri, language_id, Text::new(source)).unwrap();
    let mut expected_open = opened.clone();
    // The native workspace, unlike the browser text model, retains CRLF.
    expected_open["textDocument"]["text"] = json!(source);
    assert_eq!(
        did_open,
        json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": expected_open})
    );
    assert_eq!(
        language_server_support(&mut workspace, uri, language_id, Text::new(source)),
        Err(LspError::DocumentAlreadyOpen(uri.into()))
    );

    let current = language_server_extensions();
    let mut expected_caps = composition["capabilities"].as_object().unwrap().clone();
    expected_caps.remove("general");
    expected_caps.remove("window");
    expected_caps.get_mut("textDocument").unwrap()["completion"]["completionItem"]["snippetSupport"] =
        json!(false);
    let mut session = LspSession::new(NonZeroUsize::MIN);
    let (_, wire) = session
        .initialize(Map::from_iter([(
            "capabilities".into(),
            current.client_capabilities(),
        )]))
        .unwrap();
    assert_eq!(
        Value::from(wire)["params"]["capabilities"],
        json!(expected_caps)
    );
    expected_caps
        .get_mut("textDocument")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("publishDiagnostics");
    assert_eq!(legacy.client_capabilities(), json!(expected_caps));
    assert_eq!(
        current.diagnostics_sync_delay(),
        composition["diagnosticSyncDelayMs"]
            .as_u64()
            .map(Duration::from_millis)
    );
    assert_eq!(legacy.diagnostics_sync_delay(), None);

    for bundle in [current, legacy] {
        let keymap = bundle.keymap();
        assert_eq!(
            json!(
                keymap
                    .iter()
                    .map(|binding| (&binding.key, binding.prevent_default))
                    .collect::<Vec<_>>()
            ),
            composition["keys"]
        );
        assert_eq!(
            keymap
                .into_iter()
                .map(|binding| binding.run.unwrap())
                .collect::<Vec<_>>(),
            [
                Command::Signature(SignatureCommand::Show),
                Command::Signature(SignatureCommand::Previous),
                Command::Signature(SignatureCommand::Next),
                Command::FormatDocument,
                Command::RenameSymbol,
                Command::JumpToDefinition,
                Command::FindReferences,
                Command::CloseReferencePanel,
            ]
        );
    }
    let publication = notification(json!({"uri": uri, "version": 0, "diagnostics": []}));
    assert!(
        legacy
            .receive_notification(&workspace, &publication)
            .unwrap()
            .is_none()
    );
    assert!(
        current
            .receive_notification(&workspace, &publication)
            .unwrap()
            .is_some()
    );
}

#[test]
fn diagnostic_notification_matches_upstream_mapping_and_replaces_native_lint() {
    let oracle = oracle();
    let data = &oracle["diagnostics"];
    let uri = string(&data["params"]["uri"]);
    let mut workspace = DocumentWorkspace::default();
    let source = string(&oracle["source"]);
    workspace.open_file(uri, "rust", Text::new(source)).unwrap();
    let pending = data["pending"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edit| Change::new(offset(&edit[0])..offset(&edit[1]), string(&edit[2])));
    workspace
        .record_change(uri, &ChangeSet::new(pending.collect()).unwrap())
        .unwrap();
    let file = workspace.file(uri).unwrap();
    assert!(file.has_pending_changes());
    assert_eq!(file.synced().as_str(), source);

    let mut session = LspSession::new(NonZeroUsize::MIN);
    let (initialize, _) = session.initialize(Map::new()).unwrap();
    assert!(matches!(
        session.receive(RpcMessage::Response {
            id: Some(initialize.into()),
            outcome: Ok(json!({"capabilities": {}})),
        }),
        Ok(LspEvent::Initialized(_))
    ));
    let LspEvent::Message(message) = session
        .receive(notification(data["params"].clone()))
        .unwrap()
    else {
        panic!("expected a peer notification");
    };
    let bundle = language_server_extensions();
    let publication = bundle
        .receive_notification(&workspace, &message)
        .unwrap()
        .unwrap();
    assert_eq!(publication.uri, uri);
    assert_eq!(publication.version, file.version());
    let items = data["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(publication.diagnostics.len(), items.len());
    for (item, diagnostic) in items.iter().zip(&publication.diagnostics) {
        assert_eq!(diagnostic.message, string(&item["message"]));
        let mapped = data["mapped"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry[0] == diagnostic.message)
            .unwrap();
        let severity = match string(&mapped[1]) {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            "hint" => Severity::Hint,
            other => panic!("unexpected severity {other}"),
        };
        assert_eq!(diagnostic.severity, severity);
        assert_eq!(diagnostic.range, offset(&mapped[2])..offset(&mapped[3]));
        assert!(diagnostic.source.is_none());
        assert!(diagnostic.actions.is_empty());
    }
    let mut editor = EditorState::new(file.draft().as_str());
    editor
        .update(set_diagnostics_for(&editor, publication.diagnostics))
        .unwrap();
    assert_eq!(
        LintState::get(&editor).unwrap().panel_diagnostics().len(),
        items.len()
    );
    let clear = notification(json!({"uri": uri, "diagnostics": []}));
    let clear = server_diagnostics(&workspace, &clear).unwrap().unwrap();
    assert_eq!(clear.version, file.version());
    editor
        .update(set_diagnostics_for(&editor, clear.diagnostics))
        .unwrap();
    assert_eq!(diagnostic_count(&editor), 0);

    workspace.sync(SyncKind::Incremental).unwrap();
    assert!(server_diagnostics(&workspace, &message).unwrap().is_none());
}

#[test]
fn diagnostic_notification_checks_version_and_rejects_malformed_batches() {
    let uri = "file:///main.rs";
    let mut workspace = DocumentWorkspace::default();
    workspace
        .open_file(uri, "rust", Text::new("abc\r\n😀xyz"))
        .unwrap();
    let diagnostic = json!({
        "range": {"start":{"line":1,"character":2}, "end":{"line":1,"character":5}},
        "message":"<b>literal text</b>",
    });
    let make = |version: Value, diagnostics: Value| {
        notification(json!({"uri": uri, "version": version, "diagnostics": diagnostics}))
    };
    for message in [
        RpcMessage::notification("window/logMessage", None).unwrap(),
        RpcMessage::request(RpcId::Number(1), PUBLISH, None).unwrap(),
        notification(json!({"uri": "file:///closed"})),
        make(json!(1), Value::Null),
        make(json!(-1), Value::Null),
    ] {
        assert!(server_diagnostics(&workspace, &message).unwrap().is_none());
    }
    let unversioned = make(Value::Null, json!([diagnostic]));
    let accepted = server_diagnostics(&workspace, &unversioned)
        .unwrap()
        .unwrap();
    assert_eq!(accepted.diagnostics[0].message, "<b>literal text</b>");
    assert_eq!(accepted.diagnostics[0].range, 9..12);
    assert_eq!(accepted.diagnostics[0].severity, Severity::Error);

    let mut split_pair = diagnostic.clone();
    split_pair["range"]["start"]["character"] = json!(1);
    let mut reversed = diagnostic.clone();
    reversed["range"]["end"]["character"] = json!(0);
    let mut out_of_line = diagnostic.clone();
    out_of_line["range"]["end"]["character"] = json!(6);
    let mut bad_severity = diagnostic.clone();
    bad_severity["severity"] = json!("warning");
    for message in [
        RpcMessage::notification(PUBLISH, None).unwrap(),
        notification(json!({"uri": false, "diagnostics": []})),
        make(json!("0"), json!([])),
        make(json!(2_147_483_648_i64), json!([])),
        make(json!(0), json!({})),
        make(json!(0), json!([diagnostic, split_pair])),
        make(json!(0), json!([reversed])),
        make(json!(0), json!([out_of_line])),
        make(json!(0), json!([bad_severity])),
        make(json!(0), json!([{"message":"missing range"}])),
        make(json!(0), json!([{"range":diagnostic["range"]}])),
    ] {
        assert!(
            server_diagnostics(&workspace, &message).is_err(),
            "{message:?}"
        );
    }
    workspace.close_file(uri).unwrap();
    assert!(
        server_diagnostics(&workspace, &unversioned)
            .unwrap()
            .is_none()
    );
    workspace
        .open_file(uri, "rust", Text::new("different"))
        .unwrap();
    assert!(
        server_diagnostics(&workspace, &make(json!(0), json!([])))
            .unwrap()
            .is_none()
    );
}

#[test]
fn navigation_adapters_match_pinned_requests_and_map_targets() {
    let oracle = oracle();
    let data = &oracle["navigation"];
    let uri = string(&data["params"]["textDocument"]["uri"]);
    let target_uri = string(&data["targetUri"]);
    let source = string(&oracle["source"]);
    let target_doc = string(&data["targetDoc"]);
    let position = offset(&data["requestByte"]);
    let target_start = offset(&data["targetStart"]);
    let cases = data["cases"].as_array().unwrap();
    for (oracle_case, request) in cases.iter().zip([
        definition_request,
        declaration_request,
        type_definition_request,
        implementation_request,
    ]) {
        let mut workspace = DocumentWorkspace::default();
        assert!(matches!(
            request(&workspace, uri, 0),
            Err(FeatureError::Protocol(LspError::MissingDocument(_)))
        ));
        workspace.open_file(uri, "rust", Text::new(source)).unwrap();
        workspace
            .open_file(target_uri, "rust", Text::new(target_doc))
            .unwrap();
        assert!(matches!(
            request(&workspace, uri, position - 3),
            Err(FeatureError::Protocol(LspError::InvalidByteOffset(_)))
        ));
        let query = request(&workspace, uri, position).unwrap();
        assert_eq!(query.method(), string(&oracle_case[0]));
        assert_eq!(query.capability(), string(&oracle_case[1]));
        assert_eq!(query.params(), &data["params"]);
        assert_eq!(query.version(), 0);

        change(
            &mut workspace,
            target_uri,
            Change::new(0..0, string(&data["targetEdit"])),
        );
        workspace.sync(SyncKind::Incremental).unwrap();
        change(&mut workspace, uri, Change::new(0..0, "prefix\n"));
        assert!(matches!(
            request(&workspace, uri, position),
            Err(FeatureError::UnsyncedDocument(_))
        ));
        let known = &data["known"];
        let unknown = &data["unknown"];
        let array = oracle_case[2].as_bool().unwrap();
        let response = if array {
            json!([known, unknown])
        } else {
            known.clone()
        };
        let mapped = query.resolve(&workspace, &response).unwrap().unwrap();
        assert_eq!(mapped.len(), if array { 2 } else { 1 });
        assert_eq!(mapped[0].uri, target_uri);
        assert_eq!(mapped[0].byte_range, Some(target_start..target_start + 5));
        if array {
            assert_eq!(mapped[1].uri, string(&unknown["uri"]));
            assert_eq!(mapped[1].byte_range, None);
        }
        assert_eq!(query.resolve(&workspace, &Value::Null).unwrap(), None);
        assert_eq!(query.resolve(&workspace, &json!([])).unwrap(), Some(vec![]));
        assert!(
            query
                .resolve(
                    &workspace,
                    &json!({"targetUri":target_uri,"targetSelectionRange":known["range"]})
                )
                .is_err()
        );
        change(
            &mut workspace,
            target_uri,
            Change::new(target_start + 1..target_start + 2, "X"),
        );
        assert!(
            matches!(query.resolve(&workspace, known), Err(FeatureError::StaleResponse { uri, .. }) if uri == target_uri)
        );
        workspace.close_file(target_uri).unwrap();
        workspace
            .open_file(target_uri, "rust", Text::new(target_doc))
            .unwrap();
        assert!(
            matches!(query.resolve(&workspace, known), Err(FeatureError::StaleResponse { uri, current: Some(2), .. }) if uri == target_uri)
        );
        change(
            &mut workspace,
            uri,
            Change::new(
                position + "prefix\n".len()..position + "prefix\n".len() + 1,
                "X",
            ),
        );
        assert!(
            matches!(query.resolve(&workspace, unknown), Err(FeatureError::StaleResponse { uri: stale, .. }) if stale == uri)
        );
    }
}
