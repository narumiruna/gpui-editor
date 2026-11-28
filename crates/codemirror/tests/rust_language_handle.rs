use std::{
    future::Future,
    ops::Range,
    pin::Pin,
    ptr,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, Text, Transaction,
    autocomplete::brackets::close_brackets_enabled,
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData, StreamSettings,
        brackets::bracket_matching_options,
        lezer::{
            CLOSED_BY, FOLD, GROUP, INDENT, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet,
            LrIndentContext, LrIndentStrategy, NodeProp, NodeType, OPENED_BY, ParseError,
            SyntaxNode,
        },
        rust::{RustDocument, RustNode},
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
        RustLanguage, SupportedDocument, attached_language_snapshot, json_language, rust_extension,
        rust_language,
    },
};
use serde_json::{Value, json};

fn fixture(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/rust_language_handle.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn visit<'a, T: Copy>(
    root: T,
    source: &str,
    range: impl Fn(T) -> Range<usize>,
    node_type: impl Fn(T) -> Option<&'a NodeType>,
    children: impl Fn(T) -> Vec<T>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut stack = vec![(root, 0)];
    assert_eq!(range(root), 0..source.len());
    while let Some((node, depth)) = stack.pop() {
        let ty = node_type(node).unwrap();
        let span = range(node);
        assert!(source.get(span.clone()).is_some(), "{span:?}");
        rows.push(json!([depth, ty.name(), ty.term(), span.start, span.end]));
        stack.extend(
            children(node)
                .into_iter()
                .rev()
                .map(|child| (child, depth + 1)),
        );
    }
    rows
}

fn raw_rows(root: SyntaxNode<'_>, source: &str) -> Vec<Value> {
    visit(
        root,
        source,
        SyntaxNode::range,
        SyntaxNode::node_type,
        |node| node.children().collect(),
    )
}

fn editor_rows(doc: &RustDocument) -> Vec<Value> {
    visit(
        doc.root(),
        doc.text().as_str(),
        RustNode::range,
        RustNode::node_type,
        |node| node.children().collect(),
    )
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn rust(doc: &SupportedDocument) -> &RustDocument {
    let NativeSyntax::Rust(doc) = doc.syntax() else {
        panic!("expected Rust syntax");
    };
    doc
}

fn failure(error: ParseError) -> Value {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("unexpected parse error: {error}");
    };
    json!({"offset": offset})
}

#[test]
fn raw_and_attached_trees_and_failures_match_the_pinned_rust_handle() {
    let stock = rust_language();
    let noop = stock.configure(None, None);
    let named = noop.configure(None, Some("Named Rust"));
    let strict = named.configure(Some(true), Some("Strict Rust"));
    let inherited = strict.configure(None, Some(""));
    let relaxed = inherited.configure(Some(false), Some("Relaxed Rust"));
    let handles = [stock, &noop, &named, &strict, &inherited, &relaxed];
    let types = fixture("types").next().unwrap();
    assert_eq!(json!(handles.map(RustLanguage::name)), types["names"]);
    for (index, handle) in handles.into_iter().enumerate() {
        assert!(handle.clone() == *handle);
        assert!(handles[..index].iter().all(|other| *other != handle));
        assert_eq!(handle.node_types().len(), 300);
        assert!(
            handle
                .node_types()
                .zip(stock.node_types())
                .all(|(a, b)| ptr::eq(a, b))
        );
        assert!(handle.data_facet() == stock.data_facet());
    }
    let native: Vec<_> = stock
        .node_types()
        .map(|ty| {
            let isolate = ty
                .prop(&ISOLATE)
                .copied()
                .flatten()
                .map(|isolate| match isolate {
                    Isolation::Auto => "auto",
                    Isolation::Ltr => "ltr",
                    Isolation::Rtl => "rtl",
                });
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
                isolate
            ])
        })
        .collect();
    let expected = types["types"].as_array().unwrap();
    assert_eq!(native.len(), expected.len());
    for (term, (actual, expected)) in native.iter().zip(expected).enumerate() {
        assert_eq!(actual, expected, "Rust grammar term {term}");
    }
    let root_type = stock.node_types().find(|ty| ty.is_top()).unwrap();
    assert_eq!(root_type.term(), 8);
    assert_eq!(root_type.name(), "SourceFile");
    for row in fixture("parse") {
        let source = row["source"].as_str().unwrap();
        for (handle, raw, editor) in [
            (stock, "raw", "editor"),
            (&strict, "strictRaw", "strictEditor"),
            (&inherited, "strictRaw", "strictEditor"),
            (&relaxed, "raw", "editor"),
        ] {
            let result = match handle.parse(source) {
                Ok(tree) => {
                    assert!(ptr::eq(tree.root().node_type().unwrap(), root_type));
                    json!({"tree": raw_rows(tree.root(), source)})
                }
                Err(error) => failure(error),
            };
            assert_eq!(result, row[raw], "{} {raw}", row["name"]);
            let result = match EditorState::with_extensions(source, handle.extension()) {
                Ok(state) => {
                    let doc = snapshot(&state);
                    let doc = rust(&doc);
                    assert!(ptr::eq(doc.root().node_type().unwrap(), root_type));
                    assert_eq!(doc.text().as_str(), source);
                    json!({"tree": editor_rows(doc)})
                }
                Err(ConfigurationError::StrictSyntax(error)) => failure(error),
                Err(error) => panic!("unexpected configuration error: {error:?}"),
            };
            assert_eq!(result, row[editor], "{} {editor}", row["name"]);
        }
    }
}

