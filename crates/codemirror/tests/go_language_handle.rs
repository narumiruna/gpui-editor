use std::{
    future::Future,
    ops::Range,
    ptr,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, Transaction,
    autocomplete::brackets::close_brackets_enabled,
    commands::CommentTokens,
    language::{
        LanguageData,
        brackets::bracket_matching_options,
        go::{GoDocument, GoNode},
        lezer::{
            CLOSED_BY, GROUP, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet, OPENED_BY,
            ParseError, SyntaxNode, SyntaxTree,
        },
    },
    language_data::{
        CloseBracketData, GoCompletionSource, GoLanguage, LANGUAGES, LanguageDescription,
        NativeLanguage, NativeLanguageSupport, NativeSyntax, SupportedDocument,
        attached_language_snapshot, go_extension, go_language, go_language_extension,
        json_extension,
    },
};
use serde_json::{Value, json};

fn fixture(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/go_language_handle.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn go(document: &SupportedDocument) -> &GoDocument {
    let NativeSyntax::Go(go) = document.syntax() else {
        panic!("expected Go syntax");
    };
    go
}

fn rows<N: Copy>(
    root: N,
    source: &str,
    span: impl Fn(N) -> Range<usize>,
    term: impl Fn(N) -> u16,
    children: impl Fn(N) -> Vec<N>,
) -> Value {
    assert_eq!(span(root), 0..source.len());
    let mut pending = vec![(root, 0)];
    let mut rows = Vec::new();
    while let Some((node, depth)) = pending.pop() {
        let range = span(node);
        assert!(source.get(range.clone()).is_some(), "{range:?}");
        rows.push((depth, term(node), range.start, range.end));
        pending.extend(
            children(node)
                .into_iter()
                .rev()
                .map(|child| (child, depth + 1)),
        );
    }
    json!({"rows": rows})
}

fn raw_rows(tree: &SyntaxTree, source: &str) -> Value {
    rows(
        tree.root(),
        source,
        SyntaxNode::range,
        SyntaxNode::term,
        |node| node.children().collect(),
    )
}

fn editor_rows(doc: &GoDocument) -> Value {
    rows(
        doc.root(),
        doc.text().as_str(),
        GoNode::range,
        GoNode::term,
        |node| node.children().collect(),
    )
}

fn failure(error: ParseError) -> Value {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("unexpected parse error: {error}");
    };
    json!({"offset": offset})
}

fn edit(range: Range<usize>, value: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, value)).unwrap())
}

#[test]
fn stock_rows_and_configuration_match_the_pin() {
    let stock = go_language();
    let noop = stock.configure(None, None);
    let strict = noop.configure(Some(true), Some("Strict Go"));
    let inherited = strict.configure(None, Some(""));
    let relaxed = inherited.configure(Some(false), Some("Relaxed Go"));
    let handles = [stock, &noop, &strict, &inherited, &relaxed];
    let row = fixture("types").next().unwrap();
    assert_eq!(json!(handles.map(GoLanguage::name)), row["names"]);
    for (i, handle) in handles.into_iter().enumerate() {
        assert!(handle.clone() == *handle);
        assert!(handles[..i].iter().all(|previous| *previous != handle));
        assert_eq!(handle.node_types().len(), 176);
        assert!(
            handle
                .node_types()
                .zip(stock.node_types())
                .all(|(a, b)| ptr::eq(a, b))
        );
        assert!(handle.data_facet() == stock.data_facet());
    }
    let expected = row["rows"].as_array().unwrap();
    assert_eq!(expected.len(), 176);
    for (ty, row) in stock.node_types().zip(expected) {
        let isolation = ty
            .prop(&ISOLATE)
            .and_then(|value| *value)
            .map(|value| match value {
                Isolation::Auto => "auto",
                Isolation::Ltr => "ltr",
                Isolation::Rtl => "rtl",
            });
        assert_eq!(
            json!([
                ty.term(),
                ty.name(),
                ty.is_top(),
                ty.is_skipped(),
                ty.is_error(),
                ty.is_anonymous(),
                ty.prop(&GROUP),
                ty.prop(&OPENED_BY),
                ty.prop(&CLOSED_BY),
                isolation,
                ty.prop(&LANGUAGE_DATA).and_then(Option::as_ref) == Some(stock.data_facet()),
            ]),
            *row,
            "Go grammar term {}",
            ty.term()
        );
    }
}

