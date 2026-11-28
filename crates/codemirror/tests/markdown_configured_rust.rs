use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Transaction,
    language::{
        LanguageData,
        fold::foldable_in_state,
        lezer::{FOLD, LrFoldCallback, ParseError},
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_snapshot, markdown_with_parser,
        },
    },
    language_data::{RustLanguage, rust_language},
};
use serde_json::{Value, json};

type Calls = Arc<Mutex<Vec<Value>>>;

fn install(handle: &RustLanguage, fallback: &RustLanguage) -> Extension {
    let named = MarkdownCodeParser::Rust(handle.clone());
    let fallback = MarkdownCodeParser::Rust(fallback.clone());
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |info| (info == "rr").then(|| named.clone())),
                fallback: Some(fallback),
            }),
            ..Default::default()
        },
    )
}

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn check(state: &EditorState, row: &Value, calls: &Calls) {
    let stock = rust_language();
    let regions: Vec<_> = stock
        .find_regions(state)
        .into_iter()
        .map(|range| [range.start, range.end])
        .collect();
    assert_eq!(state.text().as_str(), row["raw"].as_str().unwrap());
    assert_eq!(json!(regions), row["regions"], "{}", row["step"]);
    for probe in row["probes"].as_array().unwrap() {
        let at = byte(&probe[0]);
        for (index, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
            let actual = json!([
                stock.is_active_at(state, at, side),
                state.language_data_at::<&str>("probe", at, side),
            ]);
            assert_eq!(actual, probe[index + 1], "{probe}");
        }
    }
    for fold in row["folds"].as_array().unwrap() {
        let actual = foldable_in_state(state, byte(&fold[0]), byte(&fold[1]))
            .map(|range| [range.start, range.end]);
        assert_eq!(json!(actual), fold[2], "{}: {fold}", row["step"]);
    }
    assert_eq!(
        json!(std::mem::take(&mut *calls.lock().unwrap())),
        row["calls"]
    );
}

fn change(from: usize, to: usize, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(from..to, insert)).unwrap())
}

#[test]
fn configured_rust_markdown_mounts_preserve_identity_folds_and_strict_policy() {
    let rows: Vec<Value> = include_str!("fixtures/markdown_configured_rust.jsonl")
        .lines()
        .map(|row| serde_json::from_str(row).unwrap())
        .collect();
    let row = |name: &str| rows.iter().find(|v| v["step"] == name).unwrap();
    let stock = rust_language();
    let calls = Calls::default();
    let out = calls.clone();
    let callback: LrFoldCallback = Arc::new(move |node, state| {
        let range = node.range();
        let head = state.selection().main().head();
        out.lock().unwrap().push(json!([
            node.name(),
            node.node_type().unwrap().term(),
            range.start,
            range.end,
            head
        ]));
        let fold = node.fold_inside()?;
        Some(fold.start + usize::from(head != 0)..fold.end)
    });
    let configured = stock
        .with_node_props(&[FOLD.add(move |row| (row.term() == 43).then(|| Some(callback.clone())))])
        .configure(None, Some("rust-configured"));
    let strict = configured.configure(Some(true), Some("rust-strict"));
    let disabled =
        configured.with_node_props(&[FOLD.add(|row| (row.term() == 43).then_some(None))]);
    let slot = Compartment::new();
    let markdown = |handle| slot.reconfigure(install(handle, stock));
    let initial = EditorState::with_extensions(
        row("initial")["raw"].as_str().unwrap(),
        Extension::group([
            slot.of(install(&configured, stock)),
            stock.data(LanguageData::default().with("probe", "rust")),
        ]),
    )
    .unwrap();
    check(&initial, row("initial"), &calls);
    let mut selected = initial.clone();
    let at = byte(&row("selection")["select"]);
    selected
        .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
        .unwrap();
    assert!(Arc::ptr_eq(
        &markdown_snapshot(&initial).unwrap(),
        &markdown_snapshot(&selected).unwrap()
    ));
    check(&selected, row("selection"), &calls);
    let mut edited = selected.clone();
    let edit = &row("edit")["edit"];
    edited
        .update(change(
            byte(&edit[0]),
            byte(&edit[1]),
            edit[2].as_str().unwrap(),
        ))
        .unwrap();
    check(&edited, row("edit"), &calls);
    for (step, handle) in [("disable", &disabled), ("strict", &strict)] {
        let mut next = selected.clone();
        next.update(Transaction::default().with_effects([markdown(handle)]))
            .unwrap();
        check(&next, row(step), &calls);
    }
    let mut quoted = selected.clone();
    quoted
        .update(
            Transaction::default()
                .with_effects([slot.reconfigure(install(&configured, &configured))]),
        )
        .unwrap();
    check(&quoted, row("quoted"), &calls);
    let cut = byte(&row("reject")["cut"]);
    let mut broken = selected.clone();
    broken.update(change(cut, cut + 1, "")).unwrap();
    assert_eq!(
        broken.text().as_str(),
        row("reject")["raw"].as_str().unwrap()
    );
    let before = markdown_snapshot(&broken).unwrap();
    let error = broken
        .update(Transaction::default().with_effects([markdown(&strict)]))
        .unwrap_err();
    let EditError::Configuration(ConfigurationError::StrictSyntax(ParseError::UnexpectedToken {
        offset,
        ..
    })) = error
    else {
        panic!("expected strict Rust mount rejection, got {error:?}");
    };
    assert_eq!(offset, byte(&row("reject")["reject"]));
    assert!(Arc::ptr_eq(&before, &markdown_snapshot(&broken).unwrap()));
    assert_eq!(broken.selection(), selected.selection());
    broken
        .update(change(cut, cut, ";").with_effects([markdown(&strict)]))
        .unwrap();
    check(&broken, row("repair"), &calls);
    check(&selected, row("retained"), &calls);
}
