use std::{
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, READ_ONLY, Transaction, commands,
    language::{
        BidiIsolateOptions, IndentOptions,
        brackets::{BracketConfig, BracketMatchingOptions, bracket_matching},
        fold::foldable_in_state,
        get_indentation,
        lezer::{ParseError, SyntaxNode, SyntaxTree},
        wast::{GROUP, NodeProp, WastDocument, WastNodeType, WastPropSource, parse_raw},
    },
    language_data::{
        NativeSyntax, SupportedDocument, WastLanguage, attached_language_snapshot, wast_language,
    },
    setup::minimal_setup,
    view::{Viewport, keymap::KeyPlatform},
};
use serde_json::{Value, json};

const DOC: &str = "(module\n  (func)\n  1\n)\n(; note\nmore ;)\n\"text\"\n";
const FORM: &str = "(module (func i32.const 0) (func i32.const 0))\n";

fn oracle(name: &str) -> Value {
    include_str!("fixtures/wast_node_props.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["name"] == name)
        .unwrap()["value"]
        .take()
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn wast(snapshot: &SupportedDocument) -> &WastDocument {
    let NativeSyntax::Wast(doc) = snapshot.syntax() else {
        panic!("expected WAST attachment");
    };
    doc
}

fn assert_tree_rows<'a>(
    raw: &'a SyntaxTree,
    snapshot: &'a SupportedDocument,
    language: &WastLanguage,
) -> Vec<SyntaxNode<'a>> {
    let rows: Vec<_> = language.node_types().collect();
    let mut pending = vec![raw.root()];
    let mut nodes = Vec::new();
    let mut attached = wast(snapshot).nodes();
    while let Some(node) = pending.pop() {
        let state_node = attached.next().unwrap();
        let row = rows[usize::from(node.term())];
        assert_eq!((row.term(), row.name()), (node.term(), node.name()));
        assert_eq!(
            (state_node.kind().name(), state_node.range()),
            (node.name(), node.range())
        );
        assert!(ptr::eq(row, node.wast_node_type().unwrap()));
        assert!(ptr::eq(row, state_node.node_type()));
        pending.extend(node.children().rev());
        nodes.push(node);
    }
    assert!(attached.next().is_none());
    nodes
}

