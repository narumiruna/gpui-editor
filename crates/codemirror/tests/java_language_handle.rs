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
        LanguageData,
        brackets::bracket_matching_options,
        java::JavaDocument,
        lezer::{
            CLOSED_BY, GROUP, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet, OPENED_BY,
            ParseError, ReuseStats, SyntaxTree,
        },
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_with_parser,
        },
    },
    language_data::{
        CloseBracketData, JavaLanguage, JavaTop, LANGUAGES, LanguageDescription, NativeLanguage,
        NativeLanguageSupport, NativeSyntax, SupportedDocument, attached_language_snapshot,
        java_extension, java_language,
    },
};
use serde_json::{Value, json};

fn fixture(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/java_language_handle.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn top(value: &Value) -> JavaTop {
    match value.as_str().unwrap() {
        "Program" => JavaTop::Program,
        "ClassContent" => JavaTop::ClassContent,
        name => panic!("unexpected Java top: {name}"),
    }
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn java(doc: &SupportedDocument) -> &JavaDocument {
    let NativeSyntax::Java(doc) = doc.syntax() else {
        panic!("expected Java syntax")
    };
    doc
}

fn raw_rows(source: &str, tree: &SyntaxTree) -> Value {
    let mut pending = vec![(tree.root(), 0)];
    let mut rows = Vec::new();
    assert_eq!(tree.root().range(), 0..source.len());
    while let Some((node, depth)) = pending.pop() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some(), "{range:?}");
        rows.push((depth, node.term(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| (child, depth + 1)));
    }
    json!({"rows": rows})
}

fn editor_rows(doc: &JavaDocument) -> Value {
    let text = doc.text();
    assert_eq!(doc.root().range(), 0..text.len());
    let rows: Vec<_> = doc
        .nodes()
        .map(|node| {
            let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
            let range = node.range();
            assert!(text.slice(range.clone()).is_some(), "{range:?}");
            (depth, node.term(), range.start, range.end)
        })
        .collect();
    json!({"rows": rows})
}

fn failure(error: ParseError) -> Value {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("expected a Java syntax error, got {error}")
    };
    json!({"offset": offset})
}

#[test]
fn stock_rows_and_configured_identities_match_the_pin() {
    let stock = java_language();
    let noop = stock.configure(None, None, None);
    let members = stock.configure(Some(JavaTop::ClassContent), None, Some("Java Members"));
    let strict = stock.configure(None, Some(true), Some("Strict Java"));
    let strict_members = members.configure(None, Some(true), Some("Strict Members"));
    let inherited = strict_members.configure(None, None, Some(""));
    let relaxed = inherited.configure(None, Some(false), None);
    let switched = relaxed.configure(Some(JavaTop::Program), None, None);
    let handles = [
        stock,
        &noop,
        &members,
        &strict,
        &strict_members,
        &inherited,
        &relaxed,
        &switched,
    ];
    let types = fixture("types").next().unwrap();
    assert_eq!(json!(handles.map(JavaLanguage::name)), types["names"]);
    assert_eq!(
        json!(handles.map(|handle| handle.top().term())),
        types["tops"]
    );
    for (index, handle) in handles.into_iter().enumerate() {
        assert!(handle.clone() == *handle);
        assert!(handles[..index].iter().all(|other| *other != handle));
        assert_eq!(handle.node_types().len(), 240);
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
        assert_eq!(
            &json!([
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
                ty.prop(&LANGUAGE_DATA).and_then(Option::as_ref) == Some(stock.data_facet()),
            ]),
            row,
            "Java grammar term {}",
            ty.term()
        );
    }
}

fn strict_frontier(row: &Value) -> bool {
    let name = row["name"].as_str().unwrap();
    name.starts_with("adjacent ") && !name.starts_with("adjacent 11 ")
}

