use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    lsp::{
        DocumentSnapshot, LspError, Position, PositionIndex, Range, SyncKind, TextEdit, did_change,
        text_edits, workspace_edit,
    },
};
use serde_json::{Value, json};

const A: &str = "file:///a.rs";
const B: &str = "file:///b.rs";

fn pos(line: u32, character: u32) -> Position {
    Position { line, character }
}

fn wire_edit(start: (u32, u32), end: (u32, u32), text: &str) -> Value {
    json!({
        "range": {
            "start": {"line": start.0, "character": start.1},
            "end": {"line": end.0, "character": end.1},
        },
        "newText": text,
    })
}

#[test]
fn positions_round_trip_scalar_boundaries_and_preserve_line_endings() {
    let text = Text::new("a😀é\r\n\rβ\n");
    let index = PositionIndex::new(&text);
    for (byte, line, units) in [
        (0, 0, 0),
        (1, 0, 1),
        (5, 0, 3),
        (7, 0, 4),
        (9, 1, 0),
        (10, 2, 0),
        (12, 2, 1),
        (13, 3, 0),
    ] {
        assert_eq!(index.position(byte), Ok(pos(line, units)));
        assert_eq!(index.offset(pos(line, units)), Ok(byte));
    }
    for byte in [2, 3, 4, 6, 8, 11, 14, usize::MAX] {
        assert_eq!(index.position(byte), Err(LspError::InvalidByteOffset(byte)));
    }
    for position in [pos(0, 2), pos(0, 5), pos(1, 1), pos(4, 0), pos(0, u32::MAX)] {
        assert_eq!(
            index.offset(position),
            Err(LspError::InvalidPosition(position))
        );
    }
    let empty = Text::default();
    let index = PositionIndex::new(&empty);
    assert_eq!(index.position(0), Ok(pos(0, 0)));
    assert_eq!(index.offset(pos(0, 0)), Ok(0));
}

#[test]
fn position_json_rejects_invalid_unsigned_wire_integers() {
    let value = json!({"line": 2147483647_u32, "character": 3});
    assert_eq!(Position::try_from(&value), Ok(pos(2147483647, 3)));
    for line in [json!(-1), json!(2147483648_u32), json!(1.0), json!("1")] {
        assert_eq!(
            Position::try_from(&json!({"line": line, "character": 0})),
            Err(LspError::InvalidFormat("Position"))
        );
    }
}

#[test]
fn text_edits_apply_simultaneously_with_same_position_insertions_in_wire_order() {
    let mut text = Text::new("a😀\r\nxy");
    let edits = [
        wire_edit((1, 1), (1, 2), "!"),
        wire_edit((0, 1), (0, 1), "A"),
        wire_edit((0, 1), (0, 1), "B"),
        wire_edit((0, 1), (0, 3), "é"),
        wire_edit((0, 3), (1, 0), "\r\n>"),
    ]
    .iter()
    .map(|value| TextEdit::try_from(value).unwrap())
    .collect::<Vec<_>>();
    let changes = text_edits(&text, &edits).unwrap();
    changes.apply_to(&mut text).unwrap();
    assert_eq!(text.as_str(), "aABé\r\n>x!");
}

#[test]
fn text_edits_reject_reversed_invalid_and_overlapping_ranges() {
    let text = Text::new("a😀\r\nxy");
    let edit = |start, end| TextEdit {
        range: Range { start, end },
        new_text: "?".into(),
    };
    let reverse = edit(pos(1, 0), pos(0, 3));
    let range = reverse.range;
    assert_eq!(
        text_edits(&text, &[reverse]),
        Err(LspError::ReversedRange(range))
    );
    for position in [pos(0, 2), pos(0, 4), pos(2, 0)] {
        assert_eq!(
            text_edits(&text, &[edit(pos(0, 0), position)]),
            Err(LspError::InvalidPosition(position))
        );
    }
    for second in [
        edit(pos(0, 1), pos(1, 0)),
        edit(pos(0, 1), pos(0, 1)),
        edit(pos(0, 0), pos(0, 0)),
    ] {
        assert_eq!(
            text_edits(&text, &[edit(pos(0, 0), pos(0, 3)), second]),
            Err(LspError::OverlappingEdits)
        );
    }
}