#[test]
fn sources_read_pre_batch_rows_and_apply_in_term_then_source_order() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<(
        NodeProp<String>,
        WastPropSource,
        WastNodeType,
        WastLanguage,
        SyntaxTree,
        WastDocument,
    )>();

    let stock = wast_language();
    let row = |t: &WastNodeType| {
        json!([
            t.term(),
            t.name(),
            t.is_top(),
            t.is_skipped(),
            t.is_error(),
            t.is_anonymous(),
            t.prop(&GROUP)
        ])
    };
    assert_eq!(
        json!(stock.node_types().map(row).collect::<Vec<_>>()),
        oracle("stock")
    );
    let stock_raw = parse_raw(DOC, false).unwrap();
    assert!(ptr::eq(
        stock_raw.root().wast_node_type().unwrap(),
        stock.node_types().nth(3).unwrap()
    ));

    let events = Arc::new(Mutex::new(Vec::new()));
    let combine_events = events.clone();
    let merged: NodeProp<String> = NodeProp::with_combine(move |old, next| {
        combine_events
            .lock()
            .unwrap()
            .push(json!(["combine", old, next]));
        format!("{old}+{next}")
    });
    let seeded =
        stock.with_node_props(&[merged.add(|t| t.is("App").then(|| String::from("seed")))]);
    let source = |next| {
        let events = events.clone();
        let read = merged.clone();
        merged.add(move |t| {
            events
                .lock()
                .unwrap()
                .push(json!([next, t.term(), t.name(), t.prop(&read)]));
            t.is("App").then(|| String::from(next))
        })
    };
    let alias = NodeProp::new();
    let number = NodeProp::new();
    let unrelated = NodeProp::<i32>::new();
    let flag = NodeProp::new();
    let optional = NodeProp::new();
    let key = NodeProp::new();
    let token = Arc::new("shared value");
    let saved_token = token.clone();
    let configured = seeded.with_node_props(&[
        source("A"),
        source("B"),
        GROUP.add(|t| t.is("App").then(|| vec![String::from("Alias")])),
        alias.add(|t| t.is("Alias").then_some("alias")),
        number.add(|t| t.is("Expression").then_some(1)),
        number.add(|t| t.is("Number").then_some(0)),
        number.add(|_| None),
        flag.add(|t| t.is("String").then_some(false)),
        optional.add(|t| t.is("Keyword").then_some(Some(7))),
        optional.add(|t| t.is("Keyword").then_some(None)),
        key.add(move |t| t.is("App").then(|| saved_token.clone())),
    ]);
    let original_rows: Vec<_> = stock.node_types().collect();
    let seeded_rows: Vec<_> = seeded.node_types().collect();
    let configured_rows: Vec<_> = configured.node_types().collect();
    let changed: Vec<_> = configured_rows
        .iter()
        .zip(&seeded_rows)
        .filter_map(|(&a, &b)| (!ptr::eq(a, b)).then_some(a.term()))
        .collect();
    assert_eq!(json!(changed), oracle("changed"));
    for (old, new) in original_rows.iter().zip(&configured_rows) {
        assert_eq!(
            (
                old.term(),
                old.name(),
                old.is_skipped(),
                old.is_top(),
                old.is_anonymous(),
                old.is_error()
            ),
            (
                new.term(),
                new.name(),
                new.is_skipped(),
                new.is_top(),
                new.is_anonymous(),
                new.is_error()
            )
        );
        assert_eq!(old.prop(&number), None);
        assert_eq!(new.prop(&unrelated), None);
    }
    let app = configured_rows[6];
    assert_eq!(app.prop(&merged).map(String::as_str), Some("seed+A+B"));
    assert_eq!(
        seeded_rows[6].prop(&merged).map(String::as_str),
        Some("seed")
    );
    assert_eq!(original_rows[6].prop(&merged), None);
    assert!(app.is("App") && app.is("Alias") && !app.is("Expression"));
    assert_eq!(app.prop(&alias), None);
    assert_eq!(app.prop(&number), Some(&1));
    assert!(original_rows[6].is("Expression") && !original_rows[6].is("Alias"));
    let changed_group =
        configured.with_node_props(&[alias.add(|t| t.is("Alias").then_some("alias"))]);
    assert_eq!(
        changed_group.node_types().nth(6).unwrap().prop(&alias),
        Some(&"alias")
    );
    let same_token = token.clone();
    let reassigned =
        configured.with_node_props(&[key.add(move |t| t.is("App").then(|| same_token.clone()))]);
    let after = reassigned.node_types().nth(6).unwrap();
    assert!(!ptr::eq(app, after));
    assert!(Arc::ptr_eq(after.prop(&key).unwrap(), &token));
    let empty = configured.with_node_props(&[]);
    assert!(
        configured
            .node_types()
            .zip(empty.node_types())
            .all(|(a, b)| ptr::eq(a, b))
    );
    assert_eq!(
        configured_rows
            .iter()
            .map(|t| t.prop(&number))
            .collect::<Vec<_>>(),
        [
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&1),
            Some(&1),
            Some(&1),
            Some(&1),
            Some(&0),
            Some(&1),
            None
        ]
    );

    let raw = configured.parse(DOC).unwrap();
    let state = EditorState::with_extensions(DOC, configured.extension()).unwrap();
    let attached = snapshot(&state);
    let raw_nodes = assert_tree_rows(&raw, &attached, &configured);
    for (raw, attached) in raw_nodes.into_iter().zip(wast(&attached).nodes()) {
        assert_eq!(raw.prop(&flag), (raw.name() == "String").then_some(&false));
        assert_eq!(
            raw.prop(&optional),
            (raw.name() == "Keyword").then_some(&None)
        );
        assert_eq!(raw.prop(&flag), attached.prop(&flag));
        assert_eq!(raw.prop(&optional), attached.prop(&optional));
        assert_eq!(raw.prop(&number), attached.prop(&number));
        if raw.name() == "App" {
            assert!(Arc::ptr_eq(raw.prop(&key).unwrap(), &token));
            assert!(Arc::ptr_eq(attached.prop(&key).unwrap(), &token));
        } else {
            assert_eq!(raw.prop(&key), None);
            assert_eq!(attached.prop(&key), None);
        }
    }
    assert_eq!(json!(*events.lock().unwrap()), oracle("order"));

    let special = configured
        .with_node_props(&[number.add(|t| (t.is_error() || t.is_anonymous()).then_some(-1))]);
    assert_eq!(
        special.node_types().last().unwrap().prop(&number),
        Some(&-1)
    );
    let error = special.parse(")").unwrap();
    assert_eq!(
        error.root().children().next().unwrap().prop(&number),
        Some(&-1)
    );
    let state = EditorState::with_extensions(")", special.extension()).unwrap();
    assert_eq!(
        wast(&snapshot(&state))
            .root()
            .children()
            .next()
            .unwrap()
            .prop(&number),
        Some(&-1)
    );
}