fn replace(range: Range<usize>, value: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, value)).unwrap())
}

fn reject(state: &mut EditorState, transaction: Transaction, expected: &Value) {
    let text = state.text().clone();
    let selection = state.selection().clone();
    let name = RustLanguage::active_name(state);
    let doc = snapshot(state);
    let EditError::Configuration(ConfigurationError::StrictSyntax(error)) =
        state.update(transaction).unwrap_err()
    else {
        panic!("expected a strict Rust rejection");
    };
    assert_eq!(failure(error), *expected);
    assert_eq!(state.text(), &text);
    assert_eq!(state.selection(), &selection);
    assert_eq!(RustLanguage::active_name(state), name);
    assert!(Arc::ptr_eq(&doc, &snapshot(state)));
}

#[test]
fn strict_rust_rolls_back_text_selection_and_parser_and_accepts_combined_repair() {
    let row = fixture("edit").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = byte(&row["at"]);
    let strict = rust_language().configure(Some(true), Some("Strict Rust"));
    let relaxed = strict.configure(Some(false), Some("Relaxed Rust"));
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let original = snapshot(&state);
    let retained = state.clone();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    assert!(Arc::ptr_eq(&original, &snapshot(&state)));
    let cut = || replace(at..at + 1, "");
    let strict_effect = || slot.reconfigure(strict.extension());
    reject(
        &mut state,
        cut().with_selection(EditorSelection::single(0, 0)),
        &row["rejected"],
    );
    state
        .update(cut().with_effects([slot.reconfigure(relaxed.extension())]))
        .unwrap();
    assert_eq!(state.text().as_str(), row["invalid"].as_str().unwrap());
    assert_eq!(
        RustLanguage::active_name(&state).as_deref(),
        Some(relaxed.name())
    );
    let recovered = snapshot(&state);
    assert!(rust(&recovered).is_recovered());
    assert_eq!(json!(editor_rows(rust(&recovered))), row["recovered"]);
    reject(
        &mut state,
        Transaction::default().with_effects([strict_effect()]),
        &row["rejected"],
    );
    let insert = |value| replace(at..at, value).with_effects([strict_effect()]);
    assert_eq!(row["proposed"], source.replace(';', "新"));
    reject(&mut state, insert("新"), &row["bad"]);
    state.update(insert(";")).unwrap();
    assert_eq!(
        RustLanguage::active_name(&state).as_deref(),
        Some(strict.name())
    );
    let repaired = snapshot(&state);
    assert_eq!(state.text().as_str(), source);
    assert_eq!(json!(editor_rows(rust(&repaired))), row["repaired"]);
    assert!(!rust(&repaired).is_recovered());
    assert!(!Arc::ptr_eq(&repaired, &original));
    assert!(Arc::ptr_eq(&snapshot(&retained), &original));
}

fn probe(state: &EditorState, at: usize, side: Assoc) -> Vec<&'static str> {
    state.language_data_at::<&str>("probe", at, side)
}

