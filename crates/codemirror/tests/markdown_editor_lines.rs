use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    Text, Transaction, change_filter,
    language::{
        fold::foldable,
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownDocument, MarkdownMarkupCommand,
            MarkdownNode, MarkdownParserConfig, markdown, markdown_snapshot, markdown_with_parser,
            plan_markdown_markup,
        },
    },
    language_data::{NativeLanguage, NativeSyntax, SupportedDocument},
};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/markdown_editor_lines.jsonl");

fn rows() -> impl Iterator<Item = Value> {
    FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn resolve(name: &str) -> Option<NativeLanguage> {
    match name {
        "wat" | "wast" => Some(NativeLanguage::Wast),
        "json" => Some(NativeLanguage::Json),
        _ => None,
    }
}

fn extension(controlled: bool) -> Extension {
    let config = MarkdownConfig::default();
    if controlled {
        markdown_with_parser(
            config,
            MarkdownParserConfig {
                code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(resolve))),
                ..Default::default()
            },
        )
    } else {
        markdown(config)
    }
}

fn document(snapshot: &SupportedDocument) -> &MarkdownDocument {
    let NativeSyntax::Markdown(doc) = snapshot.syntax() else {
        panic!("expected Markdown");
    };
    doc
}

fn host_rows(node: &MarkdownNode, level: usize, out: &mut Vec<Value>) {
    let Range { start, end } = node.range();
    out.push(json!([level, node.kind().name(), start, end]));
    for child in node.children() {
        host_rows(child, level + 1, out);
    }
}

