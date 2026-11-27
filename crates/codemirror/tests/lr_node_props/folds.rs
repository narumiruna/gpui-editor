use std::{ops::Range, sync::Mutex};

use gpui_editor_codemirror::{
    EditorSelection,
    language::{
        fold::{fold_service, foldable_in_state},
        lezer::{FOLD, LrFoldCallback, LrFoldNode},
    },
};

use super::*;

fn range(value: &Value) -> Range<usize> {
    let [from, to] = serde_json::from_value::<[usize; 2]>(value.clone()).unwrap();
    from..to
}

fn pair(range: Range<usize>) -> [usize; 2] {
    [range.start, range.end]
}

fn node_span(node: LrFoldNode<'_>) -> Value {
    let span = node.range();
    json!([node.name(), span.start, span.end])
}

fn with_fold(
    parser: &MarkdownCodeParser,
    targets: Vec<String>,
    callback: Option<LrFoldCallback>,
) -> MarkdownCodeParser {
    with_props(
        parser,
        &[FOLD.add(move |row| {
            targets
                .iter()
                .any(|t| row.name() == t)
                .then(|| callback.clone())
        })],
    )
}

#[test]
fn lr_folds_use_current_state_and_mapped_nodes_in_pinned_candidate_order() {
    for line in include_str!("../fixtures/lr_fold_props.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let [stock, _] = handles(&row);
        let source = row["source"].as_str().unwrap();
        let name = row["name"].as_str().unwrap();
        let standalone = row["scope"] == "standalone";
        let targets: Vec<String> = serde_json::from_value(row["targets"].clone()).unwrap();
        for node in with_lr!(&stock, handle => handle.node_types().collect::<Vec<_>>()) {
            assert_eq!(
                node.prop(&FOLD).is_some(),
                targets.iter().any(|name| name == node.name())
                    || matches!(node.name(), "TokensBody" | "SkipBody" | "PrecedenceBody")
            );
        }
        let install = |parser: MarkdownCodeParser| {
            if standalone {
                with_lr!(&parser, handle => handle.extension())
            } else {
                let name = name.to_owned();
                markdown_with_parser(
                    MarkdownConfig::default(),
                    MarkdownParserConfig {
                        code_languages: Some(MarkdownCodeLanguages::Configured {
                            resolve: Arc::new(move |label| (label == name).then(|| parser.clone())),
                            fallback: None,
                        }),
                        ..Default::default()
                    },
                )
            }
        };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let callback: LrFoldCallback = Arc::new({
            let calls = calls.clone();
            move |node, state| {
                assert!(node.node_type().unwrap().prop(&FOLD).unwrap().is_some());
                let head = state.selection().main().head();
                calls.lock().unwrap().push(json!([
                    node_span(node),
                    node.parent().map(node_span),
                    node.children().map(node_span).collect::<Vec<_>>(),
                    head
                ]));
                let from = node.children().next()?.range().end + usize::from(head != 0);
                let to = node.children().next_back()?.range().start;
                Some(from..to)
            }
        });
        let parser = with_fold(&stock, targets.clone(), Some(callback));
        let disabled = with_fold(&stock, targets, None);
        let slot = Compartment::new();
        let selected_head = serde_json::from_value::<usize>(row["head"].clone()).unwrap();
        let create = |extension| EditorState::with_extensions(source, extension).unwrap();
        let initial = create(slot.of(install(parser.clone())));
        let mut selected = initial.clone();
        selected
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::single(selected_head, selected_head)),
            )
            .unwrap();
        let snapshot = |state: &EditorState| {
            if standalone {
                attached_language_snapshot(state)
            } else {
                markdown_snapshot(state)
            }
            .unwrap()
        };
        assert!(Arc::ptr_eq(&snapshot(&initial), &snapshot(&selected)));
        for check in row["checks"].as_array().unwrap() {
            let line = range(&check["line"]);
            let step = check["step"].as_str().unwrap();
            let state = match step {
                "stock" => create(install(stock.clone())),
                "configured" | "gap" => initial.clone(),
                "selected" | "retained" => selected.clone(),
                "disabled" => {
                    let mut state = selected.clone();
                    state
                        .update(
                            Transaction::default()
                                .with_effects([slot.reconfigure(install(disabled.clone()))]),
                        )
                        .unwrap();
                    assert!(!Arc::ptr_eq(&snapshot(&state), &snapshot(&selected)));
                    state
                }
                "service" => {
                    let serviced = range(&check["range"]);
                    create(Extension::group([
                        install(parser.clone()),
                        fold_service(move |_, _, _| Some(serviced.clone())),
                    ]))
                }
                "edited" => {
                    let mut state = selected.clone();
                    let edit =
                        ChangeSet::single(Change::new(selected_head..selected_head, "0")).unwrap();
                    state.update(Transaction::new(edit)).unwrap();
                    state
                }
                _ => unreachable!(),
            };
            let actual = foldable_in_state(&state, line.start, line.end);
            assert_eq!(
                json!(actual.map(pair)),
                check["range"],
                "{name}/{} {step}: fold",
                row["scope"]
            );
            assert_eq!(
                json!(std::mem::take(&mut *calls.lock().unwrap())),
                check["calls"],
                "{name}/{} {step}: callbacks",
                row["scope"]
            );
        }
    }
}

#[test]
fn unrelated_props_keep_the_legacy_stock_fold_choice() {
    let source = "@tokens { Word { \"😀\"\r\n    \"x\"\r  }\r}";
    let stock = lezer_language();
    let field = NodeProp::new();
    let configured = stock.with_node_props(&[field.add(|row| row.is_top().then_some(true))]);
    for parser in [stock.clone(), configured] {
        let state = EditorState::with_extensions(source, parser.extension()).unwrap();
        // For these narrowed bounds the pin selects 16..35. Only the
        // configured callback path uses that inside-out search.
        assert_eq!(foldable_in_state(&state, 9, 23), Some(9..37));
    }
}

#[test]
fn invalid_host_ranges_do_not_hide_a_valid_nested_fold() {
    let source = "[{ \r\n  \"😀\": 1\r\n}]";
    let line = 0..source.find('\r').unwrap();
    let parser = MarkdownCodeParser::Json(json_language().clone());
    for bad in [
        line.end + 1,
        source.find('😀').unwrap() + 1,
        source.len() + 1,
    ] {
        let parser = with_fold(
            &parser,
            vec!["Array".into()],
            Some(Arc::new(move |_, _| Some(1..bad))),
        );
        let state =
            EditorState::with_extensions(source, with_lr!(&parser, handle => handle.extension()))
                .unwrap();
        let inner = 2..source.rfind('}').unwrap();
        let syntax = attached_language_snapshot(&state).unwrap();
        assert_eq!(foldable_in_state(&state, line.start, line.end), Some(inner));
        assert_eq!(foldable_in_state(&state, 0, line.end + 1), None);
        assert_eq!(
            syntax.syntax().foldable(line.start, line.end),
            Some(1..source.len() - 1)
        );
        let stale = EditorState::new(source.replace('1', "2"));
        assert_eq!(
            syntax
                .syntax()
                .foldable_in_scope(&stale, 0..source.len(), line.clone()),
            None
        );
    }
}
