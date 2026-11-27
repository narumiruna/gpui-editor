use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, MapMode, Text,
    lsp::{DocumentWorkspace, LspError, Position, SyncKind},
};
use serde_json::json;

const FIRST: &str = "file:///first.md";
const SECOND: &str = "file:///second.yaml";

fn edit(range: std::ops::Range<usize>, text: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, text)).unwrap()
}

#[test]
fn open_sync_close_and_reopen_keep_the_pinned_versions_and_message_shapes() {
    let mut workspace = DocumentWorkspace::default();
    let open = workspace
        .open_file(FIRST, "markdown", Text::new("字\r\n"))
        .unwrap();
    assert_eq!(
        open,
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":FIRST,"languageId":"markdown","text":"字\r\n","version":0}
        }})
    );
    assert_eq!(
        workspace.open_file(FIRST, "plain", Text::new("")),
        Err(LspError::DocumentAlreadyOpen(FIRST.into()))
    );
    workspace.record_change(FIRST, &edit(3..3, "💻")).unwrap();
    workspace.record_change(FIRST, &edit(0..3, "新")).unwrap();
    let file = workspace.file(FIRST).unwrap();
    assert_eq!(file.uri(), FIRST);
    assert_eq!(file.language_id(), "markdown");
    assert_eq!(file.synced().as_str(), "字\r\n");
    assert_eq!(file.draft().as_str(), "新💻\r\n");
    assert_eq!(file.snapshot().version, Some(0));
    assert_eq!(workspace.connected().collect::<Vec<_>>(), [open]);
    assert_eq!(
        workspace.sync(SyncKind::Incremental).unwrap(),
        [json!({
            "jsonrpc":"2.0","method":"textDocument/didChange","params":{
                "textDocument":{"uri":FIRST,"version":1},"contentChanges":[{"text":"新💻\r\n"}]
            }
        })]
    );
    assert_eq!(
        workspace.file(FIRST).unwrap().snapshot().text.as_str(),
        "新💻\r\n"
    );
    assert!(workspace.sync(SyncKind::Incremental).unwrap().is_empty());
    assert_eq!(
        workspace.close_file(FIRST),
        Some(
            json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":FIRST}}})
        )
    );
    assert_eq!(workspace.close_file(FIRST), None);
    let reopened = workspace
        .open_file(FIRST, "markdown", Text::new("Fresh"))
        .unwrap();
    assert_eq!(reopened["params"]["textDocument"]["version"], json!(2));
}

#[test]
fn mapping_uses_the_request_snapshot_before_pending_edits_and_tracks_deletion() {
    let mut workspace = DocumentWorkspace::default();
    workspace
        .open_file(FIRST, "markdown", Text::new("a😀b\r\nx"))
        .unwrap();
    workspace.record_change(FIRST, &edit(0..0, "始")).unwrap();
    let mapping = workspace.workspace_mapping();
    let pos = Position {
        line: 0,
        character: 3,
    };
    assert_eq!(
        mapping.map_position(FIRST, pos, Assoc::Before, MapMode::Simple),
        Ok(Some(8))
    );
    assert_eq!(
        mapping.map_offset(FIRST, 2, Assoc::Before, MapMode::Simple),
        None
    );
    assert_eq!(
        mapping.map_position(
            FIRST,
            Position {
                line: 0,
                character: 2
            },
            Assoc::Before,
            MapMode::Simple
        ),
        Err(LspError::InvalidPosition(Position {
            line: 0,
            character: 2
        }))
    );

    workspace.sync(SyncKind::Full).unwrap();
    assert_eq!(
        mapping.map_position(FIRST, pos, Assoc::After, MapMode::Simple),
        Ok(Some(8))
    );
    workspace.record_change(FIRST, &edit(4..9, "猫")).unwrap();
    assert_eq!(
        mapping.map_position(FIRST, pos, Assoc::Before, MapMode::Simple),
        Ok(Some(4))
    );
    assert_eq!(
        mapping.map_position(FIRST, pos, Assoc::After, MapMode::Simple),
        Ok(Some(7))
    );
    assert_eq!(
        mapping.map_position(FIRST, pos, Assoc::After, MapMode::TrackDel),
        Ok(None)
    );
    assert_eq!(
        mapping.map_position(
            FIRST,
            Position {
                line: 1,
                character: 0
            },
            Assoc::After,
            MapMode::Simple
        ),
        Ok(Some(9))
    );

    workspace.close_file(FIRST).unwrap();
    workspace
        .open_file(FIRST, "markdown", Text::new("same URI"))
        .unwrap();
    assert_eq!(mapping.get_mapping(FIRST), None);
    assert_eq!(
        mapping.map_position(FIRST, pos, Assoc::Before, MapMode::Simple),
        Err(LspError::MissingDocument(FIRST.into()))
    );
}