#[test]
fn raw_and_attached_parses_match_the_scoped_pin_in_original_bytes() {
    for row in fixture("parse") {
        let source = row["source"].as_str().unwrap();
        let top = top(&row["top"]);
        let handle = java_language().configure(Some(top), None, None);
        let strict = handle
            .configure(None, Some(true), None)
            .configure(None, None, Some(""));
        let relaxed = strict.configure(None, Some(false), None);
        let root = handle.node_types().nth(usize::from(top.term())).unwrap();
        let name = row["name"].as_str().unwrap();
        for (handle, raw, editor) in [
            (&handle, "rawStock", "editorStock"),
            (&strict, "strictRaw", "strictEditor"),
            (&relaxed, "rawStock", "editorStock"),
        ] {
            if raw == "strictRaw" && strict_frontier(&row) {
                continue;
            }
            let actual_raw = match handle.parse(source) {
                Ok(tree) => {
                    assert!(ptr::eq(tree.root().node_type().unwrap(), root));
                    raw_rows(source, &tree)
                }
                Err(error) => failure(error),
            };
            let actual_editor = match EditorState::with_extensions(source, handle.extension()) {
                Ok(state) => {
                    let snapshot = snapshot(&state);
                    let doc = java(&snapshot);
                    assert_eq!(doc.text().as_str(), source);
                    assert!(ptr::eq(doc.node_type(doc.root().term()).unwrap(), root));
                    editor_rows(doc)
                }
                Err(ConfigurationError::StrictSyntax(error)) => failure(error),
                Err(error) => panic!("unexpected configuration error: {error:?}"),
            };
            assert_eq!(actual_raw, row[raw], "{name} {raw}");
            assert_eq!(actual_editor, row[editor], "{name} {editor}");
        }
    }
}

#[test]
fn strict_java_keeps_its_native_generic_error_offsets() {
    for row in fixture("parse").filter(strict_frontier) {
        let source = row["source"].as_str().unwrap();
        let top = top(&row["top"]);
        let handle = java_language().configure(Some(top), Some(true), None);
        let raw = handle.parse(source).unwrap_err();
        let Some(ConfigurationError::StrictSyntax(editor)) =
            EditorState::with_extensions(source, handle.extension()).err()
        else {
            panic!("expected strict Java rejection");
        };
        // Native GLR reports these adjacent generic errors three bytes later.
        for (key, error) in [("strictRaw", raw), ("strictEditor", editor)] {
            assert_eq!(
                failure(error),
                json!({"offset": byte(&row[key]["offset"]) + 3}),
                "{} {key}",
                row["name"],
            );
        }
    }
}

#[test]
fn java_reports_native_work_limits_without_retry() {
    for row in fixture("budget") {
        let top = top(&row["top"]);
        let source = row["source"].as_str().unwrap();
        let (offset, upstream_rows) = match top {
            JavaTop::Program => (84, 117),
            JavaTop::ClassContent => (103, 87),
        };
        assert_eq!(row["count"], upstream_rows);
        let permissive = java_language().configure(Some(top), None, None);
        let strict = permissive.configure(None, Some(true), None);
        let limit = ParseError::LimitExceeded { offset };
        assert_eq!(strict.parse(source).unwrap_err(), limit);
        assert_eq!(permissive.parse(source).unwrap_err(), limit);
        // CRLF projection cannot move the reported offset into normalized bytes.
        let ParseError::LimitExceeded { offset } =
            strict.parse(&format!("\n\n{source}")).unwrap_err()
        else {
            panic!("expected a work limit after the normalized prefix");
        };
        let source = format!("\r\n\r\n{source}");
        let limit = ParseError::LimitExceeded { offset: offset + 2 };
        assert_eq!(
            EditorState::with_extensions(&source, strict.extension())
                .err()
                .unwrap(),
            ConfigurationError::StrictSyntax(limit),
        );
        let state = EditorState::with_extensions(&source, permissive.extension()).unwrap();
        let doc = snapshot(&state);
        let doc = java(&doc);
        assert_eq!(doc.parse_error(), Some(limit));
        let [marker] = doc
            .root()
            .children()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        assert_eq!(marker.term(), 0);
        assert_eq!(marker.range(), 0..source.len());
    }
}

fn replace(range: Range<usize>, value: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, value)).unwrap())
}

