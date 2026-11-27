use std::ops::Range as ByteRange;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    lsp::{
        DocumentWorkspace, LspError, PositionIndex, Range, SyncKind, features::FeatureError,
        references::references_request,
    },
};
use serde_json::{Value, json};

const SOURCE: &str = "file:///source.rs";
const TARGET: &str = "file:///target.rs";

fn open(workspace: &mut DocumentWorkspace, uri: &str, text: &str) {
    workspace.open_file(uri, "rust", Text::new(text)).unwrap();
}

fn change(workspace: &mut DocumentWorkspace, uri: &str, range: ByteRange<usize>, text: &str) {
    let edit = ChangeSet::single(Change::new(range, text)).unwrap();
    workspace.record_change(uri, &edit).unwrap();
}

fn location(uri: &str, line: u32, start: u32, end: u32) -> Value {
    json!({"uri": uri, "range": {
        "start": {"line": line, "character": start},
        "end": {"line": line, "character": end}
    }})
}

// The fixture records the public findReferences command and panel output from
// the manifest-verified dist/index.js. A scratch jsdom editor used an in-memory
// RPC peer. The native test only needs the recorded JSON.
#[test]
fn pinned_reference_request_and_panel_plan() {
    let oracle: Value = serde_json::from_str(include_str!("fixtures/lsp_references.json")).unwrap();
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let pin = manifest["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "@codemirror/lsp-client")
        .unwrap();
    assert_eq!(
        oracle["package"],
        format!(
            "@codemirror/lsp-client@{}",
            pin["version"].as_str().unwrap()
        )
    );
    assert_eq!(oracle["integrity"], pin["integrity"]);
    let string = |key| oracle[key].as_str().unwrap();
    let uri = string("uri");
    let other = string("otherUri");
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, uri, string("doc"));
    open(&mut workspace, other, string("otherDoc"));
    change(&mut workspace, uri, 0..0, string("preRequest"));
    let sync = workspace.sync(SyncKind::Incremental).unwrap();
    assert_eq!(sync.len(), 1);
    assert_eq!(sync[0]["method"], "textDocument/didChange");
    assert_eq!(
        sync[0]["params"]["textDocument"]["version"],
        oracle["syncVersion"]
    );
    let offset = usize::try_from(oracle["originalHead"].as_u64().unwrap()).unwrap();
    let request = references_request(&workspace, uri, offset + string("preRequest").len()).unwrap();
    assert_eq!(request.method(), oracle["method"].as_str().unwrap());
    assert_eq!(request.capability(), "referencesProvider");
    assert_eq!(json!(request.version()), oracle["syncVersion"]);
    assert_eq!(*request.params(), oracle["params"]);
    let source_snapshot = workspace.file(uri).unwrap().synced().clone();
    let other_snapshot = workspace.file(other).unwrap().synced().clone();

    change(&mut workspace, other, 0..0, string("otherSynced"));
    assert_eq!(workspace.sync(SyncKind::Incremental).unwrap().len(), 1);
    change(&mut workspace, uri, 0..0, string("sourcePending"));
    let locations = request
        .resolve(&workspace, &oracle["result"])
        .unwrap()
        .unwrap();
    let input = oracle["result"].as_array().unwrap();
    assert_eq!(locations.len(), input.len());
    let mut display = Vec::new();
    for (location, wire) in locations.iter().zip(input) {
        assert_eq!(location.uri, wire["uri"].as_str().unwrap());
        assert_eq!(location.range, Range::try_from(&wire["range"]).unwrap());
        let (snapshot, shift) = match location.uri.as_str() {
            target if target == uri => (&source_snapshot, string("sourcePending").len()),
            target if target == other => (&other_snapshot, string("otherSynced").len()),
            unknown => {
                assert!(workspace.file(unknown).is_none());
                assert!(location.preview.is_none());
                continue;
            }
        };
        let index = PositionIndex::new(snapshot);
        let start = index.offset(location.range.start).unwrap() + shift;
        let end = index.offset(location.range.end).unwrap() + shift;
        let preview = location.preview.as_ref().unwrap();
        assert_eq!(preview.byte_range, start..end);
        if preview.starts_file_group {
            display.push(json!({"header": preview.file_name}));
        }
        display.push(
            json!({"line": preview.line_number, "before": preview.before,
            "matched": preview.matched, "after": preview.after}),
        );
    }
    assert_eq!(json!(display), oracle["display"]);
    let (first, last) = (locations.first().unwrap(), locations.last().unwrap());
    assert_eq!(first.uri, last.uri);
    assert_eq!(first.range, last.range);
    assert_eq!(
        first.preview.as_ref().unwrap().byte_range,
        last.preview.as_ref().unwrap().byte_range
    );
    let empty = oracle["emptyResults"].as_array().unwrap();
    assert_eq!(request.resolve(&workspace, &empty[0]).unwrap(), None);
    assert_eq!(
        request.resolve(&workspace, &empty[1]).unwrap(),
        Some(vec![])
    );
    let inert = request.resolve(&workspace, &empty[2]).unwrap().unwrap();
    assert_eq!(inert.len(), 1);
    assert!(inert[0].preview.is_none());
}