#[test]
fn same_handle_reuses_checkpoints_and_config_switches_build_cold() {
    let marker = NodeProp::new();
    let visits = Arc::new(AtomicUsize::new(0));
    let count = visits.clone();
    let before = wast_language().with_node_props(&[marker.add(move |t| {
        count.fetch_add(1, Ordering::Relaxed);
        (t.is("App") || t.is("Keyword")).then_some("old")
    })]);
    let same_rows = before.configure(None, None);
    let after = before
        .with_node_props(&[marker.add(|t| (t.is("App") || t.is("Keyword")).then_some("new"))]);
    assert_eq!(before.name(), after.name());
    for (a, b) in before.node_types().zip(after.node_types()) {
        assert_eq!(ptr::eq(a, b), !a.is("App") && !a.is("Keyword"));
    }
    let long = FORM.repeat(12);
    let slot = Compartment::new();
    let initial = EditorState::with_extensions(
        &long,
        Extension::group([slot.of(before.extension()), READ_ONLY.of(true)]),
    )
    .unwrap();
    let original = snapshot(&initial);
    let update = |tx| {
        let mut state = initial.clone();
        state.update(tx).unwrap();
        assert!(state.read_only());
        snapshot(&state)
    };
    let install = |lang: &WastLanguage| {
        Transaction::default().with_effects([slot.reconfigure(lang.extension())])
    };
    let selected = update(Transaction::default().with_selection(EditorSelection::single(1, 1)));
    let reinstalled = update(install(&before.clone()));
    let distinct = update(install(&same_rows));
    assert_eq!(
        json!([&selected, &reinstalled, &distinct].map(|next| Arc::ptr_eq(&original, next))),
        oracle("state identity")
    );
    assert!(
        before
            .node_types()
            .zip(same_rows.node_types())
            .all(|(a, b)| ptr::eq(a, b))
    );
    let counts = |snap: &SupportedDocument, lang: &WastLanguage, value| {
        let raw = lang.parse(snap.text().as_str()).unwrap();
        let nodes = assert_tree_rows(&raw, snap, lang);
        let mut counts = [0, 0];
        for node in nodes {
            let got = node.prop(&marker).copied();
            assert_eq!(
                got,
                matches!(node.name(), "App" | "Keyword").then_some(value)
            );
            match got {
                Some("old") => counts[0] += 1,
                Some("new") => counts[1] += 1,
                None => (),
                _ => unreachable!(),
            }
        }
        counts
    };
    assert_eq!(counts(&distinct, &same_rows, "old"), [96, 0]);
    for (at, reused) in [
        (long.find('0').unwrap(), (1, 1, 231)),
        (long.rfind('0').unwrap(), (530, 180, 3)),
    ] {
        let markers: Vec<_> = [&before, &same_rows, &after]
            .into_iter()
            .enumerate()
            .map(|(i, lang)| {
                let next = update(
                    Transaction::new(ChangeSet::single(Change::new(at..at + 1, "10")).unwrap())
                        .with_effects([slot.reconfigure(lang.extension())]),
                );
                assert!(!Arc::ptr_eq(&original, &next));
                assert_eq!(next.text().slice(at..at + 2), Some("10"));
                let doc = wast(&next);
                let stats = doc.reuse_stats();
                assert_eq!(
                    (
                        stats.resumed_at,
                        stats.nodes_reused,
                        doc.reused_suffix_actions()
                    ),
                    if i == 0 { reused } else { (0, 0, 0) }
                );
                counts(&next, lang, if i == 2 { "new" } else { "old" })
            })
            .collect();
        assert_eq!(json!(markers), oracle("state markers"));
    }
    assert_eq!(initial.text().as_str(), long);
    assert_eq!(original.text().as_str(), long);
    assert!(Arc::ptr_eq(&original, &snapshot(&initial)));
    assert_eq!(counts(&original, &before, "old"), [96, 0]);
    assert_eq!(visits.load(Ordering::Relaxed), 13);
}

