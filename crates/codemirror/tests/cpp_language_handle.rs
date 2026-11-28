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
        HighlightTag, IndentOptions, LanguageData, StreamSettings,
        brackets::{BracketConfig, bracket_matching_options},
        cpp::{CppDocument, CppNode},
        fold::foldable_in_state,
        lezer::{
            CLOSED_BY, FOLD, GROUP, INDENT, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet,
            LrIndentContext, LrIndentStrategy, NodeProp, OPENED_BY, ParseError, SyntaxNode,
            SyntaxTree, style_tags,
        },
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_with_parser,
        },
    },
    language_data::{
        CloseBracketData, CppLanguage, LANGUAGES, LanguageDescription, NativeLanguage,
        NativeLanguageSupport, NativeSyntax, SupportedDocument, attached_language_snapshot,
        cpp_extension, cpp_language, json_language,
    },
};
use serde_json::{Value, json};

#[path = "cpp_language_handle/configured_services.rs"]
mod configured_services;

fn fixture(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/cpp_language_handle.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn rows<N: Copy>(
    source: &str,
    root: N,
    span: impl Fn(N) -> Range<usize>,
    term: impl Fn(N) -> u16,
    children: impl Fn(N) -> Vec<N>,
) -> Value {
    let mut pending = vec![(root, 0)];
    let mut rows = Vec::new();
    assert_eq!(span(root), 0..source.len());
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

fn raw_rows(source: &str, tree: &SyntaxTree) -> Value {
    rows(
        source,
        tree.root(),
        SyntaxNode::range,
        SyntaxNode::term,
        |n| n.children().collect(),
    )
}

fn editor_rows(doc: &CppDocument) -> Value {
    rows(
        doc.text().as_str(),
        doc.root(),
        CppNode::range,
        |n| n.node_type().term(),
        |n| n.children().collect(),
    )
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn cpp(doc: &SupportedDocument) -> &CppDocument {
    let NativeSyntax::Cpp(doc) = doc.syntax() else {
        panic!("expected C++ syntax")
    };
    doc
}

fn failure(error: ParseError) -> Value {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("unexpected parse error: {error}")
    };
    json!({"offset": offset})
}

#[test]
fn stock_rows_and_configured_identities_match_the_pin() {
    let stock = cpp_language();
    let noop = stock.configure(None, None);
    let named = noop.configure(None, Some("Named C++"));
    let strict = named.configure(Some(true), Some("Strict C++"));
    let inherited = strict.configure(None, Some(""));
    let relaxed = inherited.configure(Some(false), Some("Relaxed C++"));
    let handles = [stock, &noop, &named, &strict, &inherited, &relaxed];
    let types = fixture("types").next().unwrap();
    assert_eq!(json!(handles.map(CppLanguage::name)), types["names"]);
    for (i, handle) in handles.into_iter().enumerate() {
        assert!(handle.clone() == *handle);
        assert!(handles[..i].iter().all(|other| *other != handle));
        assert_eq!(handle.node_types().len(), 351);
        assert!(
            handle
                .node_types()
                .zip(stock.node_types())
                .all(|(a, b)| ptr::eq(a, b))
        );
        assert!(handle.data_facet() == stock.data_facet());
    }
    let expected = types["rows"].as_array().unwrap();
    assert_eq!(stock.node_types().len(), expected.len());
    for (ty, row) in stock.node_types().zip(expected) {
        let isolate = ty
            .prop(&ISOLATE)
            .and_then(|value| *value)
            .map(|value| match value {
                Isolation::Auto => "auto",
                Isolation::Ltr => "ltr",
                Isolation::Rtl => "rtl",
            });
        let actual = json!([
            ty.term(),
            ty.name(),
            ty.is_top(),
            ty.is_skipped(),
            ty.is_error(),
            ty.is_anonymous(),
            ty.prop(&GROUP),
            ty.prop(&OPENED_BY),
            ty.prop(&CLOSED_BY),
            isolate,
            ty.prop(&LANGUAGE_DATA).and_then(Option::as_ref) == Some(stock.data_facet())
        ]);
        assert_eq!(&actual, row, "C++ grammar term {}", ty.term());
    }
}

#[test]
fn raw_and_attached_parses_use_their_own_byte_coordinates() {
    let stock = cpp_language();
    let strict = stock.configure(Some(true), Some("Strict C++"));
    let inherited = strict.configure(None, Some(""));
    let relaxed = inherited.configure(Some(false), None);
    let root = stock.node_types().find(|ty| ty.is_top()).unwrap();
    for row in fixture("parse") {
        if row["name"] == "valid beyond native budget" {
            continue;
        }
        let source = row["source"].as_str().unwrap();
        for (handle, raw, editor) in [
            (stock, "raw", "editor"),
            (&strict, "strictRaw", "strictEditor"),
            (&inherited, "strictRaw", "strictEditor"),
            (&relaxed, "raw", "editor"),
        ] {
            let actual = match handle.parse(source) {
                Ok(tree) => {
                    assert!(ptr::eq(tree.root().node_type().unwrap(), root));
                    raw_rows(source, &tree)
                }
                Err(error) => failure(error),
            };
            assert_eq!(actual, row[raw], "{} {raw}", row["name"]);
            let actual = match EditorState::with_extensions(source, handle.extension()) {
                Ok(state) => {
                    let doc = snapshot(&state);
                    let doc = cpp(&doc);
                    assert!(ptr::eq(doc.root().node_type(), root));
                    assert_eq!(doc.text().as_str(), source);
                    editor_rows(doc)
                }
                Err(ConfigurationError::StrictSyntax(error)) => failure(error),
                Err(error) => panic!("unexpected configuration error: {error:?}"),
            };
            assert_eq!(actual, row[editor], "{} {editor}", row["name"]);
        }
    }
}

#[test]
fn strict_cpp_reports_native_pruned_glr_limits_without_recovery() {
    // The pin accepts this source. The 17-repeat oracle in the preceding test
    // also rules out the smaller shared EditorStrict budget.
    let row = fixture("parse")
        .find(|row| row["name"] == "valid beyond native budget")
        .unwrap();
    let source = row["source"].as_str().unwrap();
    assert_eq!(row["strictRaw"]["rows"], row["strictEditor"]["rows"]);
    assert_eq!(row["strictRaw"]["rows"].as_array().unwrap().len(), 369);
    let strict = cpp_language().configure(Some(true), None);
    assert_eq!(
        strict.parse(source).unwrap_err(),
        ParseError::LimitExceeded { offset: 432 }
    );
    let source = &source.replace('\n', "\r\n");
    let error = EditorState::with_extensions(source, strict.extension())
        .err()
        .unwrap();
    assert_eq!(
        error,
        ConfigurationError::StrictSyntax(ParseError::LimitExceeded {
            offset: source.len()
        })
    );
    let state = EditorState::with_extensions(source, cpp_extension()).unwrap();
    let doc = snapshot(&state);
    let doc = cpp(&doc);
    assert!(doc.is_recovered());
    let error = doc.root().children().next().unwrap();
    assert!(error.node_type().is_error());
    assert_eq!(error.range(), 0..source.len());
}

fn replace(range: Range<usize>, value: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, value)).unwrap())
}