#[test]
fn stock_direct_and_catalog_attachments_share_identity_data_and_precedence() {
    let row = fixture("data").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let entry = LanguageDescription::match_name(LANGUAGES, "Rust", false).unwrap();
    let mut load = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(bundle)) = Pin::new(&mut load).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native Rust catalog support");
    };
    let stock = rust_language();
    assert!(ptr::eq(stock, bundle.rust_language().unwrap()));
    assert_eq!(bundle.completion_source_capacity(), 0);
    assert!(NativeLanguage::Yaml.support().rust_language().is_none());
    let catalog = bundle.extension().unwrap();
    let slot = Compartment::new();
    let named = stock.configure(None, Some("Named Rust"));
    let data = |label| LanguageData::default().with("probe", label);
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([
            named.data(data("normal")),
            stock.data(data("high")).with_precedence(Precedence::High),
            slot.of(rust_extension()),
        ]),
    )
    .unwrap();
    let original = snapshot(&state);
    let swap = |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    state.update(swap(catalog.clone())).unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &original));
    state
        .update(swap(Extension::group([
            rust_extension(),
            catalog,
            stock.clone().extension(),
        ])))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &original));
    assert!(bracket_matching_options(&state).unwrap().is_none());
    assert!(!close_brackets_enabled(&state).unwrap());
    for at in row["positions"].as_array().unwrap().iter().map(byte) {
        for side in [Assoc::Before, Assoc::After] {
            assert!(stock.is_active_at(&state, at, side));
            assert!(named.is_active_at(&state, at, side));
            assert_eq!(json!(probe(&state, at, side)), row["single"]);
            let comments = state.language_data_at::<CommentTokens>("commentTokens", at, side);
            assert_eq!(comments.len(), 1);
            assert_eq!(comments[0].line, row["comments"][0]["line"].as_str());
            assert_eq!(
                comments[0].block,
                Some((
                    row["comments"][0]["block"]["open"].as_str().unwrap(),
                    row["comments"][0]["block"]["close"].as_str().unwrap()
                ))
            );
            let brackets = state.language_data_at::<CloseBracketData>("closeBrackets", at, side);
            assert_eq!(brackets.len(), 1);
            assert_eq!(
                json!(brackets[0].string_prefixes),
                row["brackets"][0]["stringPrefixes"]
            );
            assert!(brackets[0].brackets.is_none() && brackets[0].before.is_none());
            let rules = state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
            assert_eq!(rules.len(), 1);
            for trigger in row["indent"].as_array().unwrap() {
                assert_eq!(
                    rules[0](trigger[0].as_str().unwrap()),
                    trigger[1].as_bool().unwrap()
                );
            }
        }
    }
    for at in [
        source.find('\n').unwrap(),
        source.find('🦀').unwrap() + 1,
        source.len() + 1,
    ] {
        assert!(!stock.is_active_at(&state, at, Assoc::After));
        assert!(probe(&state, at, Assoc::After).is_empty());
    }
    let regions = |handle: &RustLanguage, state: &EditorState| {
        json!(
            handle
                .find_regions(state)
                .into_iter()
                .map(|r| [r.start, r.end])
                .collect::<Vec<_>>()
        )
    };
    assert_eq!(regions(&named, &state), row["regions"]);
    state
        .update(swap(stock.configure(None, None).extension()))
        .unwrap();
    assert!(!Arc::ptr_eq(&snapshot(&state), &original));
    assert_eq!(
        editor_rows(rust(&snapshot(&state))),
        editor_rows(rust(&original))
    );
    state
        .update(swap(Extension::group([
            stock.configure(None, None).extension(),
            named.extension().with_precedence(Precedence::High),
        ])))
        .unwrap();
    assert_eq!(
        RustLanguage::active_name(&state).as_deref(),
        Some(named.name())
    );
    assert_eq!(json!(probe(&state, 0, Assoc::After)), row["distinct"]);
    let other = EditorState::with_extensions(
        "{}",
        Extension::group([
            named.extension(),
            json_language()
                .extension()
                .with_precedence(Precedence::High),
        ]),
    )
    .unwrap();
    assert!(RustLanguage::active_name(&other).is_none());
    assert!(!named.is_active_at(&other, 0, Assoc::After));
    assert!(named.find_regions(&other).is_empty());
}

