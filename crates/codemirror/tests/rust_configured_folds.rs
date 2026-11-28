use std::{
    collections::BTreeMap,
    ops::Range,
    ptr,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Precedence, Transaction,
    language::{
        fold::{fold_service, foldable_in_state},
        lezer::{FOLD, LrFoldCallback, LrSyntaxNode, NodeProp},
        rust::{RustDocument, RustNodeKind},
    },
    language_data::{
        NativeSyntax, RustLanguage, SupportedDocument, attached_language_snapshot, rust_language,
    },
};
use serde_json::{Value, json};

fn fixture() -> BTreeMap<String, Value> {
    include_str!("fixtures/rust_configured_folds.jsonl")
        .lines()
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["case"].as_str().unwrap().to_owned(), row)
        })
        .collect()
}

fn pair(range: Range<usize>) -> [usize; 2] {
    [range.start, range.end]
}

fn range(value: &Value) -> Range<usize> {
    let [from, to] = serde_json::from_value::<[usize; 2]>(value.clone()).unwrap();
    from..to
}

fn span(node: LrSyntaxNode<'_>) -> Value {
    let range = node.range();
    json!([
        node.name(),
        node.node_type().unwrap().term(),
        range.start,
        range.end
    ])
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn rust(snapshot: &SupportedDocument) -> &RustDocument {
    let NativeSyntax::Rust(document) = snapshot.syntax() else {
        panic!("expected attached Rust syntax");
    };
    document
}

fn with_block(base: &RustLanguage, callback: Option<LrFoldCallback>) -> RustLanguage {
    base.with_node_props(&[FOLD.add(move |row| (row.term() == 43).then(|| callback.clone()))])
}

fn replace(range: Range<usize>, value: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, value)).unwrap())
}