#[test]
fn workspace_changes_plan_all_documents_without_mutation() {
    let first = Text::new("é\r\nx");
    let second = Text::new("xy");
    let value = json!({"changes": {
        (A): [wire_edit((0, 0), (1, 0), "a\r\n")],
        (B): [wire_edit((0, 1), (0, 2), "b")],
    }});
    let resolve = |uri: &str| match uri {
        A => Some(DocumentSnapshot {
            text: &first,
            version: Some(4),
        }),
        B => Some(DocumentSnapshot {
            text: &second,
            version: None,
        }),
        _ => None,
    };
    let plan = workspace_edit(&value, resolve).unwrap();
    assert_eq!(plan.len(), 2);
    for (uri, source, expected) in [(A, &first, "a\r\nx"), (B, &second, "xb")] {
        let changes = &plan.iter().find(|doc| doc.uri == uri).unwrap().changes;
        let mut updated = source.clone();
        changes.apply_to(&mut updated).unwrap();
        assert_eq!(updated.as_str(), expected);
    }
    assert_eq!(first.as_str(), "é\r\nx");
    assert_eq!(second.as_str(), "xy");
    assert_eq!(
        workspace_edit(&value, |uri| resolve(uri).filter(|_| uri == A)),
        Err(LspError::MissingDocument(B.into()))
    );
    let mut invalid = value.clone();
    invalid["changes"][B][0]["range"]["end"]["character"] = json!(3);
    assert_eq!(
        workspace_edit(&invalid, resolve),
        Err(LspError::InvalidPosition(pos(0, 3)))
    );
}

#[test]
fn workspace_document_changes_take_precedence_and_check_versions() {
    let text = Text::new("old");
    let resolve = |_: &str| {
        Some(DocumentSnapshot {
            text: &text,
            version: Some(4),
        })
    };
    let mut edit = json!({
        "changes": { (A): [wire_edit((0, 0), (0, 3), "ignored")] },
        "documentChanges": [{
            "textDocument": {"uri": A, "version": 4},
            "edits": [wire_edit((0, 0), (0, 3), "new")],
        }],
    });
    for version in [json!(4), Value::Null] {
        edit["documentChanges"][0]["textDocument"]["version"] = version;
        let plan = workspace_edit(&edit, resolve).unwrap();
        assert_eq!(plan.len(), 1);
        let mut updated = text.clone();
        plan[0].changes.apply_to(&mut updated).unwrap();
        assert_eq!(updated.as_str(), "new");
    }
    edit["documentChanges"][0]["textDocument"]["version"] = json!(3);
    assert_eq!(
        workspace_edit(&edit, resolve),
        Err(LspError::VersionMismatch {
            uri: A.into(),
            requested: 3,
            snapshot: Some(4)
        })
    );
    edit["documentChanges"][0]["textDocument"]
        .as_object_mut()
        .unwrap()
        .remove("version");
    assert_eq!(
        workspace_edit(&edit, resolve),
        Err(LspError::InvalidFormat("TextDocumentEdit"))
    );
}

#[test]
fn workspace_edit_rejects_resource_annotations_and_repeated_documents() {
    let text = Text::new("a");
    let resolve = |_: &str| {
        Some(DocumentSnapshot {
            text: &text,
            version: None,
        })
    };
    let resource = json!({"documentChanges": [{"kind": "create", "uri": B}]});
    assert_eq!(
        workspace_edit(&resource, resolve),
        Err(LspError::UnsupportedWorkspaceEdit("resource operations"))
    );
    let mut annotated = wire_edit((0, 0), (0, 1), "b");
    annotated["annotationId"] = json!("confirmation");
    let batch = |edit| json!({"textDocument": {"uri": A, "version": null}, "edits": [edit]});
    let doc_edit = batch(wire_edit((0, 0), (0, 1), "b"));
    assert_eq!(
        workspace_edit(&json!({"documentChanges": [batch(annotated)]}), resolve),
        Err(LspError::UnsupportedWorkspaceEdit("annotated text edits"))
    );
    assert_eq!(
        workspace_edit(&json!({"documentChanges": [doc_edit, doc_edit]}), resolve),
        Err(LspError::RepeatedDocument(A.into()))
    );
}

#[test]
fn sync_capabilities_use_only_upstreams_default_position_encoding() {
    for (capabilities, mode) in [
        (json!({}), SyncKind::None),
        (
            json!({"textDocumentSync": {"openClose": true}}),
            SyncKind::None,
        ),
        (json!({"textDocumentSync": 0}), SyncKind::None),
        (json!({"textDocumentSync": 1}), SyncKind::Full),
        (json!({"textDocumentSync": 2}), SyncKind::Incremental),
        (
            json!({"positionEncoding": "utf-16", "textDocumentSync": {"change": 2}}),
            SyncKind::Incremental,
        ),
    ] {
        assert_eq!(SyncKind::from_server_capabilities(&capabilities), Ok(mode));
    }
    for encoding in ["utf-8", "utf-32"] {
        assert_eq!(
            SyncKind::from_server_capabilities(&json!({"positionEncoding": encoding})),
            Err(LspError::UnsupportedPositionEncoding(encoding.into()))
        );
    }
    assert_eq!(
        SyncKind::from_server_capabilities(&json!({"textDocumentSync": {"change": 3}})),
        Err(LspError::InvalidFormat("textDocumentSync"))
    );
}