#[test]
fn typed_rows_survive_edits_and_leave_state_free_rust_services_unchanged() {
    let row = fixture("props").next().unwrap();
    let source_row = fixture("data").next().unwrap();
    let source = source_row["source"].as_str().unwrap();
    let stock = rust_language();
    let mark = NodeProp::<u16>::new();
    let alternate = LanguageFacet::new(Some(LanguageData::default().with("probe", "alternate")));
    let marked = stock.with_node_props(&[
        mark.add(|ty| {
            if ty.is_top() {
                Some(0)
            } else {
                ty.is("Identifier").then(|| ty.term())
            }
        }),
        LANGUAGE_DATA.add({
            let alternate = alternate.clone();
            move |ty| ty.is_top().then(|| Some(alternate.clone()))
        }),
        // The state-free document helpers below do not use configured rows.
        FOLD.add(|ty| ty.is("Block").then_some(None)),
        INDENT.add(|ty| {
            ty.is("Block").then(|| {
                Some(Arc::new(|_: &LrIndentContext<'_>| Some(11usize)) as LrIndentStrategy)
            })
        }),
        CLOSED_BY.add(|ty| ty.is("{").then(|| vec![")".into()])),
    ]);
    assert!(marked.data_facet() == stock.data_facet());
    let types: Vec<_> = marked.node_types().collect();
    assert_eq!(
        json!(
            types
                .iter()
                .filter_map(|ty| ty
                    .is("Identifier")
                    .then(|| ty.prop(&mark).copied())
                    .flatten())
                .collect::<Vec<_>>()
        ),
        row["identifiers"]
    );
    let raw = marked.parse(source).unwrap();
    let root_ty = raw.root().node_type().unwrap();
    assert!(ptr::eq(root_ty, types[8]));
    assert_eq!(json!(root_ty.prop(&mark)), row["top"]);
    assert!(root_ty.prop(&LANGUAGE_DATA).and_then(Option::as_ref) == Some(&alternate));
    let stock_doc = RustDocument::parse(Text::new(source));
    let open = source.find('{').unwrap();
    let inner = source.find("  let").unwrap();
    let services = |doc: &RustDocument| {
        (
            doc.foldable(source.find("fn").unwrap(), open + 1),
            doc.indentation(inner, StreamSettings::default(), IndentOptions::default()),
            doc.bracket_near(open + 1).unwrap().matched,
        )
    };
    let expected = services(&stock_doc);
    assert!(expected.0.is_some() && expected.1 == Some(2) && expected.2);
    for strict in [false, true] {
        let handle = marked.configure(Some(strict), None);
        let mut state = EditorState::with_extensions(source, handle.extension()).unwrap();
        let verify = |state: &EditorState| {
            let doc = snapshot(state);
            let doc = rust(&doc);
            for node in doc.nodes() {
                let ty = node.node_type().unwrap();
                assert!(ptr::eq(ty, types[usize::from(ty.term())]));
                assert_eq!(node.kind().name(), ty.name());
            }
            assert!(ptr::eq(doc.root().node_type().unwrap(), root_ty));
            assert_eq!(services(doc), expected);
            assert_eq!(json!(probe(state, 0, Assoc::After)), row["probe"]);
            assert!(!handle.is_active_at(state, 0, Assoc::After));
            assert!(!stock.is_active_at(state, 0, Assoc::After));
            assert_eq!(
                json!(
                    handle
                        .find_regions(state)
                        .into_iter()
                        .map(|r| [r.start, r.end])
                        .collect::<Vec<_>>()
                ),
                row["regions"]
            );
        };
        verify(&state);
        let at = source.find('字').unwrap();
        state
            .update(replace(at..at + '字'.len_utf8(), "新"))
            .unwrap();
        verify(&state);
    }
    let disabled = marked.with_node_props(&[LANGUAGE_DATA.add(|ty| ty.is_top().then_some(None))]);
    let state = EditorState::with_extensions(source, disabled.extension()).unwrap();
    assert!(probe(&state, 0, Assoc::After).is_empty());
    assert!(!disabled.is_active_at(&state, 0, Assoc::After));
    assert!(
        disabled
            .find_regions(&state)
            .into_iter()
            .eq(std::iter::once(0..source.len()))
    );
}