#[test]
fn raw_and_attached_trees_and_strict_offsets_match_the_pin() {
    let stock = go_language();
    let strict = stock.configure(Some(true), None);
    let root = stock.node_types().find(|ty| ty.is_top()).unwrap();
    let compare = |handle: &GoLanguage, row: &Value, raw: &Value, editor: &Value| {
        let source = row["source"].as_str().unwrap();
        let actual = match handle.parse(source) {
            Ok(tree) => {
                assert!(ptr::eq(tree.root().node_type().unwrap(), root));
                raw_rows(&tree, source)
            }
            Err(error) => failure(error),
        };
        assert_eq!(actual, *raw, "{} raw", row["name"]);
        let actual = match EditorState::with_extensions(source, handle.extension()) {
            Ok(state) => {
                let snapshot = snapshot(&state);
                let doc = go(&snapshot);
                assert!(ptr::eq(doc.root().node_type(), root));
                assert_eq!(doc.text().as_str(), source);
                editor_rows(doc)
            }
            Err(ConfigurationError::StrictSyntax(error)) => failure(error),
            Err(error) => panic!("unexpected editor error: {error:?}"),
        };
        assert_eq!(actual, *editor, "{} attached", row["name"]);
    };
    assert_eq!(fixture("parse").count(), 17);
    for row in fixture("parse") {
        compare(stock, &row, &row["raw"], &row["editor"]);
        compare(&strict, &row, &row["strictRaw"], &row["strictEditor"]);
    }
    assert_eq!(fixture("package-test").count(), 66);
    for row in fixture("package-test") {
        let expected = json!({"rows": row["rows"]});
        compare(stock, &row, &expected, &expected);
        let expected = row["strictError"]
            .as_u64()
            .map_or(expected, |offset| json!({"offset": offset}));
        compare(&strict, &row, &expected, &expected);
    }
}

#[test]
fn strict_edits_and_reconfiguration_are_atomic() {
    let row = fixture("edit").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = byte(&row["at"]);
    let strict = go_language().configure(Some(true), Some("Strict Go"));
    let inherited = strict.configure(None, Some(""));
    let relaxed = inherited.configure(Some(false), Some("Relaxed Go"));
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let retained = state.clone();
    let old = snapshot(&state);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    assert!(Arc::ptr_eq(&old, &snapshot(&state)));
    let reject = |state: &mut EditorState, transaction: Transaction, expected: &Value| {
        let prior = snapshot(state);
        let selection = state.selection().clone();
        let name = GoLanguage::active_name(state);
        let EditError::Configuration(ConfigurationError::StrictSyntax(error)) =
            state.update(transaction).unwrap_err()
        else {
            panic!("expected strict Go rejection");
        };
        assert_eq!(failure(error), *expected);
        assert_eq!(state.selection(), &selection);
        assert_eq!(state.text(), prior.text());
        assert_eq!(GoLanguage::active_name(state), name);
        assert!(Arc::ptr_eq(&prior, &snapshot(state)));
    };
    let cut = || edit(at..at + 1, "");
    reject(
        &mut state,
        cut().with_selection(EditorSelection::single(0, 0)),
        &row["rejected"],
    );
    state
        .update(cut().with_effects([slot.reconfigure(relaxed.extension())]))
        .unwrap();
    assert_eq!(
        GoLanguage::active_name(&state).as_deref(),
        Some(relaxed.name())
    );
    assert_eq!(state.text().as_str(), row["invalid"]);
    assert_eq!(editor_rows(go(&snapshot(&state))), row["recovered"]);
    assert!(go(&snapshot(&state)).is_recovered());
    let strict_effect = || slot.reconfigure(inherited.extension());
    reject(
        &mut state,
        Transaction::default().with_effects([strict_effect()]),
        &row["rejected"],
    );
    let repair = |value| edit(at..at, value).with_effects([strict_effect()]);
    reject(&mut state, repair("@"), &row["bad"]);
    state.update(repair("}")).unwrap();
    assert_eq!(
        GoLanguage::active_name(&state).as_deref(),
        Some(strict.name())
    );
    assert_eq!(state.text().as_str(), source);
    assert_eq!(editor_rows(go(&snapshot(&state))), row["repaired"]);
    assert_eq!(snapshot(&state).reuse_stats(), Default::default());
    assert!(Arc::ptr_eq(&snapshot(&retained), &old));
}

