use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Assoc, Compartment, EditorSelection, EditorState, Extension, Transaction,
    language::{
        LanguageData,
        fold::foldable_in_state,
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_snapshot, markdown_with_parser,
        },
        wast::{
            FOLD, SUBLANGUAGES, WastDataFacet, WastFoldCallback, WastNode, WastSublanguage,
            WastSublanguageMode,
        },
    },
    language_data::{NativeLanguage, WastLanguage, wast_language},
};
use serde_json::{Value, json};

fn install(language: WastLanguage) -> Extension {
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |name| {
                    (name == "wat").then(|| MarkdownCodeParser::Wast(language.clone()))
                }),
                fallback: None,
            }),
            ..Default::default()
        },
    )
}

fn span(node: WastNode<'_>) -> Value {
    let range = node.range();
    json!([node.node_type().name(), range.start, range.end])
}

fn callback_read(node: WastNode<'_>, state: &EditorState) -> Value {
    let row = state.text().line_at(node.range().start).unwrap();
    json!([
        span(node),
        node.parent().map(span),
        node.children().map(span).collect::<Vec<_>>(),
        [
            json!(row.range.start),
            json!(row.range.end),
            json!(row.text)
        ],
        state.selection().main().head(),
    ])
}

fn native_reads(mut calls: Value) -> Value {
    for call in calls.as_array_mut().unwrap() {
        // An overlay root's JavaScript parent is the Markdown fence. The
        // Rust WastNode API exposes only WAST parents.
        if call[0][0] == "Module" {
            assert_eq!(call[1][0], "FencedCode");
            call[1] = Value::Null;
        }
    }
    calls
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn installed_wast_callbacks_match_pinned_quoted_and_ordinary_mounts() {
    for row in include_str!("fixtures/wast_markdown_callbacks.jsonl")
        .lines()
        .skip(2)
    {
        let row: Value = serde_json::from_str(row).unwrap();
        let source = row["source"].as_str().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app: WastFoldCallback = Arc::new({
            let calls = calls.clone();
            move |node, state| {
                calls.lock().unwrap().push(callback_read(node, state));
                let mut children = node.children();
                Some(children.next()?.range().end + 1..children.next_back()?.range().start)
            }
        });
        let number = WastDataFacet::new(Some(LanguageData::default().with("probe", "number")));
        let rule = WastSublanguage::new(WastSublanguageMode::Extend, number, {
            let calls = calls.clone();
            move |node, state| {
                calls.lock().unwrap().push(callback_read(node, state));
                node.node_type().is("Number") && state.text().slice(node.range()) == Some("12")
            }
        });
        let stock = wast_language();
        let configured = stock.with_node_props(&[
            FOLD.add(move |row| row.is("App").then(|| Some(app.clone()))),
            SUBLANGUAGES.add(move |row| row.is_top().then(|| vec![rule.clone()])),
        ]);
        let slot = Compartment::new();
        let mut retained = EditorState::with_extensions(
            source,
            Extension::group([
                slot.of(install(configured)),
                stock.data(LanguageData::default().with("probe", "stock")),
            ]),
        )
        .unwrap();
        let snapshot = markdown_snapshot(&retained).unwrap();
        let head = source.find("34").unwrap();
        retained
            .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
            .unwrap();
        assert!(Arc::ptr_eq(
            &snapshot,
            &markdown_snapshot(&retained).unwrap()
        ));
        let mut current = retained.clone();
        current
            .update(Transaction::default().with_effects([slot.reconfigure(install(stock.clone()))]))
            .unwrap();
        assert_eq!(retained.text(), current.text());
        let stock_snapshot = markdown_snapshot(&current).unwrap();
        assert!(!Arc::ptr_eq(&snapshot, &stock_snapshot));
        assert!(Arc::ptr_eq(
            &snapshot,
            &markdown_snapshot(&retained).unwrap()
        ));

        let drain = || std::mem::take(&mut *calls.lock().unwrap());
        let pair = |r: Range<usize>| [r.start, r.end];
        for state in [&retained, &current] {
            assert_eq!(
                json!(
                    stock
                        .find_regions(state)
                        .into_iter()
                        .map(pair)
                        .collect::<Vec<_>>()
                ),
                row["regions"]
            );
        }
        for probe in row["data"].as_array().unwrap() {
            let at = byte(&probe[0]);
            for (side, expected) in [(Assoc::Before, &probe[1]), (Assoc::After, &probe[2])] {
                let configured_data = json!([expected[2], native_reads(expected[3].clone())]);
                let stock_data = json!([
                    if expected[0] == true {
                        vec!["stock"]
                    } else {
                        vec![]
                    },
                    []
                ]);
                for (state, snapshot, data) in [
                    (&retained, &snapshot, configured_data),
                    (&current, &stock_snapshot, stock_data),
                ] {
                    let active = stock.is_active_at(state, at, side);
                    let markdown = snapshot.markdown_language_presence_at(at, side)
                        == Some(NativeLanguage::Markdown);
                    assert_eq!(json!([active, markdown]), json!([expected[0], expected[1]]));
                    let actual = state.language_data_at::<&str>("probe", at, side);
                    assert_eq!(
                        json!([actual, drain()]),
                        data,
                        "{}: {at} {side:?}",
                        row["step"]
                    );
                }
            }
        }
        for fold in row["folds"].as_array().unwrap() {
            let (start, end) = (byte(&fold[0]), byte(&fold[1]));
            for (state, expected) in [(&current, &fold[3]), (&retained, &fold[2])] {
                let result = foldable_in_state(state, start, end).map(pair);
                assert_eq!(
                    json!([result, drain()]),
                    json!([expected[0], native_reads(expected[1].clone())])
                );
            }
        }
        for invalid in [
            source.rfind('😀').unwrap() + 1,
            byte(&row["folds"][0][1]) + 1,
        ] {
            for side in [Assoc::Before, Assoc::After] {
                assert!(!stock.is_active_at(&retained, invalid, side));
                assert!(
                    retained
                        .language_data_at::<&str>("probe", invalid, side)
                        .is_empty()
                );
            }
            assert_eq!(
                foldable_in_state(&retained, byte(&row["folds"][0][0]), invalid),
                None
            );
            assert_eq!(foldable_in_state(&retained, invalid, invalid + 1), None);
            assert!(drain().is_empty());
        }
    }
}

#[test]
fn invalid_segmented_fold_proposals_do_not_hide_valid_ancestors() {
    let source = "前 😀\r\n\r\n> ```wat\r\n> (module (func \"😀\"\r\n> 12\r\n> ))\r\n> ```";
    let inner = source.find("(func").unwrap();
    let line_end = source.find("\r\n> 12").unwrap();
    let left = line_end + 2;
    let right = left + 2;
    let close = source.rfind(')').unwrap();
    let astral = source.rfind('😀').unwrap();
    let inner_fold = inner + 2..right;
    let outer_fold = inner + 1..close;
    for (end, proposal, expected) in [
        (line_end, inner_fold.clone(), inner_fold.clone()),
        (line_end, inner + 2..left, inner + 2..left),
        (left + 1, left..right, left..right),
        (right, right..close, right..close),
        (line_end, inner + 2..left + 1, outer_fold.clone()),
        (left + 1, left + 1..close, outer_fold.clone()),
        (line_end, astral + 1..right, outer_fold.clone()),
        (left, line_end + 1..close, outer_fold.clone()),
        (line_end, inner + 2..source.len(), outer_fold.clone()),
        (line_end, inner + 2..usize::MAX, outer_fold.clone()),
        (line_end, close..inner + 2, outer_fold.clone()),
        (line_end, inner + 2..inner + 2, outer_fold.clone()),
    ] {
        let app: WastFoldCallback = Arc::new({
            let outer_fold = outer_fold.clone();
            move |node, _| {
                Some(if node.range().start == inner {
                    proposal.clone()
                } else {
                    outer_fold.clone()
                })
            }
        });
        let language = wast_language()
            .with_node_props(&[FOLD.add(move |row| row.is("App").then(|| Some(app.clone())))]);
        let state = EditorState::with_extensions(source, install(language)).unwrap();
        assert_eq!(foldable_in_state(&state, inner, end), Some(expected));
    }
}
