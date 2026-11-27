use std::{
    num::NonZeroUsize,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditError, EditorSelection, EditorState, Extension,
    READ_ONLY, TAB_SIZE, Transaction,
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle, StreamSettings, get_indentation,
        indent_service,
        wast::{INDENT, WastIndentContext, WastIndentStrategy},
    },
    language_data::{NativeSyntax, WastLanguage, attached_language_snapshot, wast_language},
};
use serde_json::{Value, json};

fn oracle(name: &str) -> Value {
    include_str!("fixtures/wast_indent_props.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["name"] == name)
        .unwrap()["value"]
        .take()
}

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

type Events = Arc<Mutex<Vec<Value>>>;

fn enter(cx: &WastIndentContext<'_>, events: &Events) -> usize {
    let state = cx.state().unwrap();
    let lines = cx.line_context();
    let position = lines.position();
    let line = lines.text().line_at(position).unwrap();
    assert_eq!(lines.settings().tab_size.get(), 8);
    assert_eq!(state.tab_size(), lines.settings().tab_size);
    assert!(state.text().as_str().contains("\r\n"));
    assert!(state.text().as_str().contains('😀'));
    assert_eq!(lines.text().len(), state.text().len());
    let range = cx.node().range();
    events.lock().unwrap().push(json!([
        cx.node().node_type().name(),
        [range.start, range.end],
        position,
        [line.range.start, line.range.end],
        lines.text_after_pos(position, Assoc::After).unwrap(),
        lines.unit(),
        state.read_only(),
    ]));
    line.number
}

fn app(extra: usize, events: Events) -> WastIndentStrategy {
    Arc::new(move |cx| {
        enter(cx, &events);
        let column = if cx.node().range().start == 1 {
            cx.base_indent()? + extra
        } else {
            cx.continue_indent()? + cx.line_context().unit()
        };
        events.lock().unwrap().push(json!(["App", column]));
        Some(column)
    })
}

#[test]
fn pinned_strategies_and_state_identities() {
    let source = oracle("source");
    let source = source.as_str().unwrap();
    let queries = oracle("queries");
    let identities = oracle("identity");
    let events: Events = Arc::default();
    let old_app = app(7, events.clone());
    let new_app = app(13, events.clone());
    let keyword: WastIndentStrategy = Arc::new({
        let events = events.clone();
        move |cx| {
            let line = enter(cx, &events);
            let column = if line == 3 {
                None
            } else {
                cx.continue_indent().map(|column| column + 1)
            };
            events.lock().unwrap().push(json!(["Keyword", column]));
            column
        }
    });
    let app_prop = |app: WastIndentStrategy| INDENT.add(move |t| t.is("App").then(|| app.clone()));
    let stock = wast_language();
    let before = stock.with_node_props(&[app_prop(old_app.clone()), {
        let keyword = keyword.clone();
        INDENT.add(move |t| t.is("Keyword").then(|| keyword.clone()))
    }]);
    let same_rows = before.configure(None, None);
    let after = before.with_node_props(&[app_prop(new_app.clone())]);
    for row in stock.node_types() {
        assert_eq!(row.prop(&INDENT).is_some(), row.is("App"));
    }
    let slot = Compartment::new();
    let initial = EditorState::with_extensions(
        source,
        Extension::group([
            slot.of(before.extension()),
            READ_ONLY.of(true),
            TAB_SIZE.of(NonZeroUsize::new(8).unwrap()),
            INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
        ]),
    )
    .unwrap();
    let original = attached_language_snapshot(&initial).unwrap();
    assert_eq!(initial.text().as_str(), source);
    let with_base = |start| (start == 0).then_some(5);
    for (i, query) in queries.as_array().unwrap().iter().enumerate() {
        let position = offset(&query["position"]);
        let options = if i == 4 {
            IndentOptions {
                simulate_break: Some(position),
                simulate_double_break: true,
                override_indentation: Some(&with_base),
            }
        } else {
            IndentOptions::default()
        };
        let column = get_indentation(&initial, position, options, None).unwrap();
        assert_eq!(json!(column), query["column"]);
        let calls = std::mem::take(&mut *events.lock().unwrap());
        assert_eq!(json!(calls), query["events"]);
    }
    let invalid = source.find('😀').unwrap() + 1;
    assert_eq!(
        get_indentation(&initial, invalid, IndentOptions::default(), None),
        Err(EditError::InvalidRange)
    );
    assert!(events.lock().unwrap().is_empty());

    let first = offset(&queries[0]["position"]);
    let update = |tx| {
        let mut state = initial.clone();
        state.update(tx).unwrap();
        state
    };
    let install = |lang: &WastLanguage| slot.reconfigure(lang.extension());
    let same_tree =
        |state: &EditorState| Arc::ptr_eq(&attached_language_snapshot(state).unwrap(), &original);
    let identity = [
        update(Transaction::default().with_selection(EditorSelection::single(first, first))),
        update(Transaction::default().with_effects([install(&before)])),
        update(Transaction::default().with_effects([install(&same_rows)])),
    ]
    .each_ref()
    .map(same_tree);
    assert_eq!(json!(identity), identities["identity"]);

    let edit_at = offset(&identities["editAt"]);
    let changes = ChangeSet::single(Change::new(edit_at..edit_at + 1, "10")).unwrap();
    for (i, (language, strategy)) in [
        (&before, &old_app),
        (&same_rows, &old_app),
        (&after, &new_app),
    ]
    .into_iter()
    .enumerate()
    {
        let state = update(Transaction::new(changes.clone()).with_effects([install(language)]));
        assert_eq!(
            state.text().as_str(),
            identities["editedSource"].as_str().unwrap()
        );
        let snapshot = attached_language_snapshot(&state).unwrap();
        let (NativeSyntax::Wast(doc), NativeSyntax::Wast(original_doc)) =
            (snapshot.syntax(), original.syntax())
        else {
            panic!("expected WAST syntax");
        };
        let raw = language.parse(state.text().as_str()).unwrap();
        let mut pending = vec![raw.root()];
        for node in doc.nodes() {
            let raw = pending.pop().unwrap();
            assert_eq!(raw.range(), node.range());
            assert!(ptr::eq(raw.wast_node_type().unwrap(), node.node_type()));
            pending.extend(raw.children().rev());
            let expected = match node.kind().name() {
                "App" => strategy,
                "Keyword" => &keyword,
                _ => continue,
            };
            assert!(Arc::ptr_eq(node.prop(&INDENT).unwrap(), expected));
        }
        assert!(pending.is_empty());
        let same_row = ptr::eq(
            doc.root().children().next().unwrap().node_type(),
            original_doc.root().children().next().unwrap().node_type(),
        );
        let column = get_indentation(&state, first, IndentOptions::default(), None).unwrap();
        assert_eq!(
            json!([same_tree(&state), same_row, column]),
            identities["edited"][i]
        );
    }
    assert_eq!(initial.text().as_str(), source);
}

#[test]
fn state_services_win_and_explicit_syntax_uses_the_query_state() {
    let configured = |base| {
        let indent: WastIndentStrategy = Arc::new(move |cx| {
            Some(
                base + cx
                    .state()
                    .map_or(0, |state| state.selection().main().head()),
            )
        });
        wast_language().with_node_props(&[INDENT.add(move |t| t.is("App").then(|| indent.clone()))])
    };
    let source = "(func)";
    let slot = Compartment::new();
    let passed = Arc::new(AtomicUsize::new(0));
    let defer = indent_service({
        let passed = passed.clone();
        move |state, lines| {
            assert_eq!(state.selection().main().head(), 3);
            assert_eq!(lines.position(), 2);
            passed.fetch_add(1, Ordering::Relaxed);
            IndentDecision::Defer
        }
    });
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([
            configured(10).extension(),
            defer,
            slot.of(Extension::group([])),
        ]),
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    let explicit = EditorState::with_extensions(source, configured(20).extension()).unwrap();
    let explicit = attached_language_snapshot(&explicit).unwrap();
    let stale = EditorState::with_extensions("(call)", configured(30).extension()).unwrap();
    let stale = attached_language_snapshot(&stale).unwrap();
    let at = |state: &EditorState, syntax: Option<&NativeSyntax>| {
        get_indentation(state, 2, IndentOptions::default(), syntax).unwrap()
    };
    assert_eq!(at(&state, None), Some(13));
    assert_eq!(at(&state, Some(explicit.syntax())), Some(23));
    assert_eq!(at(&state, Some(stale.syntax())), Some(13));
    assert_eq!(
        explicit
            .syntax()
            .indentation(2, StreamSettings::default(), IndentOptions::default()),
        Some(20),
    );
    for (decision, expected) in [
        (IndentDecision::Column(6), Some(6)),
        (IndentDecision::Inherit, None),
    ] {
        let service = indent_service(move |_, _| decision);
        state
            .update(Transaction::default().with_effects([slot.reconfigure(service)]))
            .unwrap();
        assert_eq!(at(&state, Some(explicit.syntax())), expected);
    }
    assert_eq!(passed.load(Ordering::Relaxed), 5);
}

#[test]
fn continuation_bounds_callback_depth_and_repeated_calls() {
    let calls = Arc::new(AtomicUsize::new(0));
    let indent: WastIndentStrategy = Arc::new({
        let calls = calls.clone();
        move |cx| {
            calls.fetch_add(1, Ordering::Relaxed);
            let first = cx.continue_indent();
            let second = cx.continue_indent();
            first.or(second)
        }
    });
    let language = wast_language()
        .with_node_props(&[INDENT.add(move |t| t.is("App").then(|| indent.clone()))]);
    let depth = 2048;
    let source = format!("{}foo{}", "(".repeat(depth), ")".repeat(depth));
    let state = EditorState::with_extensions(source, language.extension()).unwrap();
    assert_eq!(
        get_indentation(&state, depth + 1, IndentOptions::default(), None),
        Ok(None)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 64);
    let state = EditorState::with_extensions("(foo)", language.extension()).unwrap();
    assert_eq!(
        get_indentation(&state, 2, IndentOptions::default(), None),
        Ok(Some(0))
    );
    assert_eq!(calls.load(Ordering::Relaxed), 65);
}