#[test]
fn checked_limits_do_not_publish_strict_trees_or_raw_editor_fallback() {
    let stock = go_language();
    let strict = stock.configure(Some(true), None);
    for (row, (name, offset, pinned_rows)) in
        fixture("limit").zip([("branch", 27, 52), ("work", 36, 49)])
    {
        assert_eq!(row["name"], name);
        assert_eq!(row["pinnedRows"], pinned_rows);
        let source = row["source"].as_str().unwrap();
        let limit = ParseError::LimitExceeded { offset };
        assert_eq!(strict.parse(source).unwrap_err(), limit);
        assert_eq!(stock.parse(source).unwrap_err(), limit);
        assert_eq!(
            EditorState::with_extensions(source, strict.extension()).err(),
            Some(ConfigurationError::StrictSyntax(limit))
        );
        let state = EditorState::with_extensions(source, stock.extension()).unwrap();
        let snapshot = snapshot(&state);
        let doc = go(&snapshot);
        assert!(doc.is_recovered());
        assert_eq!(
            editor_rows(doc),
            json!({"rows": [[0, 3, 0, source.len()], [1, 0, 0, source.len()]]})
        );
    }
    let row = fixture("depth").next().unwrap();
    let depth = byte(&row["depth"]);
    let source = format!("var x = {}1{};\n", "(".repeat(depth), ")".repeat(depth));
    assert_eq!(
        failure(strict.parse(&source).unwrap_err()),
        row["strictRaw"]
    );
    let Err(ConfigurationError::StrictSyntax(error)) =
        EditorState::with_extensions(&source, strict.extension())
    else {
        panic!("expected a checked Go depth failure");
    };
    assert_eq!(failure(error), row["strictEditor"]);
    assert!(stock.parse(&source).is_ok());

    let row = fixture("fallback").next().unwrap();
    let source = row["source"].as_str().unwrap();
    assert_eq!(failure(strict.parse(source).unwrap_err()), row["strictRaw"]);
    let Err(ConfigurationError::StrictSyntax(error)) =
        EditorState::with_extensions(source, strict.extension())
    else {
        panic!("expected a checked Go syntax failure");
    };
    assert_eq!(failure(error), row["strictEditor"]);
    assert_eq!(
        raw_rows(&stock.parse(source).unwrap(), source)["rows"]
            .as_array()
            .unwrap()
            .len(),
        byte(&row["rawRows"])
    );
    let normalized = source.replace("\r\n", "\n");
    assert_eq!(
        stock.parse(&normalized).unwrap_err(),
        ParseError::LimitExceeded {
            offset: normalized.len()
        }
    );
    let state = EditorState::with_extensions(source, go_language_extension()).unwrap();
    let snapshot = snapshot(&state);
    let doc = go(&snapshot);
    assert!(doc.is_recovered());
    assert_eq!(doc.nodes().len(), 17);
    assert_eq!(byte(&row["editorRows"]), 234);
    assert_eq!(
        doc.root().children().map(GoNode::term).collect::<Vec<_>>(),
        [4, 111, 151, 0]
    );
    assert_eq!(
        doc.nodes()
            .filter(|node| node.node_type().is_error())
            .map(GoNode::range)
            .collect::<Vec<_>>(),
        [35..35, 35..314]
    );
    assert_eq!(snapshot.reuse_stats(), Default::default());
}

fn load_go() -> Arc<NativeLanguageSupport> {
    let desc = LanguageDescription::match_name(LANGUAGES, "Go", false).unwrap();
    let mut load = std::pin::pin!(desc.catalog_support_load().unwrap().load());
    let Poll::Ready(Ok(bundle)) = load.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native Go catalog support");
    };
    bundle
}