fn reject(state: &mut EditorState, transaction: Transaction, expected: &Value) {
    let old = snapshot(state);
    let selection = state.selection().clone();
    let name = CppLanguage::active_name(state);
    let EditError::Configuration(ConfigurationError::StrictSyntax(error)) =
        state.update(transaction).unwrap_err()
    else {
        panic!("expected strict C++ rejection")
    };
    assert_eq!(failure(error), *expected);
    assert_eq!(state.text(), old.text());
    assert_eq!(state.selection(), &selection);
    assert_eq!(CppLanguage::active_name(state), name);
    assert!(Arc::ptr_eq(&snapshot(state), &old));
}

#[test]
fn strict_cpp_rolls_back_rejected_edits_and_parser_switches() {
    let row = fixture("edit").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = byte(&row["at"]);
    let strict = cpp_language().configure(Some(true), Some("Strict C++"));
    let relaxed = strict.configure(Some(false), Some("Relaxed C++"));
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let retained = state.clone();
    let original = snapshot(&state);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &original));
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
    assert_eq!(
        CppLanguage::active_name(&state).as_deref(),
        Some(relaxed.name())
    );
    assert_eq!(state.text().as_str(), row["invalid"].as_str().unwrap());
    let recovered = snapshot(&state);
    assert!(cpp(&recovered).is_recovered());
    assert_eq!(editor_rows(cpp(&recovered)), row["recovered"]);
    reject(
        &mut state,
        Transaction::default().with_effects([strict_effect()]),
        &row["rejected"],
    );
    let insert = |value| replace(at..at, value).with_effects([strict_effect()]);
    reject(&mut state, insert("新"), &row["bad"]);
    state.update(insert(";")).unwrap();
    assert_eq!(
        CppLanguage::active_name(&state).as_deref(),
        Some(strict.name())
    );
    assert_eq!(state.text().as_str(), source);
    assert_eq!(editor_rows(cpp(&snapshot(&state))), row["repaired"]);
    assert!(Arc::ptr_eq(&snapshot(&retained), &original));
}