fn reject(state: &mut EditorState, transaction: Transaction, expected: &Value) {
    let old = snapshot(state);
    let selection = state.selection().clone();
    let name = JavaLanguage::active_name(state);
    let EditError::Configuration(ConfigurationError::StrictSyntax(error)) =
        state.update(transaction).unwrap_err()
    else {
        panic!("expected strict Java rejection")
    };
    assert_eq!(failure(error), *expected);
    assert_eq!(state.text(), old.text());
    assert_eq!(state.selection(), &selection);
    assert_eq!(JavaLanguage::active_name(state), name);
    assert!(Arc::ptr_eq(&snapshot(state), &old));
}

#[test]
fn strict_edits_and_reconfiguration_are_atomic() {
    let row = fixture("edit").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = byte(&row["at"]);
    let strict = java_language().configure(
        Some(JavaTop::ClassContent),
        Some(true),
        Some("Strict Members"),
    );
    let relaxed = strict.configure(None, Some(false), Some("Relaxed Members"));
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let retained = state.clone();
    let original = snapshot(&state);
    assert_eq!(
        JavaLanguage::active_name(&state).as_deref(),
        Some(strict.name())
    );
    let cut = || replace(at..at + 1, "");
    let strict_effect = || slot.reconfigure(strict.extension());
    reject(
        &mut state,
        cut().with_selection(EditorSelection::single(1, 1)),
        &row["rejected"],
    );
    state
        .update(cut().with_effects([slot.reconfigure(relaxed.extension())]))
        .unwrap();
    assert_eq!(
        JavaLanguage::active_name(&state).as_deref(),
        Some(relaxed.name())
    );
    let recovered = snapshot(&state);
    assert!(java(&recovered).is_recovered());
    assert_eq!(editor_rows(java(&recovered)), row["accepted"]);
    reject(
        &mut state,
        Transaction::default().with_effects([strict_effect()]),
        &row["rejected"],
    );
    state
        .update(replace(at..at, ";").with_effects([strict_effect()]))
        .unwrap();
    assert_eq!(state.text().as_str(), source);
    assert_eq!(
        JavaLanguage::active_name(&state).as_deref(),
        Some(strict.name())
    );
    assert_eq!(editor_rows(java(&snapshot(&state))), row["repaired"]);
    state
        .update(Transaction::default().with_effects([slot.reconfigure(java_extension())]))
        .unwrap();
    assert_eq!(editor_rows(java(&snapshot(&state))), row["topSwitch"]);
    assert!(Arc::ptr_eq(&snapshot(&retained), &original));
}

#[test]
fn attached_edits_reproject_crlf_and_keep_only_eligible_recovery_seeds() {
    let row = fixture("pair").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = byte(&row["splitAt"]);
    let mut state = EditorState::with_extensions(source, java_extension()).unwrap();
    let attached = snapshot(&state);
    assert_eq!(editor_rows(java(&attached)), row["joined"]);
    let raw = JavaDocument::parse(Text::new(source));
    let raw_comment = raw.highlights(0..source.len()).unwrap().next().unwrap();
    let editor_comment = java(&attached)
        .highlights(0..source.len())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(raw_comment.range, 0..at);
    assert_eq!(editor_comment.range, 0..at - 1);
    assert_eq!(raw_comment.tags, editor_comment.tags);
    state.update(replace(at..at, "// note")).unwrap();
    assert_eq!(editor_rows(java(&snapshot(&state))), row["split"]);
    state.update(replace(at..at + "// note".len(), "")).unwrap();
    assert_eq!(state.text().as_str(), source);
    assert_eq!(editor_rows(java(&snapshot(&state))), row["joined"]);

    let before = "// 🦀 seed\r\nclass Seed { int café = 42; }\r\n".repeat(12);
    let malformed = format!("{before}class Tail {{\r\n  int x = ;\r\n}}\r\n");
    for (padding, can_reuse) in [(0, true), (8000, false)] {
        let source = format!("{}{malformed}", "\r\n".repeat(padding));
        assert_eq!(source.len() <= 16 * 1024, can_reuse);
        assert!(source.replace("\r\n", "\n").len() <= 16 * 1024);
        let at = source.rfind(';').unwrap();
        let mut state = EditorState::with_extensions(&source, java_extension()).unwrap();
        let broken = snapshot(&state);
        assert_eq!(
            failure(java(&broken).parse_error().unwrap()),
            json!({"offset": at})
        );
        assert_eq!(broken.reuse_stats(), ReuseStats::default());
        state.update(replace(at..at, "7")).unwrap();
        let repaired = snapshot(&state);
        assert!(!java(&repaired).is_recovered());
        let reuse = repaired.reuse_stats();
        assert_eq!(reuse.resumed_at > 0, can_reuse);
        assert!(reuse.resumed_at <= at && source.is_char_boundary(reuse.resumed_at));
        if can_reuse {
            let legacy = JavaDocument::parse(Text::new(source.replace("\r\n", "\n")));
            let normalized_at = legacy.text().as_str().rfind(';').unwrap();
            let legacy = legacy
                .reparse(replace(normalized_at..normalized_at, "7").changes())
                .unwrap();
            let mut expected = legacy.reuse_stats().resumed_at;
            for (at, _) in state.text().as_str().match_indices("\r\n") {
                if at >= expected {
                    break;
                }
                expected += 1;
            }
            assert_eq!(reuse.resumed_at, expected);
        }
    }
}