fn sources(state: &EditorState, at: usize) -> Vec<GoCompletionSource> {
    state.language_data_at("autocomplete", at, Assoc::Before)
}

#[test]
fn the_cached_catalog_keeps_two_ordered_callable_sources_and_the_stock_parser() {
    let row = fixture("factory").next().unwrap();
    let desc = LanguageDescription::match_name(LANGUAGES, "Go", false).unwrap();
    assert_eq!(desc.name, row["name"]);
    assert_eq!(json!(desc.aliases), row["alias"]);
    assert_eq!(json!(desc.extensions), row["extensions"]);
    assert!(desc.filename.is_none() && row["filename"].is_null());
    let bundle = load_go();
    assert!(Arc::ptr_eq(&bundle, &load_go()));
    assert!(ptr::eq(bundle.go_language().unwrap(), go_language()));
    assert!(NativeLanguage::Go.support().extension().is_none());
    assert!(NativeLanguage::Json.support().go_language().is_none());
    assert_eq!(bundle.completion_source_capacity(), 2);
    let catalog = bundle.extension().unwrap();
    let repeated = [
        go_language_extension(),
        catalog.clone(),
        Extension::group([catalog.clone(), bundle.extension().unwrap()]),
        Extension::group([go_extension(), go_extension()]),
    ];
    assert_eq!(
        json!(repeated.map(|extension| {
            sources(&EditorState::with_extensions("", extension).unwrap(), 0).len()
        })),
        row["repeated"]
    );

    let row = fixture("completion").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(source, slot.of(go_language_extension())).unwrap();
    assert!(sources(&state, 0).is_empty());
    assert_eq!(
        state
            .language_data_at::<CommentTokens>("commentTokens", 0, Assoc::Before)
            .len(),
        1
    );
    assert!(
        snapshot(&state)
            .indexed_completions(0, true)
            .next()
            .is_some()
    );
    let initial = snapshot(&state);
    let select = |ext| Transaction::default().with_effects([slot.reconfigure(ext)]);
    for extension in [
        go_language().extension(),
        go_extension(),
        catalog.clone(),
        go_extension(),
        catalog.clone(),
    ] {
        state.update(select(extension)).unwrap();
        assert!(Arc::ptr_eq(&snapshot(&state), &initial));
    }
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &initial));
    assert!(bracket_matching_options(&state).unwrap().is_none());
    assert!(!close_brackets_enabled(&state).unwrap());
    let [fixed, local]: [GoCompletionSource; 2] = sources(&state, 0).try_into().unwrap();
    let result = fixed(&state, 0, true).unwrap();
    let options: Vec<_> = result
        .options
        .iter()
        .map(|option| {
            json!([
                option.label,
                option.detail,
                option.kind,
                initial.completion_snippet(0, option).is_some()
            ])
        })
        .collect();
    assert_eq!(json!(options), row["options"]);
    for point in row["points"].as_array().unwrap() {
        let at = byte(&point[0]);
        for (index, explicit) in [false, true].into_iter().enumerate() {
            let values = [fixed, local].map(|provider| provider(&state, at, explicit));
            let actual = json!([
                values[0].as_ref().map(|r| json!([r.from, r.options.len()])),
                values[1].as_ref().map(|r| json!([
                    r.from,
                    r.options
                        .iter()
                        .map(|o| (&o.kind, &o.label))
                        .collect::<Vec<_>>()
                ]))
            ]);
            assert_eq!(
                actual,
                point[index + 1],
                "completion at {at}, explicit={explicit}"
            );
        }
    }
    // No projected editor position corresponds to the second byte of CRLF.
    let midpoint = source.find("\r\n").unwrap() + 1;
    assert_eq!(sources(&state, midpoint).len(), 2);
    assert!(fixed(&state, midpoint, true).is_some());
    let interior = source.find('😀').unwrap() + 1;
    assert!(sources(&state, interior).is_empty());
    assert!(fixed(&state, interior, true).is_none());
    assert!(local(&state, interior, true).is_none());

    let late = source.find("late").unwrap();
    state.update(edit(late..late + 4, "inner")).unwrap();
    assert!(!Arc::ptr_eq(&snapshot(&state), &initial));
    let check_edited = |state: &EditorState| {
        let doc = snapshot(state);
        let at = state.text().as_str().rfind("café").unwrap() + "café".len();
        let [fixed, local]: [GoCompletionSource; 2] = sources(state, at).try_into().unwrap();
        let fixed = fixed(state, at, true).unwrap();
        assert!(doc.completion_snippet(at, &fixed.options[0]).is_some());
        let local = local(state, at, true).unwrap();
        assert!(local.options.iter().any(|option| option.label == "inner"));
        assert!(local.options.iter().all(|option| option.label != "late"));
    };
    check_edited(&state);
    state
        .update(select(Extension::group([
            catalog.clone(),
            json_extension().with_precedence(Precedence::High),
        ])))
        .unwrap();
    assert!(sources(&state, 0).is_empty());
    assert!(fixed(&state, 0, true).is_none());
    assert!(!go_language().is_active_at(&state, 0, Assoc::After));
    state.update(select(catalog)).unwrap();
    check_edited(&state);
    let previous = snapshot(&state);
    state
        .update(select(go_language().configure(None, None).extension()))
        .unwrap();
    assert!(!Arc::ptr_eq(&snapshot(&state), &previous));
    assert!(sources(&state, 0).is_empty());
    state.update(select(Extension::default())).unwrap();
    assert!(attached_language_snapshot(&state).is_none());
    assert!(go_language().find_regions(&state).is_empty());
}