#[test]
fn same_flush_version_per_file_even_without_notifications() {
    let mut workspace = DocumentWorkspace::default();
    workspace
        .open_file(SECOND, "yaml", Text::new("name: "))
        .unwrap();
    workspace
        .open_file(FIRST, "markdown", Text::new("test"))
        .unwrap();
    workspace.record_change(FIRST, &edit(0..0, "a")).unwrap();
    workspace.record_change(SECOND, &edit(6..6, "字")).unwrap();
    let sent = workspace.sync(SyncKind::Full).unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0]["params"]["textDocument"],
        json!({"uri":SECOND,"version":1})
    );
    assert_eq!(
        sent[1]["params"]["textDocument"],
        json!({"uri":FIRST,"version":1})
    );
    workspace
        .record_change(SECOND, &edit(0..0, "---\n"))
        .unwrap();
    assert!(workspace.sync(SyncKind::None).unwrap().is_empty());
    assert_eq!(
        workspace
            .files()
            .iter()
            .map(|f| f.version())
            .collect::<Vec<_>>(),
        [2, 1]
    );
    let reconnect = workspace.connected().next().unwrap();
    assert_eq!(reconnect["params"]["textDocument"]["text"], "---\nname: 字");
}

#[test]
fn invalid_utf8_edit_does_not_update_the_draft_or_any_active_mapping() {
    let mut workspace = DocumentWorkspace::default();
    workspace
        .open_file(FIRST, "markdown", Text::new("a😀b"))
        .unwrap();
    let mapping = workspace.workspace_mapping();
    let second = workspace.workspace_mapping();
    workspace.record_change(FIRST, &edit(5..6, "B")).unwrap();
    let previous = mapping.get_mapping(FIRST);
    assert_eq!(
        workspace.record_change(FIRST, &edit(2..3, "!")),
        Err(LspError::InvalidChangeSet)
    );
    assert_eq!(workspace.file(FIRST).unwrap().draft().as_str(), "a😀B");
    assert_eq!(mapping.get_mapping(FIRST), previous);
    assert_eq!(second.get_mapping(FIRST), previous);
    assert_eq!(
        workspace.record_change(SECOND, &ChangeSet::default()),
        Err(LspError::MissingDocument(SECOND.into()))
    );
}

#[test]
fn compose_large_incremental_changes_against_the_last_synchronized_document() {
    let mut workspace = DocumentWorkspace::default();
    let source = "x".repeat(1024) + "\r\n字";
    workspace
        .open_file(SECOND, "yaml", Text::new(source))
        .unwrap();
    workspace.record_change(SECOND, &edit(0..0, "o")).unwrap();
    workspace
        .record_change(SECOND, &edit(1027..1030, "👋"))
        .unwrap();
    let messages = workspace.sync(SyncKind::Incremental).unwrap();
    assert_eq!(
        messages[0]["params"]["contentChanges"],
        json!([
            {"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":1}},"text":"👋"},
            {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"text":"o"}
        ])
    );
}