#[test]
fn attached_rust_folds_read_live_state_and_keep_verified_rows_in_retained_trees() {
    let replay = fixture();
    let source = replay["inputs"]["source"].as_str().unwrap();
    let stock = rust_language();
    let stock_rows: Vec<_> = stock
        .node_types()
        .filter(|row| row.prop(&FOLD).and_then(Option::as_ref).is_some())
        .map(|row| json!([row.term(), row.name()]))
        .collect();
    assert_eq!(json!(stock_rows), replay["pins"]["rows"]);

    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback: LrFoldCallback = Arc::new({
        let calls = calls.clone();
        move |node, state| {
            let head = state.selection().main().head();
            calls.lock().unwrap().push(json!({
                "node": span(node), "parent": node.parent().map(span),
                "children": node.children().map(span).collect::<Vec<_>>(), "head": head,
            }));
            let inside = node.fold_inside()?;
            // The pinned callback steps back over one logical line break.
            let step = 1 + usize::from(state.text().as_str()[..inside.end].ends_with("\r\n"));
            Some(inside.start + usize::from(head != 0)..inside.end - step)
        }
    });
    let configured = with_block(stock, Some(callback.clone()));
    let disabled = with_block(&configured, None);
    let configured_row = configured.node_types().nth(43).unwrap();
    assert!(Arc::ptr_eq(
        configured_row.prop(&FOLD).unwrap().as_ref().unwrap(),
        &callback
    ));
    assert!(matches!(
        disabled.node_types().nth(43).unwrap().prop(&FOLD),
        Some(None)
    ));
    for (row, stock) in configured.node_types().zip(stock.node_types()) {
        assert_eq!(ptr::eq(row, stock), stock.term() != 43);
    }
    let verify_row = |state: &EditorState, handle: &RustLanguage| {
        let doc = snapshot(state);
        let expected = handle.node_types().nth(43).unwrap();
        let actual = rust(&doc)
            .nodes()
            .find(|node| node.kind() == RustNodeKind::Block)
            .unwrap();
        assert!(ptr::eq(actual.node_type().unwrap(), expected));
    };
    let query = |case: &str, state: &EditorState| {
        let row = &replay[case];
        let line = range(&row["line"]);
        assert_eq!(
            json!(foldable_in_state(state, line.start, line.end).map(pair)),
            row["fold"],
            "{case}"
        );
        assert_eq!(
            json!(std::mem::take(&mut *calls.lock().unwrap())),
            row["calls"],
            "{case}"
        );
    };
    let make = |handle: &RustLanguage, source: &str| {
        EditorState::with_extensions(source, handle.extension()).unwrap()
    };
    let language = Compartment::new();
    let services = Compartment::new();
    let initial = EditorState::with_extensions(
        source,
        Extension::group([
            language.of(configured.extension()),
            services.of(Extension::default()),
        ]),
    )
    .unwrap();
    let original = snapshot(&initial);
    verify_row(&initial, &configured);
    assert_eq!(
        rust(&original).syntax_lines().line(2).unwrap().range,
        range(&replay["configured"]["line"])
    );
    query("configured", &initial);
    query("stock", &make(stock, source));
    let unrelated = NodeProp::new();
    let marked = stock.with_node_props(&[unrelated.add(|row| row.is_top().then_some(true))]);
    query("stock", &make(&marked, source));
    let mut selected = initial.clone();
    let head = rust(&original).syntax_lines().line(3).unwrap().range.start;
    selected
        .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&selected), &original));
    query("selection only", &selected);

    let at = source.find('字').unwrap();
    let mut edited = selected.clone();
    edited
        .update(replace(at..at + '字'.len_utf8(), "橋橋"))
        .unwrap();
    assert!(!Arc::ptr_eq(&snapshot(&edited), &original));
    verify_row(&edited, &configured);
    query("edit shifts closing range", &edited);
    let mut removed = selected.clone();
    removed
        .update(Transaction::default().with_effects([language.reconfigure(disabled.extension())]))
        .unwrap();
    assert!(!Arc::ptr_eq(&snapshot(&removed), &original));
    verify_row(&removed, &disabled);
    query("explicit removal", &removed);
    let line = range(&replay["stock"]["line"]);
    let stock_fold = Some(range(&replay["stock"]["fold"]));
    for state in [&initial, &selected, &removed] {
        let doc = snapshot(state);
        assert_eq!(doc.syntax().foldable(line.start, line.end), stock_fold);
        assert_eq!(rust(&doc).foldable(line.start, line.end), stock_fold);
        assert!(calls.lock().unwrap().is_empty());
    }
    removed
        .update(Transaction::default().with_effects([language.reconfigure(stock.extension())]))
        .unwrap();
    query("restore stock", &removed);

    let service_calls = Arc::new(Mutex::new(Vec::new()));
    let provider = |name, precedence, result: Option<Range<usize>>| {
        let service_calls = service_calls.clone();
        fold_service(move |_, _, _| {
            service_calls.lock().unwrap().push(name);
            result.clone()
        })
        .with_precedence(precedence)
    };
    for case in ["service wins", "invalid first service result"] {
        let expected = &replay[case]["fold"];
        let proposal = if expected[1].is_null() {
            19..999
        } else {
            range(expected)
        };
        let mut serviced = selected.clone();
        let extensions = Extension::group([
            provider("low", Precedence::Low, Some(18..47)),
            provider("normal", Precedence::Default, Some(proposal)),
            provider("high", Precedence::High, None),
        ]);
        serviced
            .update(Transaction::default().with_effects([services.reconfigure(extensions)]))
            .unwrap();
        assert!(Arc::ptr_eq(&snapshot(&serviced), &original));
        if expected[1].is_null() {
            // The pin returns its first invalid service result. Native rejects it
            // without calling another provider or trying the node rule.
            assert_eq!(foldable_in_state(&serviced, line.start, line.end), None);
            assert!(calls.lock().unwrap().is_empty());
        } else {
            query(case, &serviced);
        }
        assert_eq!(
            json!(std::mem::take(&mut *service_calls.lock().unwrap())),
            replay[&format!("{case}, order")]["order"]
        );
        serviced
            .update(
                Transaction::default().with_effects([services.reconfigure(Extension::default())]),
            )
            .unwrap();
        query(&format!("removed {case}"), &serviced);
        assert!(service_calls.lock().unwrap().is_empty());
    }

    let strict = configured.configure(Some(true), None);
    query("strict valid", &make(&strict, source));
    let at = source.find(';').unwrap();
    let mut recovering = initial.clone();
    recovering.update(replace(at..at + 1, "")).unwrap();
    let recovered = snapshot(&recovering);
    assert!(rust(&recovered).is_recovered());
    verify_row(&recovering, &configured);
    query("missing semicolon", &recovering);
    query(
        "missing semicolon, disabled",
        &make(&disabled, recovering.text().as_str()),
    );
    let switch = || language.reconfigure(strict.extension());
    assert!(matches!(
        recovering.update(Transaction::default().with_effects([switch()])),
        Err(EditError::Configuration(ConfigurationError::StrictSyntax(
            _
        )))
    ));
    assert!(Arc::ptr_eq(&snapshot(&recovering), &recovered));
    assert!(calls.lock().unwrap().is_empty());
    recovering
        .update(replace(at..at, ";").with_effects([switch()]))
        .unwrap();
    let repaired = snapshot(&recovering);
    assert!(!rust(&repaired).is_recovered());
    assert!(!Arc::ptr_eq(&repaired, &recovered));
    verify_row(&recovering, &strict);
    query("repaired strict input", &recovering);
    let open = source.replace("}\r\n", "");
    query("missing closing brace", &make(&configured, &open));
    query("missing closing brace, disabled", &make(&disabled, &open));
    let trailing = replay["inputs"]["trailingError"].as_str().unwrap();
    query(
        "trailing nonempty error folds to node end",
        &make(stock, trailing),
    );
    for (index, source) in replay["inputs"]["comments"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let state = make(&configured, source.as_str().unwrap());
        let case = format!("stock comment {index}");
        if replay[&case]["fold"][1].is_null() {
            // The pin's half-scalar endpoint has no native UTF-8 projection.
            let line = range(&replay[&case]["line"]);
            assert_eq!(foldable_in_state(&state, line.start, line.end), None);
        } else {
            query(&case, &state);
        }
    }
    assert!(Arc::ptr_eq(&snapshot(&selected), &original));
    assert!(Arc::ptr_eq(&snapshot(&initial), &original));
    query("retained selected", &selected);
    query("retained initial", &initial);
}