fn load() -> Arc<NativeLanguageSupport> {
    let entry = LanguageDescription::match_filename(LANGUAGES, "Example.java").unwrap();
    let mut load = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(bundle)) = Pin::new(&mut load).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native Java catalog support")
    };
    bundle
}

fn probe(state: &EditorState, at: usize, side: Assoc) -> Vec<&'static str> {
    state.language_data_at::<&str>("probe", at, side)
}

#[test]
fn stock_factory_direct_and_catalog_share_one_parser_but_noop_is_distinct() {
    let stock = java_language();
    let noop = stock.configure(None, None, None);
    let bundle = load();
    assert!(Arc::ptr_eq(&bundle, &load()));
    assert!(ptr::eq(bundle.java_language().unwrap(), stock));
    assert!(ptr::eq(
        NativeLanguage::Java.support().java_language().unwrap(),
        stock
    ));
    assert!(NativeLanguage::Java.support().extension().is_none());
    assert!(NativeLanguage::Cpp.support().java_language().is_none());
    assert_eq!(bundle.completion_source_capacity(), 0);
    let catalog = bundle.extension().unwrap();
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions("// 字😀\r\nclass A {}", slot.of(catalog.clone())).unwrap();
    let original = snapshot(&state);
    let switch = |ext| Transaction::default().with_effects([slot.reconfigure(ext)]);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    for extension in [
        stock.extension(),
        java_extension(),
        Extension::group([catalog, java_extension(), stock.extension()]),
    ] {
        state.update(switch(extension)).unwrap();
        assert!(Arc::ptr_eq(&original, &snapshot(&state)));
    }
    assert!(bracket_matching_options(&state).unwrap().is_none());
    assert!(!close_brackets_enabled(&state).unwrap());
    assert_eq!(
        state
            .language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After)
            .len(),
        1
    );
    state.update(switch(noop.extension())).unwrap();
    let rebuilt = snapshot(&state);
    assert!(!Arc::ptr_eq(&original, &rebuilt));
    assert_eq!(editor_rows(java(&original)), editor_rows(java(&rebuilt)));
    state.update(switch(noop.clone().extension())).unwrap();
    assert!(Arc::ptr_eq(&rebuilt, &snapshot(&state)));
}