fn load(name: &str) -> Arc<NativeLanguageSupport> {
    let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let mut load = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(bundle)) = Pin::new(&mut load).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native C++ catalog support")
    };
    bundle
}

fn probe(state: &EditorState, at: usize, side: Assoc) -> Vec<&'static str> {
    state.language_data_at::<&str>("probe", at, side)
}

#[test]
fn direct_c_and_cpp_share_one_parser_and_metadata_provider() {
    let row = fixture("data").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let stock = cpp_language();
    let c = load("C");
    let cc = load("C++");
    assert!(!Arc::ptr_eq(&c, &cc));
    for (name, bundle) in [("C", &c), ("C++", &cc)] {
        assert!(Arc::ptr_eq(bundle, &load(name)));
        assert!(ptr::eq(bundle.cpp_language().unwrap(), stock));
        assert_eq!(bundle.completion_source_capacity(), 0);
    }
    assert!(NativeLanguage::Rust.support().cpp_language().is_none());
    let c = c.extension().unwrap();
    let cc = cc.extension().unwrap();
    let slot = Compartment::new();
    let named = stock.configure(None, Some("Named C++"));
    let probe_data = |label| LanguageData::default().with("probe", label);
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([
            named.data(probe_data("normal")),
            stock
                .data(probe_data("high"))
                .with_precedence(Precedence::High),
            slot.of(c.clone()),
        ]),
    )
    .unwrap();
    let original = snapshot(&state);
    let swap = |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    for next in [
        cc.clone(),
        cpp_extension(),
        Extension::group([c, cpp_extension(), cc, stock.extension()]),
    ] {
        state.update(swap(next)).unwrap();
        assert!(Arc::ptr_eq(&original, &snapshot(&state)));
    }
    assert!(bracket_matching_options(&state).unwrap().is_none());
    assert!(!close_brackets_enabled(&state).unwrap());
    for at in row["positions"].as_array().unwrap().iter().map(byte) {
        for side in [Assoc::Before, Assoc::After] {
            assert!(stock.is_active_at(&state, at, side));
            assert!(named.is_active_at(&state, at, side));
            assert_eq!(json!(probe(&state, at, side)), row["single"]);
            let comments = state.language_data_at::<CommentTokens>("commentTokens", at, side);
            let [comments] = comments.as_slice() else {
                panic!("one comment datum")
            };
            assert_eq!(comments.line, row["comments"][0]["line"].as_str());
            let (open, close) = comments.block.unwrap();
            assert_eq!(
                json!({"open": open, "close": close}),
                row["comments"][0]["block"]
            );
            let brackets = state.language_data_at::<CloseBracketData>("closeBrackets", at, side);
            let [brackets] = brackets.as_slice() else {
                panic!("one close-bracket datum")
            };
            assert_eq!(
                json!(brackets.string_prefixes),
                row["brackets"][0]["stringPrefixes"]
            );
            assert!(brackets.brackets.is_none() && brackets.before.is_none());
            let triggers = state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
            let [trigger] = triggers.as_slice() else {
                panic!("one indent-on-input datum")
            };
            for pair in row["indent"].as_array().unwrap() {
                assert_eq!(json!(trigger(pair[0].as_str().unwrap())), pair[1]);
            }
        }
    }
    for at in [
        source.find('\n').unwrap(),
        source.find('😀').unwrap() + 1,
        source.len() + 1,
    ] {
        assert!(!stock.is_active_at(&state, at, Assoc::After));
        assert!(probe(&state, at, Assoc::After).is_empty());
    }
    let noop = stock.configure(None, None);
    state.update(swap(noop.extension())).unwrap();
    let replaced = snapshot(&state);
    assert!(!Arc::ptr_eq(&replaced, &original));
    assert_eq!(editor_rows(cpp(&replaced)), editor_rows(cpp(&original)));
    state.update(swap(noop.clone().extension())).unwrap();
    assert!(Arc::ptr_eq(&replaced, &snapshot(&state)));
    state
        .update(swap(Extension::group([
            noop.extension(),
            named.extension().with_precedence(Precedence::High),
        ])))
        .unwrap();
    assert_eq!(
        CppLanguage::active_name(&state).as_deref(),
        Some(named.name())
    );
    assert_eq!(json!(probe(&state, 0, Assoc::After)), row["distinct"]);
    state.update(swap(json_language().extension())).unwrap();
    assert!(CppLanguage::active_name(&state).is_none());
    assert!(named.find_regions(&state).is_empty());
}