#[test]
fn incremental_did_change_uses_right_to_left_grouped_old_document_ranges() {
    let before = Text::new(format!("🚀a\r\nline\r\nlast{}", "#".repeat(1024)));
    let changes = ChangeSet::new(vec![
        Change::new(13..17, "END"),
        Change::new(0..4, "X\n"),
        Change::new(4..5, "YZ"),
        Change::new(7..7, "α"),
        Change::new(7..7, "β"),
        Change::new(7..8, "!"),
    ])
    .unwrap();
    let notification = did_change(A, 7, &before, &changes, SyncKind::Incremental)
        .unwrap()
        .unwrap();
    let event = |start, end, text| {
        let edit = wire_edit(start, end, text);
        json!({"range": edit["range"], "text": text})
    };
    assert_eq!(
        notification,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {"textDocument": {"uri": A, "version": 7}, "contentChanges": [
                event((2, 0), (2, 4), "END"),
                event((1, 0), (1, 1), "αβ!"),
                event((0, 0), (0, 3), "X\nYZ"),
            ]},
        })
    );

    let mut receiver = before.clone();
    for event in notification["params"]["contentChanges"].as_array().unwrap() {
        let edit = TextEdit {
            range: Range::try_from(&event["range"]).unwrap(),
            new_text: event["text"].as_str().unwrap().into(),
        };
        text_edits(&receiver, &[edit])
            .unwrap()
            .apply_to(&mut receiver)
            .unwrap();
    }
    let mut expected = before.clone();
    changes.apply_to(&mut expected).unwrap();
    assert_eq!(receiver, expected);
    assert!(receiver.as_str().starts_with("X\nYZ\r\nαβ!ine\r\nEND"));
}

#[test]
fn sync_threshold_uses_new_utf16_length_with_crlf_as_one_break() {
    let changes = ChangeSet::single(Change::new(0..0, "😀")).unwrap();
    for (columns, incremental) in [(1020, false), (1021, true)] {
        let text = Text::new(format!("\r\n{}", "x".repeat(columns)));
        let change = did_change(A, 1, &text, &changes, SyncKind::Incremental)
            .unwrap()
            .unwrap();
        let event = &change["params"]["contentChanges"][0];
        assert_eq!(event.get("range").is_some(), incremental);
        if !incremental {
            assert_eq!(event["text"], format!("😀{text}"));
        }
        let full = did_change(A, 1, &text, &changes, SyncKind::Full)
            .unwrap()
            .unwrap();
        assert_eq!(
            full["params"]["contentChanges"],
            json!([{"text": format!("😀{text}")}])
        );
    }

    let before = Text::new(format!("\rX{}", "x".repeat(1022)));
    let changes = ChangeSet::single(Change::new(1..2, "\n")).unwrap();
    let change = did_change(A, 1, &before, &changes, SyncKind::Incremental)
        .unwrap()
        .unwrap();
    assert_eq!(
        change["params"]["contentChanges"],
        json!([{"text": format!("\r\n{}", "x".repeat(1022))}])
    );
}

#[test]
fn sync_falls_back_for_crlf_splits_but_rejects_invalid_native_byte_ranges() {
    let before = Text::new(format!("😀\r\n{}", "x".repeat(1024)));
    let split = ChangeSet::single(Change::new(4..5, "")).unwrap();
    let change = did_change(A, 1, &before, &split, SyncKind::Incremental)
        .unwrap()
        .unwrap();
    assert_eq!(
        change["params"]["contentChanges"],
        json!([{"text": format!("😀\n{}", "x".repeat(1024))}])
    );

    let bad = ChangeSet::single(Change::new(1..2, "")).unwrap();
    for mode in [SyncKind::Full, SyncKind::Incremental] {
        assert_eq!(
            did_change(A, 1, &before, &bad, mode),
            Err(LspError::InvalidChangeSet)
        );
    }
    assert_eq!(did_change(A, 1, &before, &split, SyncKind::None), Ok(None));
    assert_eq!(
        did_change(A, 1, &before, &ChangeSet::default(), SyncKind::Incremental),
        Ok(None)
    );
}