#[test]
fn configured_strict_attachment_keeps_read_only_and_error_semantics() {
    let key = NodeProp::new();
    let strict = wast_language()
        .configure(Some(true), Some("strict sample"))
        .with_node_props(&[key.add(|t| t.is("Module").then_some("root"))]);
    let reset = strict.configure(Some(false), Some("recovered sample"));
    assert_eq!(strict.name(), "strict sample");
    let slot = Compartment::new();
    let source = ";; 🦀\r\n(module)";
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([slot.of(strict.extension()), READ_ONLY.of(true)]),
    )
    .unwrap();
    let original = snapshot(&state);
    let strict_row = strict.node_types().nth(3).unwrap();
    assert!(commands::delete_char_forward(&state).unwrap().is_none());
    assert!(state.read_only());
    assert!(ptr::eq(wast(&original).root().node_type(), strict_row));
    assert_eq!(wast(&original).root().prop(&key), Some(&"root"));
    assert_eq!(
        strict.parse(source).unwrap().root().prop(&key),
        Some(&"root")
    );
    let append = |at, text| Transaction::new(ChangeSet::single(Change::new(at..at, text)).unwrap());
    let bad = append(source.len(), "(").with_selection(EditorSelection::single(1, 1));
    let Err(EditError::Configuration(ConfigurationError::StrictSyntax(
        ParseError::UnexpectedToken { offset, .. },
    ))) = state.update(bad)
    else {
        panic!("strict configuration accepted invalid input");
    };
    assert_eq!(offset, source.len() + 1);
    assert_eq!(state.selection().main().head(), 0);
    assert_eq!(state.text().as_str(), source);
    assert!(Arc::ptr_eq(&original, &snapshot(&state)));

    state.update(append(source.len(), "(func)")).unwrap(); // Read-only does not reject explicit transactions.
    let valid = snapshot(&state);
    assert_eq!(valid.text().as_str(), format!("{source}(func)"));
    assert!(ptr::eq(wast(&valid).root().node_type(), strict_row));
    state
        .update(append(valid.text().len(), "(").with_effects([slot.reconfigure(reset.extension())]))
        .unwrap();
    let recovered = snapshot(&state);
    assert_eq!(recovered.text().as_str(), format!("{source}(func)("));
    assert!(ptr::eq(wast(&recovered).root().node_type(), strict_row));
    assert_eq!(wast(&recovered).root().prop(&key), Some(&"root"));
    assert!(matches!(
        state.update(Transaction::default().with_effects([slot.reconfigure(strict.extension())])),
        Err(EditError::Configuration(ConfigurationError::StrictSyntax(
            _
        )))
    ));
    assert!(Arc::ptr_eq(&recovered, &snapshot(&state)));
    assert_eq!(original.text().as_str(), source);
    assert_eq!(wast(&original).root().prop(&key), Some(&"root"));
}

#[test]
fn configured_rows_do_not_change_stock_consumers() {
    let configured =
        wast_language().with_node_props(&[GROUP.add(|t| t.is("Expression").then(Vec::new))]);
    let state = EditorState::with_extensions(
        DOC,
        Extension::group([
            configured.extension(),
            bracket_matching(BracketMatchingOptions {
                brackets: Some(Arc::from("[]")),
                max_scan_distance: Some(1),
                ..Default::default()
            }),
        ]),
    )
    .unwrap();
    let attached = snapshot(&state);
    let doc = wast(&attached);
    let options = BracketConfig {
        brackets: "[]",
        max_scan_distance: 1,
        after_cursor: true,
    };
    let mut matches = minimal_setup::<()>(state.clone(), KeyPlatform::Linux).bracket_matches();
    assert_eq!(matches.len(), 1);
    let bracket = matches.pop().unwrap();
    assert_eq!(
        doc.bracket_near_with_config(0, options),
        Some(bracket.clone())
    );
    let pair = |r: std::ops::Range<usize>| [r.start, r.end];
    let indent =
        [17, 2, 12].map(|at| get_indentation(&state, at, IndentOptions::default(), None).unwrap());
    let fold = [[0, 7], [23, 30]].map(|[from, to]| foldable_in_state(&state, from, to).map(pair));
    assert_eq!(
        json!({"indent": indent, "fold": fold,
        "brackets": [json!(pair(bracket.start)), json!(bracket.end.map(pair)), json!(bracket.matched)]}),
        oracle("consumers")
    );
    let keyword = doc.nodes().find(|node| node.range() == (1..7)).unwrap();
    assert!(!keyword.node_type().is("Expression"));
    assert!(keyword.kind().is_expression());
    let tag = doc.highlights(keyword.range()).unwrap().next().unwrap();
    assert_eq!(tag.range, 1..7);
    assert_eq!(
        tag.tags.iter().map(|tag| tag.name()).collect::<Vec<_>>(),
        ["keyword"]
    );
    let marks = doc
        .bidi_isolate_marks(
            &Viewport::complete(doc.text()),
            BidiIsolateOptions {
                always_isolate: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        marks
            .iter(0)
            .map(|mark| (mark.from..mark.to, mark.value.bidi_direction()))
            .collect::<Vec<_>>(),
        [(23..38, None), (39..45, None)]
    );
}