#[test]
fn root_props_change_data_and_presence_without_changing_stock_document_services() {
    let row = fixture("props").next().unwrap();
    let data = fixture("data").next().unwrap();
    let source = data["source"].as_str().unwrap();
    let stock = cpp_language();
    let mark = NodeProp::<u16>::new();
    let alternate = LanguageFacet::new(Some(LanguageData::default().with("probe", "alternate")));
    let marked = stock.with_node_props(&[
        mark.add(|ty| (ty.is_top() || ty.is("Identifier")).then(|| ty.term())),
        LANGUAGE_DATA.add(move |ty| ty.is_top().then(|| Some(alternate.clone()))),
        FOLD.add(|ty| ty.is("CompoundStatement").then_some(None)),
        INDENT.add(|ty| {
            ty.is("CompoundStatement").then(|| {
                Some(Arc::new(|_: &LrIndentContext<'_>| Some(17usize)) as LrIndentStrategy)
            })
        }),
        CLOSED_BY.add(|ty| ty.is("{").then(|| vec![")".into()])),
        style_tags([("Number", vec![HighlightTag::new("invalid")])]).unwrap(),
    ]);
    assert!(marked != *stock && marked.data_facet() == stock.data_facet());
    for (old, new) in stock.node_types().zip(marked.node_types()) {
        if new.prop(&mark).is_some()
            || matches!(new.prop(&FOLD), Some(None))
            || new.prop(&INDENT).is_some()
            || matches!(new.name(), "{" | "Number")
        {
            assert!(!ptr::eq(old, new));
        } else {
            assert!(ptr::eq(old, new));
        }
    }
    let root = marked.node_types().find(|ty| ty.is_top()).unwrap();
    assert_eq!(root.prop(&mark), Some(&307));
    let tree = marked.parse(source).unwrap();
    assert!(ptr::eq(tree.root().node_type().unwrap(), root));
    let mut pending = vec![tree.root()];
    let number = std::iter::from_fn(|| {
        let node = pending.pop()?;
        pending.extend(node.children());
        Some(node)
    })
    .find(|node| node.name() == "Number")
    .unwrap();
    assert_eq!(
        number.style_tags(&[]).unwrap().tags,
        [HighlightTag::new("invalid")]
    );
    let open = source.find('{').unwrap();
    let inner = source.find("  return").unwrap();
    let services = |doc: &CppDocument| {
        let spans: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| (span.range, span.tags.to_vec()))
            .collect();
        (
            doc.foldable(source.find("int").unwrap(), open + 1),
            doc.indentation(inner, StreamSettings::default(), IndentOptions::default()),
            spans,
        )
    };
    let expected = services(&CppDocument::parse(Text::new(source)));
    assert!(expected.0.is_some() && expected.1 == Some(2));
    let assert_marked = |state: &EditorState, handle: &CppLanguage| {
        let doc = snapshot(state);
        let doc = cpp(&doc);
        let rows: Vec<_> = handle.node_types().collect();
        for node in doc.nodes() {
            let ty = node.node_type();
            assert!(ptr::eq(ty, rows[usize::from(ty.term())]));
            assert_eq!(node.kind().name(), ty.name());
        }
        assert!(ptr::eq(doc.root().node_type(), root));
        assert_eq!(services(doc), expected);
        assert_eq!(
            foldable_in_state(state, source.find("int").unwrap(), open + 1),
            None
        );
        let bracket = doc
            .bracket_near_with_config(open + 1, BracketConfig::default())
            .unwrap();
        assert!(!bracket.matched);
        assert_eq!(bracket.start, open..open + 1);
        assert_eq!(json!(probe(state, 0, Assoc::After)), row["probe"]);
        let comments = state.language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After);
        assert!(comments.is_empty() && row["comments"] == json!([]));
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
    for handle in [marked.clone(), marked.configure(Some(true), None)] {
        let mut state = EditorState::with_extensions(source, handle.extension()).unwrap();
        assert_marked(&state, &handle);
        let at = source.find('字').unwrap();
        state
            .update(replace(at..at + '字'.len_utf8(), "新"))
            .unwrap();
        assert_marked(&state, &handle);
    }
    let absent = marked.with_node_props(&[LANGUAGE_DATA.add(|ty| ty.is_top().then_some(None))]);
    let state = EditorState::with_extensions(source, absent.extension()).unwrap();
    assert!(probe(&state, 0, Assoc::After).is_empty());
    assert!(!absent.is_active_at(&state, 0, Assoc::After));
    assert!(
        absent
            .find_regions(&state)
            .into_iter()
            .eq(std::iter::once(0..source.len()))
    );
}

#[test]
fn stock_facet_finds_both_markdown_aliases_at_original_endpoints() {
    let row = fixture("markdown").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let state = EditorState::with_extensions(
        source,
        markdown_with_parser(
            MarkdownConfig::default(),
            MarkdownParserConfig {
                code_languages: Some(MarkdownCodeLanguages::Catalog(LANGUAGES.into())),
                ..Default::default()
            },
        ),
    )
    .unwrap();
    let stock = cpp_language();
    let named = stock.configure(None, Some("Named C++"));
    assert!(CppLanguage::active_name(&state).is_none());
    let regions = stock.find_regions(&state);
    assert_eq!(named.find_regions(&state), regions);
    assert_eq!(
        json!(regions.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>()),
        row["regions"]
    );
    for (i, range) in regions.iter().enumerate() {
        for (j, at) in [range.start, range.end].into_iter().enumerate() {
            for (k, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
                assert_eq!(
                    json!(named.is_active_at(&state, at, side)),
                    row["presence"][i][j][k]
                );
            }
        }
    }
}