#[test]
fn invalid_outer_callbacks_do_not_replace_an_inner_fold() {
    let replay = fixture();
    let source = replay["inputs"]["nested"].as_str().unwrap();
    let line = range(&replay["nested stock outer"]["line"]);
    let stock = rust_language();
    let state = EditorState::with_extensions(source, stock.extension()).unwrap();
    assert_eq!(
        foldable_in_state(&state, line.start, line.end),
        Some(range(&replay["nested stock outer"]["fold"]))
    );
    for (case, proposal) in [
        ("callback returns null", None),
        ("callback starts off the line", Some(21..27)),
        ("callback ends at the line end", Some(8..18)),
        ("invalid outer callback", Some(8..999)),
    ] {
        let order = Arc::new(Mutex::new(Vec::new()));
        let configured = with_block(
            stock,
            Some(Arc::new({
                let order = order.clone();
                move |node, _| {
                    let parent = node.parent()?.name();
                    order.lock().unwrap().push(parent);
                    if parent == "FunctionItem" {
                        proposal.clone()
                    } else {
                        node.fold_inside()
                    }
                }
            })),
        );
        let state = EditorState::with_extensions(source, configured.extension()).unwrap();
        let expected = if replay[case]["fold"][1].is_null() {
            // The pin accepts an endpoint past the document. Native skips it.
            &replay["callback returns null"]["fold"]
        } else {
            &replay[case]["fold"]
        };
        assert_eq!(
            foldable_in_state(&state, line.start, line.end),
            Some(range(expected)),
            "{case}"
        );
        assert_eq!(
            json!(*order.lock().unwrap()),
            replay[&format!("{case}, order")]["order"]
        );
    }
}