#[test]
fn selected_root_controls_data_and_presence_even_after_reparse() {
    let row = fixture("data").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let stock = java_language();
    let members = stock.configure(Some(JavaTop::ClassContent), None, Some("Java Members"));
    let alternate = LanguageFacet::new(Some(LanguageData::default().with("probe", "alternate")));
    let marked = members.with_node_props(&[LANGUAGE_DATA.add(move |ty| {
        (ty.term() == JavaTop::ClassContent.term()).then(|| Some(alternate.clone()))
    })]);
    assert!(marked.data_facet() == stock.data_facet());
    for (old, new) in stock.node_types().zip(marked.node_types()) {
        assert_eq!(
            ptr::eq(old, new),
            old.term() != JavaTop::ClassContent.term()
        );
    }
    let attach = |handle: &JavaLanguage| {
        EditorState::with_extensions(
            source,
            Extension::group([
                stock.data(LanguageData::default().with("probe", "normal")),
                members
                    .data(LanguageData::default().with("probe", "high"))
                    .with_precedence(Precedence::High),
                handle.extension(),
            ]),
        )
        .unwrap()
    };
    let state = attach(&members);
    for at in [
        source.find('\n').unwrap(),
        source.find('😀').unwrap() + 1,
        source.len() + 1,
    ] {
        assert!(!stock.is_active_at(&state, at, Assoc::After));
        assert!(probe(&state, at, Assoc::After).is_empty());
    }
    for (index, at) in row["positions"]
        .as_array()
        .unwrap()
        .iter()
        .map(byte)
        .enumerate()
    {
        for side in [Assoc::Before, Assoc::After] {
            assert!(stock.is_active_at(&state, at, side));
            assert!(members.is_active_at(&state, at, side));
            assert_eq!(json!(probe(&state, at, side)), row["data"][index]["probe"]);
            let comments = state.language_data_at::<CommentTokens>("commentTokens", at, side);
            let [comments] = comments.as_slice() else {
                panic!("expected Java comment data")
            };
            let (open, close) = comments.block.unwrap();
            assert_eq!(
                json!([{ "line": comments.line, "block": {"open": open, "close": close} }]),
                row["data"][index]["commentTokens"]
            );
            assert!(
                state
                    .language_data_at::<CloseBracketData>("closeBrackets", at, side)
                    .is_empty()
            );
            assert!(snapshot(&state).completions(at, true).is_empty());
            let triggers = state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
            let [trigger] = triggers.as_slice() else {
                panic!("expected Java trigger data")
            };
            for pair in row["triggers"].as_array().unwrap() {
                assert_eq!(json!(trigger(pair[0].as_str().unwrap())), pair[1]);
            }
        }
    }
    for strict in [false, true] {
        let marked = marked.configure(None, Some(strict), None);
        let mut state = attach(&marked);
        let at = source.find('字').unwrap();
        for _ in 0..2 {
            assert!(!stock.is_active_at(&state, at, Assoc::After));
            assert!(!marked.is_active_at(&state, at, Assoc::Before));
            assert_eq!(
                json!(probe(&state, at, Assoc::After)),
                row["marked"][0]["probe"]
            );
            assert!(
                state
                    .language_data_at::<CommentTokens>("commentTokens", at, Assoc::After)
                    .is_empty()
            );
            let ranges = marked.find_regions(&state);
            assert_eq!(
                json!(ranges.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>()),
                row["regions"]
            );
            state
                .update(replace(at..at + '字'.len_utf8(), "新"))
                .unwrap();
        }
    }
    let switched = marked.configure(Some(JavaTop::Program), None, None);
    let state = attach(&switched);
    assert!(marked.is_active_at(&state, 0, Assoc::After));
    assert_eq!(
        json!(probe(&state, 0, Assoc::After)),
        row["data"][0]["probe"]
    );
}

#[test]
fn stock_facet_finds_existing_java_fence_segments() {
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
    assert!(JavaLanguage::active_name(&state).is_none());
    let stock = java_language();
    let members = stock.configure(Some(JavaTop::ClassContent), None, None);
    let regions = stock.find_regions(&state);
    assert_eq!(members.find_regions(&state), regions);
    assert_eq!(
        json!(regions.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>()),
        row["regions"]
    );
    for (i, range) in regions.iter().enumerate() {
        for (j, at) in [range.start, range.end].into_iter().enumerate() {
            for (k, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
                assert_eq!(
                    json!(stock.is_active_at(&state, at, side)),
                    row["presence"][i][j][k]
                );
            }
        }
    }
}