#[test]
fn effective_root_data_preserves_order_and_configured_rows_across_edits() {
    let row = fixture("data").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let stock = go_language();
    let alternate = LanguageFacet::new(Some(LanguageData::default().with("probe", "alternate")));
    let marked = stock.with_node_props(&[LANGUAGE_DATA.add({
        let alternate = alternate.clone();
        move |ty| ty.is_top().then(|| Some(alternate.clone()))
    })]);
    assert!(&marked != stock);
    assert!(marked.data_facet() == stock.data_facet());
    let changed: Vec<_> = stock
        .node_types()
        .zip(marked.node_types())
        .filter_map(|(a, b)| (!ptr::eq(a, b)).then_some(b.term()))
        .collect();
    assert_eq!(changed, [3]);
    let marked_root = marked.node_types().find(|ty| ty.is_top()).unwrap();
    let raw = marked.parse("package p\n").unwrap();
    assert!(ptr::eq(raw.root().node_type().unwrap(), marked_root));
    let make = |lang: &GoLanguage| {
        EditorState::with_extensions(
            source,
            Extension::group([
                lang.extension(),
                stock.data(LanguageData::default().with("probe", "normal")),
                marked
                    .data(LanguageData::default().with("probe", "high"))
                    .with_precedence(Precedence::High),
                alternate.of(LanguageData::default().with("probe", "added alternate")),
            ]),
        )
        .unwrap()
    };
    let state = make(stock);
    let mut configured = make(&marked);
    let probe =
        |state: &EditorState, at, side| json!(state.language_data_at::<&str>("probe", at, side));
    for at in row["positions"].as_array().unwrap().iter().map(byte) {
        for side in [Assoc::Before, Assoc::After] {
            assert!(stock.is_active_at(&state, at, side));
            assert!(marked.is_active_at(&state, at, side));
            assert!(!stock.is_active_at(&configured, at, side));
            assert!(!marked.is_active_at(&configured, at, side));
            assert_eq!(probe(&state, at, side), row["stock"]);
            assert_eq!(probe(&configured, at, side), row["marked"]);
            let comments = |state: &EditorState| {
                state
                    .language_data_at::<CommentTokens>("commentTokens", at, side)
                    .iter()
                    .map(|entry| {
                        let (open, close) = entry.block.unwrap();
                        json!({"line": entry.line, "block": {"open": open, "close": close}})
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(json!(comments(&state)), row["comments"]);
            assert_eq!(json!(comments(&configured)), row["markedComments"]);
            let data = state.language_data_at::<CloseBracketData>("closeBrackets", at, side);
            assert_eq!(data.len(), 1);
            assert_eq!(json!(data[0].brackets), row["brackets"][0]["brackets"]);
            assert!(data[0].before.is_none() && data[0].string_prefixes.is_empty());
            assert!(
                state
                    .language_data_at::<&str>("wordChars", at, side)
                    .is_empty()
            );
        }
    }
    let [trigger]: [fn(&str) -> bool; 1] = state
        .language_data_at::<fn(&str) -> bool>("indentOnInput", 0, Assoc::After)
        .try_into()
        .unwrap();
    for entry in row["indent"].as_array().unwrap() {
        assert_eq!(json!(trigger(entry[0].as_str().unwrap())), entry[1]);
    }
    assert_eq!(
        json!(
            stock
                .find_regions(&configured)
                .iter()
                .map(|r| [r.start, r.end])
                .collect::<Vec<_>>()
        ),
        row["regions"]
    );
    let at = source.find('π').unwrap();
    configured
        .update(edit(at..at + 'π'.len_utf8(), "λ"))
        .unwrap();
    let next = snapshot(&configured);
    assert!(ptr::eq(go(&next).root().node_type(), marked_root));
    assert_eq!(probe(&configured, 0, Assoc::After), row["marked"]);
    assert_eq!(probe(&state, at + 1, Assoc::After), json!([]));
    assert!(!stock.is_active_at(&state, at + 1, Assoc::After));
    let midpoint = source.find("\r\n").unwrap() + 1;
    assert_eq!(probe(&state, midpoint, Assoc::After), row["stock"]);
    assert!(stock.is_active_at(&state, midpoint, Assoc::After));

    let absent = marked.with_node_props(&[LANGUAGE_DATA.add(|ty| ty.is_top().then_some(None))]);
    let state = make(&absent);
    assert_eq!(probe(&state, 0, Assoc::After), json!([]));
    assert!(!stock.is_active_at(&state, 0, Assoc::After));
    assert_eq!(
        stock.find_regions(&state),
        std::iter::once(0..source.len()).collect::<Vec<_>>()
    );

    let strict = marked.configure(Some(true), None);
    let mut state = EditorState::with_extensions("var café = 1\r\n", strict.extension()).unwrap();
    state.update(edit(0..0, "package p\r\n")).unwrap();
    assert!(ptr::eq(
        go(&snapshot(&state)).root().node_type(),
        marked_root
    ));
    assert_eq!(probe(&state, 0, Assoc::After), json!(["alternate"]));
}

#[test]
fn tracked_checkpoints_survive_repair_and_crlf_split_join() {
    let mut source: String = (0..20)
        .map(|n| format!("var prefix{n} = {n}\r\n"))
        .collect();
    source.push_str("func f(param int) {\r\n\tvar café = 2\r\n\treturn café\r\n}\r\n");
    let mut state = EditorState::with_extensions(&source, go_language_extension()).unwrap();
    let clean = snapshot(&state);
    let expected = editor_rows(go(&clean));
    let close = source.len() - "}\r\n".len();
    state.update(edit(close..source.len(), "")).unwrap();
    let broken = snapshot(&state);
    assert!(go(&broken).is_recovered());
    assert_eq!(broken.reuse_stats(), Default::default());
    state.update(edit(close..close, "}\r\n")).unwrap();
    let repaired = snapshot(&state);
    assert!(!go(&repaired).is_recovered());
    assert_eq!(editor_rows(go(&repaired)), expected);
    assert!(repaired.reuse_stats().nodes_reused > 0);
    assert!((64..close).contains(&repaired.reuse_stats().resumed_at));

    let cr = source[..close].rfind("\r\n").unwrap();
    for (range, insert) in [(cr + 1..cr + 2, ""), (cr + 1..cr + 1, "\n")] {
        state.update(edit(range, insert)).unwrap();
        let current = snapshot(&state);
        let fresh = GoDocument::parse(state.text().clone());
        assert_eq!(editor_rows(go(&current)), editor_rows(&fresh));
        let reuse = current.reuse_stats();
        assert!(reuse.nodes_reused > 0);
        assert!(reuse.resumed_at <= cr && state.text().as_str().is_char_boundary(reuse.resumed_at));
    }
    assert_eq!(state.text().as_str(), source);
    assert_eq!(editor_rows(go(&snapshot(&state))), expected);
}