fn child_rows(
    nodes: impl Iterator<Item = (&'static str, Range<usize>, usize)>,
    segments: &[Range<usize>],
) -> Vec<Value> {
    let map = |mut at: usize, end: bool| {
        for segment in segments {
            if at < segment.len() || (end && at == segment.len()) {
                return segment.start + at;
            }
            at -= segment.len();
        }
        assert_eq!(at, 0);
        segments.last().unwrap().end
    };
    let mut remaining = Vec::new();
    nodes
        .map(|(name, range, children)| {
            while remaining.last() == Some(&0) {
                remaining.pop();
            }
            let depth = remaining.len();
            if let Some(count) = remaining.last_mut() {
                *count -= 1;
            }
            remaining.push(children);
            json!([depth, name, map(range.start, false), map(range.end, true)])
        })
        .collect()
}

fn normalized(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

fn mount_rows(doc: &MarkdownDocument, controlled: bool) -> Vec<Value> {
    doc.mounts(|name| if controlled { resolve(name) } else { None })
        .map(|mount| {
            let segments: Vec<_> = mount.source_ranges().collect();
            let source: String = segments
                .iter()
                .map(|range| doc.text().slice(range.clone()).unwrap())
                .collect();
            assert_eq!(mount.syntax().text().as_str(), source, "raw child text");
            let tree = match mount.syntax() {
                NativeSyntax::Wast(wast) => child_rows(
                    wast.nodes()
                        .map(|n| (n.kind().name(), n.range(), n.children().len())),
                    &segments,
                ),
                NativeSyntax::Json(json) => child_rows(
                    json.nodes()
                        .map(|n| (n.kind().name(), n.range(), n.children().count())),
                    &segments,
                ),
                _ => panic!("unexpected mounted language"),
            };
            let ranges: Vec<_> = segments.iter().map(|r| [r.start, r.end]).collect();
            json!({"ranges": ranges, "source": normalized(&source), "tree": tree})
        })
        .collect()
}

fn folds(state: &EditorState, doc: &MarkdownDocument) -> Vec<[usize; 4]> {
    let bytes = doc.text().as_str().as_bytes();
    let mut start = 0;
    let mut rows = Vec::new();
    for end in 0..=bytes.len() {
        let width = match bytes.get(end..) {
            Some([b'\r', b'\n', ..]) => 2,
            Some([b'\r' | b'\n', ..]) => 1,
            Some([]) => 0,
            _ => continue,
        };
        if end >= start {
            if let Some(range) = foldable(state, start, end, |_, from, to| doc.foldable(from, to)) {
                rows.push([start, end, range.start, range.end]);
            }
            start = end + width;
        }
    }
    rows
}

fn check_assoc(snapshot: &SupportedDocument, row: &Value, controlled: bool) {
    let mounts = row["controlled"]["mounts"].as_array().unwrap();
    for probe in row["controlled"]["assoc"].as_array().unwrap() {
        let (at, left, _, right, _): (usize, String, String, String, String) =
            serde_json::from_value(probe.clone()).unwrap();
        let covered = |side| {
            mounts.iter().any(|mount| {
                mount["ranges"].as_array().unwrap().iter().any(|range| {
                    let [from, to]: [usize; 2] = serde_json::from_value(range.clone()).unwrap();
                    match side {
                        Assoc::Before => from < at && at <= to,
                        Assoc::After => from <= at && at < to,
                    }
                })
            })
        };
        // The pinned language facet includes both endpoints. Native routing
        // instead follows the associated side of each segment, including gaps.
        assert_eq!(left, right);
        for (side, pinned) in [(Assoc::Before, left), (Assoc::After, right)] {
            let expected = if controlled && covered(side) {
                resolve(&pinned).unwrap()
            } else {
                NativeLanguage::Markdown
            };
            assert_eq!(
                snapshot.language_at(at, side),
                Some(expected),
                "{row} at {at}"
            );
        }
    }
}

fn check_pin_header(row: &Value) {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let packages = row["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 5);
    assert_eq!(packages[0]["name"], "@codemirror/lang-markdown");
    assert_eq!(packages[0]["version"], "6.5.2");
    assert_eq!(packages[1]["name"], "@codemirror/lang-wast");
    assert_eq!(packages[1]["version"], "6.0.2");
    for package in packages {
        let pin = ["packages", "referenceDependencies", "runtimeDependencies"]
            .into_iter()
            .flat_map(|group| manifest[group].as_array().unwrap())
            .find(|pin| pin["name"] == package["name"])
            .unwrap();
        for key in ["version", "tarball", "integrity"] {
            assert_eq!(package[key], pin[key]);
        }
    }
}

#[test]
fn pinned_default_and_controlled_markdown_follow_logical_editor_lines() {
    let mut fixture = rows();
    check_pin_header(&fixture.next().unwrap());
    let fixture: Vec<_> = fixture.take_while(|row| row["name"] != "markup").collect();
    assert_eq!(fixture.len(), 17);
    for (controlled, mode) in [(false, "default"), (true, "controlled")] {
        let extension = extension(controlled);
        let mut state = EditorState::with_extensions("", extension.clone()).unwrap();
        let mut edits = 0;
        for row in &fixture {
            let raw = row["raw"].as_str().unwrap();
            let label = format!("{mode}: {} {:?}", row["name"], row["edit"]);
            let [anchor, head]: [usize; 2] =
                serde_json::from_value(row["selection"].clone()).unwrap();
            if row["edit"].is_array() {
                let (from, to, insert): (usize, usize, String) =
                    serde_json::from_value(row["edit"].clone()).unwrap();
                state.update(transaction(from..to, &insert)).unwrap();
                edits += 1;
            } else {
                state = EditorState::create_from_text(
                    Text::new(raw),
                    Some(EditorSelection::single(anchor, head)),
                    extension.clone(),
                )
                .unwrap();
            }
            assert_eq!(state.text().as_str(), raw, "{label}");
            let selection = state.selection().main();
            assert_eq!(
                [selection.anchor(), selection.head()],
                [anchor, head],
                "{label}"
            );
            let snapshot = markdown_snapshot(&state).unwrap();
            let doc = document(&snapshot);
            assert_eq!(doc.text(), state.text(), "{label}");
            if row["edit"].is_array() {
                let fresh_state = EditorState::with_extensions(raw, extension.clone()).unwrap();
                let fresh = markdown_snapshot(&fresh_state).unwrap();
                assert_eq!(doc.root(), document(&fresh).root(), "{label}: cold tree");
                assert_eq!(
                    snapshot.highlights(0..raw.len()),
                    fresh.highlights(0..raw.len()),
                    "{label}: cold highlights"
                );
                if (5..=7).contains(&edits) {
                    assert_eq!(doc.reused_blocks(), 3, "{label}");
                    assert_eq!(doc.parsed_bytes(), [80, 79, 79][edits - 5], "{label}");
                } else {
                    assert_eq!(doc.reused_blocks(), 0, "{label}");
                    assert_eq!(doc.parsed_bytes(), raw.len(), "{label}");
                }
            }
            let mut host = Vec::new();
            host_rows(doc.root(), 0, &mut host);
            assert_eq!(json!(host), row["host"], "{label}");
            assert_eq!(
                json!(mount_rows(doc, controlled)),
                row[mode]["mounts"],
                "{label}"
            );
            check_assoc(&snapshot, row, controlled);
            // Compare Markdown folds against the no-child oracle. The mounted
            // WAST folds in the controlled oracle are not Markdown block folds.
            assert_eq!(
                json!(folds(&state, doc)),
                row["default"]["folds"],
                "{label}"
            );
        }
        assert_eq!(edits, 10);
    }
}

fn transaction(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

#[test]
fn accepted_seam_edits_replace_mounts_and_leave_old_snapshots_unchanged() {
    let raw = "# 😀\r\r```wast\r\n(module)\r```\r\r# Next\r尾";
    let split = raw.find("\r\n").unwrap() + 1;
    let label = raw.find("wast").unwrap();
    let filter = change_filter(move |_, _| {
        ChangePermission::Protect(std::iter::once(label..label + 4).collect())
    });
    let mut state =
        EditorState::with_extensions(raw, Extension::group([extension(true), filter])).unwrap();
    let old = markdown_snapshot(&state).unwrap();
    assert_eq!(
        old.language_at(split + 2, Assoc::After),
        Some(NativeLanguage::Wast)
    );
    // Reject the label replacement but accept the edit that splits CRLF.
    let changes = ChangeSet::new(vec![
        Change::new(label..label + 4, "text"),
        Change::new(split..split + 1, ""),
    ])
    .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    let split_text = raw.replacen("\r\n", "\r", 1);
    assert_eq!(state.text().as_str(), split_text);
    let updated = markdown_snapshot(&state).unwrap();
    assert!(!Arc::ptr_eq(&old, &updated));
    assert_eq!(old.text().as_str(), raw);
    assert_eq!(
        updated.language_at(split + 1, Assoc::After),
        Some(NativeLanguage::Wast)
    );
    assert_eq!(document(&updated).reused_blocks(), 0);
    assert_eq!(document(&updated).parsed_bytes(), split_text.len());

    state.update(transaction(label..label + 4, "text")).unwrap();
    assert!(Arc::ptr_eq(&updated, &markdown_snapshot(&state).unwrap()));
    state
        .update(transaction(label..label + 4, "text").without_filtering())
        .unwrap();
    let removed = markdown_snapshot(&state).unwrap();
    assert_eq!(
        removed.language_at(split + 1, Assoc::After),
        Some(NativeLanguage::Markdown)
    );
    assert_eq!(
        updated.language_at(split + 1, Assoc::After),
        Some(NativeLanguage::Wast)
    );
    assert_eq!(
        state.update(transaction(3..3, "x")),
        Err(EditError::InvalidRange)
    );
    assert!(Arc::ptr_eq(&removed, &markdown_snapshot(&state).unwrap()));
}

#[test]
fn direct_native_projection_keeps_one_break_and_reuses_ordinary_lone_cr_blocks() {
    let raw = "# 首\r\r\n# Old 😀\r\r# Anchor\r\rTail\r";
    let doc = MarkdownDocument::parse(Text::new(raw));
    assert_eq!(doc.text().as_str(), raw);
    let old = raw.find("Old").unwrap();
    let updated = doc
        .reparse(&ChangeSet::single(Change::new(old..old + 3, "新しい")).unwrap())
        .unwrap();
    let output = raw.replace("Old", "新しい");
    let fresh = MarkdownDocument::parse(Text::new(&output));
    assert_eq!(updated.root(), fresh.root());
    assert_eq!(updated.reused_blocks(), 2);
    assert!(updated.parsed_bytes() < output.len());
    let lf = normalized(&output);
    let view = MarkdownDocument::parse(Text::new(&lf));
    let logical_nodes = |doc: &MarkdownDocument| {
        doc.nodes()
            .map(|node| {
                (
                    node.kind(),
                    normalized(doc.text().slice(node.range()).unwrap()),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(logical_nodes(&updated), logical_nodes(&view));
    let from = output.find("# Anchor").unwrap();
    let to = from + "# Anchor".len();
    assert_eq!(
        updated.foldable(from, to),
        Some(to..output.rfind('\r').unwrap())
    );
}

#[test]
fn selection_and_header_fold_keep_raw_coordinates() {
    let raw = "# 見出し\r\r> ```wast\r> (module ;; 😀\r>  (func $f))\r> ```";
    let mut state = EditorState::with_extensions(raw, extension(true)).unwrap();
    let snapshot = markdown_snapshot(&state).unwrap();
    let at = raw.find("$f").unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(at + 1, at + 1)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot, &markdown_snapshot(&state).unwrap()));
    let expanded = snapshot.select_parent_syntax(state.selection()).unwrap();
    assert_eq!(expanded.main().range(), at..at + 2);
    let heading_end = raw.find('\r').unwrap();
    assert_eq!(
        foldable(&state, 0, heading_end, |_, _, _| None),
        Some(heading_end..raw.len())
    );
    assert_eq!(
        snapshot.language_at(raw.find('😀').unwrap() + 1, Assoc::Before),
        None
    );
}

#[test]
fn markup_uses_projected_lines_and_keeps_lone_cr_bytes() {
    let rows: Vec<_> = rows().filter(|row| row["name"] == "markup").collect();
    assert_eq!(rows.len(), 4);
    for row in rows {
        let raw = row["raw"].as_str().unwrap();
        let caret = serde_json::from_value(row["caret"].clone()).unwrap();
        let mut state = EditorState::create_from_text(
            Text::new(raw),
            Some(EditorSelection::single(caret, caret)),
            extension(false),
        )
        .unwrap();
        let command = if row["command"] == "continue" {
            MarkdownMarkupCommand::Continue
        } else {
            MarkdownMarkupCommand::DeleteBackward
        };
        let plan = plan_markdown_markup(&state, command).unwrap().unwrap();
        state.update(plan).unwrap();
        let pinned = row["out"].as_str().unwrap();
        assert_eq!(state.text().as_str(), pinned.replace('\n', "\r"));
        assert_eq!(normalized(state.text().as_str()), pinned);
        assert_eq!(json!(state.selection().main().head()), row["cursor"]);
    }
}