#[test]
fn configured_rust_folds_validate_utf8_crlf_and_contiguous_scope() {
    let source = "fn f() { if yes { \r\n  \"😀\"\r\n} }\r\n";
    let prefix = "🦀\r\n";
    let host = format!("{prefix}{source}outside");
    let origin = prefix.len();
    let body = origin..origin + source.len();
    let line = origin..origin + source.find('\r').unwrap();
    let inner = origin + source.rfind('{').unwrap() + 1..origin + source.find('}').unwrap();
    let state = EditorState::new(host.as_str());
    for bad in [line.end + 1, host.find('😀').unwrap() + 1, body.end + 1] {
        let expected = host.clone();
        let configured = with_block(
            rust_language(),
            Some(Arc::new(move |node, state| {
                assert_eq!(state.text().as_str(), expected);
                if node.parent()?.name() == "FunctionItem" {
                    Some(node.range().start + 1..bad)
                } else {
                    node.fold_inside()
                }
            })),
        );
        let local = EditorState::with_extensions(source, configured.extension()).unwrap();
        let snapshot = snapshot(&local);
        let syntax = snapshot.syntax();
        let query = |state, body, line| syntax.foldable_in_scope(state, body, line);
        assert_eq!(
            query(&state, body.clone(), line.clone()),
            Some(inner.clone())
        );
        assert_eq!(query(&state, body.clone(), line.start..line.end + 1), None);
        assert_eq!(query(&state, body.clone(), line.start - 1..line.end), None);
        assert_eq!(query(&state, body.start + 1..body.end, line.clone()), None);
        let stale = EditorState::new(host.replace("yes", "not"));
        assert_eq!(query(&stale, body.clone(), line.clone()), None);
    }
}

#[test]
fn local_rust_fallback_folds_do_not_infer_a_verified_row() {
    let replay = fixture();
    let input = &replay["local fallback input"];
    let count = serde_json::from_value::<usize>(input["count"].clone()).unwrap();
    let prefix = input["prefix"].as_str().unwrap();
    let tail = input["tail"].as_str().unwrap();
    let source = format!("{prefix}{}", tail.repeat(count));
    let term = serde_json::from_value::<usize>(input["term"].clone()).unwrap();
    let pinned_stock = &replay["local fallback input, pinned stock"];
    let line = range(&pinned_stock["line"]);
    let expected = Some(range(&pinned_stock["fold"]));
    let pinned_disabled = &replay["local fallback input, pinned disabled"];
    assert!(pinned_disabled["fold"].is_null());
    let stock = rust_language();
    let overridden = with_block(
        stock,
        Some(Arc::new(|_, _| panic!("callback on a rowless block"))),
    );
    let disabled = with_block(stock, None);
    for handle in [stock, &overridden, &disabled] {
        assert_eq!(handle.node_types().nth(term).unwrap().name(), "Block");
        let state = EditorState::with_extensions(source.as_str(), handle.extension()).unwrap();
        let doc = snapshot(&state);
        let rust = rust(&doc);
        assert!(rust.is_recovered());
        let block = rust
            .nodes()
            .find(|node| node.kind() == RustNodeKind::Block)
            .unwrap();
        // The pin retains a verified Block and disables its fold. The local
        // fallback lacks a verified row and keeps the native stock fold.
        assert!(block.node_type().is_none());
        assert_eq!(rust.foldable(line.start, line.end), expected);
        assert_eq!(foldable_in_state(&state, line.start, line.end), expected);
    }
}