#[test]
fn request_requires_an_open_synchronized_source_and_a_valid_byte_position() {
    let mut workspace = DocumentWorkspace::default();
    assert!(matches!(
        references_request(&workspace, SOURCE, 0),
        Err(FeatureError::Protocol(LspError::MissingDocument(uri))) if uri == SOURCE
    ));
    open(&mut workspace, SOURCE, "a\r\n😀x");
    for offset in [2, 4, 99] {
        assert!(matches!(
            references_request(&workspace, SOURCE, offset),
            Err(FeatureError::Protocol(LspError::InvalidByteOffset(actual))) if actual == offset
        ));
    }
    change(&mut workspace, SOURCE, 0..0, "x");
    assert!(matches!(
        references_request(&workspace, SOURCE, 0),
        Err(FeatureError::UnsyncedDocument(uri)) if uri == SOURCE
    ));
}

#[test]
fn reject_bad_location_shapes_and_positions_without_a_partial_plan() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, SOURCE, "😀word");
    let request = references_request(&workspace, SOURCE, 4).unwrap();
    let valid = location(SOURCE, 0, 2, 6);
    let unknown = "file:///not-open.rs";
    for invalid in [
        valid.clone(),
        json!([null]),
        json!([{"uri": SOURCE}]),
        json!([{"uri": false, "range": valid["range"]}]),
        json!([{"uri": unknown, "range": {"start":{"line":0,"character":-1},
            "end":{"line":0,"character":1}}}]),
        json!([valid, location(SOURCE, 0, 1, 2)]),
        json!([valid, location(SOURCE, 0, 2, 7)]),
        json!([valid, location(SOURCE, 1, 0, 1)]),
        json!([valid, location(unknown, 0, 2, 1)]),
    ] {
        assert!(
            matches!(
                request.resolve(&workspace, &invalid),
                Err(FeatureError::Protocol(_))
            ),
            "{invalid}"
        );
    }
}

#[test]
fn targets_use_the_original_workspace_and_late_uris_stay_inert() {
    let mut workspace = DocumentWorkspace::default();
    open(&mut workspace, SOURCE, "use word");
    open(&mut workspace, TARGET, "0 word");
    change(&mut workspace, TARGET, 0..0, "é");
    let request = references_request(&workspace, SOURCE, 6).unwrap();
    let result = json!([location(TARGET, 0, 2, 6)]);
    let actual = request.resolve(&workspace, &result).unwrap().unwrap();
    assert_eq!(actual[0].preview.as_ref().unwrap().byte_range, 4..8);

    let late = "file:///late.rs";
    open(&mut workspace, late, "word");
    let result_late = json!([location(late, 0, 0, 4)]);
    let actual = request.resolve(&workspace, &result_late).unwrap().unwrap();
    assert!(actual[0].preview.is_none());

    let mut other_workspace = DocumentWorkspace::default();
    open(&mut other_workspace, SOURCE, "use word");
    open(&mut other_workspace, TARGET, "0 word");
    assert!(matches!(
        request.resolve(&other_workspace, &result),
        Err(FeatureError::StaleResponse { uri, .. }) if uri == SOURCE
    ));

    change(&mut workspace, TARGET, 5..6, "!");
    assert!(matches!(
        request.resolve(&workspace, &result),
        Err(FeatureError::StaleResponse { uri, requested: 0, current: Some(0) }) if uri == TARGET
    ));
    workspace.close_file(TARGET).unwrap();
    open(&mut workspace, TARGET, "0 word");
    assert!(matches!(
        request.resolve(&workspace, &result),
        Err(FeatureError::StaleResponse { uri, requested: 0, current: Some(1) }) if uri == TARGET
    ));
    workspace.close_file(SOURCE).unwrap();
    assert!(matches!(
        request.resolve(&workspace, &result_late),
        Err(FeatureError::StaleResponse { uri, current: None, .. }) if uri == SOURCE
    ));
    open(&mut workspace, SOURCE, "use word");
    assert!(matches!(
        request.resolve(&workspace, &result_late),
        Err(FeatureError::StaleResponse { uri, requested: 0, current: Some(1) }) if uri == SOURCE
    ));
}

#[test]
fn preview_clips_at_scalar_boundaries_and_respects_lsp_line_endings() {
    let mut workspace = DocumentWorkspace::default();
    let (before, after) = ("a".repeat(49), "b".repeat(98));
    let source = format!("ignored\r😀{before}x\rx{after}😀rest");
    open(&mut workspace, SOURCE, &source);
    let request = references_request(&workspace, SOURCE, 0).unwrap();
    let result = json!([location(SOURCE, 1, 51, 52), location(SOURCE, 2, 0, 1)]);
    let plan = request.resolve(&workspace, &result).unwrap().unwrap();
    let first = plan[0].preview.as_ref().unwrap();
    assert_eq!(first.line_number, 2);
    assert_eq!(first.file_name, "source.rs");
    assert_eq!(first.before, before);
    assert_eq!(first.matched, "x");
    assert_eq!(first.after, "");
    let second = plan[1].preview.as_ref().unwrap();
    assert_eq!(second.line_number, 3);
    assert!(!second.starts_file_group);
    assert_eq!(second.before, "");
    assert_eq!(second.matched, "x");
    assert_eq!(second.after, after);
}
