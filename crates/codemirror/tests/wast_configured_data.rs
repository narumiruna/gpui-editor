use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, Compartment, EditorSelection, EditorState,
    Extension, Precedence, Transaction, change_filter,
    language::{
        LanguageData,
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_snapshot,
            markdown_with_parser,
        },
        wast::{
            LANGUAGE_DATA, SUBLANGUAGES, WastDataFacet, WastNode, WastNodeKind, WastSublanguage,
            WastSublanguageMode,
        },
    },
    language_data::{
        CloseBracketData, NativeLanguage, NativeSyntax, SupportedDocument,
        attached_language_snapshot, json_extension, wast_language,
    },
    view::EditorView,
};
use serde_json::{Value, json};

fn mark(value: &'static str) -> LanguageData {
    LanguageData::default().with("scope", value)
}

fn keyword(node: WastNode<'_>) -> bool {
    node.kind() == WastNodeKind::Keyword
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn configured_data_matches_the_pinned_synchronous_queries() {
    let mut rows = include_str!("fixtures/wast_configured_data.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .peekable();
    let original = rows.peek().unwrap()["source"].as_str().unwrap().to_owned();
    let stock = wast_language();
    let root = WastDataFacet::new(Some(mark("root-default")));
    let sub = WastDataFacet::new(Some(mark("sub-default")));
    let replace = WastDataFacet::new(None);
    let calls = Arc::new(AtomicUsize::new(0));
    let rules = vec![
        WastSublanguage::new(WastSublanguageMode::Extend, sub.clone(), {
            let calls = calls.clone();
            move |node, state| {
                calls.fetch_add(1, Ordering::Relaxed);
                keyword(node)
                    && state.text().slice(node.range()) == Some("module")
                    && state.selection().main().head() == 0
            }
        }),
        WastSublanguage::new(WastSublanguageMode::Replace, replace.clone(), |node, _| {
            keyword(node)
        }),
        WastSublanguage::new(WastSublanguageMode::Extend, sub.clone(), |node, _| {
            assert!(
                !keyword(node),
                "the first matching rule must stop the search"
            );
            false
        }),
    ];
    let stock_rules =
        stock.with_node_props(&[SUBLANGUAGES.add(move |row| row.is_top().then(|| rules.clone()))]);
    let configured = |facet: Option<WastDataFacet>| {
        stock_rules
            .with_node_props(&[LANGUAGE_DATA.add(move |row| row.is_top().then(|| facet.clone()))])
    };
    let overridden = configured(Some(root.clone()));
    let empty = configured(Some(WastDataFacet::new(None)));
    let absent = configured(None);
    let raw = overridden.parse("(module)").unwrap();
    assert!(raw.root().prop(&LANGUAGE_DATA).unwrap().as_ref() == Some(&root));
    assert_eq!(raw.root().prop(&SUBLANGUAGES).unwrap().len(), 3);
    let raw = absent.parse("(module)").unwrap();
    assert!(raw.root().prop(&LANGUAGE_DATA).is_some_and(Option::is_none));
    for language in [&stock_rules, &overridden, &empty, &absent] {
        assert!(language.data_facet() == stock.data_facet());
    }
    assert!(root != *stock.data_facet());
    assert!(replace != WastDataFacet::new(None));

    let data = Extension::group([
        overridden.data(mark("stock-input")),
        root.of(mark("root-low")),
        root.clone()
            .of(mark("root-high"))
            .with_precedence(Precedence::High),
        sub.of(mark("sub-input")),
        WastDataFacet::new(None).of(mark("unrelated")),
    ]);
    let parser = Compartment::new();
    let extras = Compartment::new();
    let mut state = EditorState::with_extensions(
        &original,
        Extension::group([
            parser.of(stock.extension()),
            extras.of(Extension::default()),
            data.clone(),
        ]),
    )
    .unwrap();
    let mut previous: Option<Arc<SupportedDocument>> = None;
    let mut capture = |step, state: &EditorState| {
        let row = rows.next().unwrap();
        assert_eq!(row["step"], step);
        assert_eq!(row["source"], state.text().as_str());
        assert_eq!(row["head"], state.selection().main().head());
        let snapshot = attached_language_snapshot(state)
            .or_else(|| markdown_snapshot(state))
            .unwrap();
        assert_eq!(snapshot.text(), state.text());
        let root = match snapshot.syntax() {
            NativeSyntax::Wast(doc) => doc.root().kind().name(),
            NativeSyntax::Json(doc) => doc.root().kind().name(),
            NativeSyntax::Markdown(_) => "Document",
            _ => unreachable!(),
        };
        assert_eq!(row["root"], root, "{step}");
        assert_eq!(
            row["reused"],
            previous
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, &snapshot)),
            "{step}"
        );
        previous = Some(snapshot);
        let regions: Vec<_> = overridden
            .find_regions(state)
            .into_iter()
            .map(|range| [range.start, range.end])
            .collect();
        assert_eq!(row["regions"], json!(regions), "{step}");
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "{step}: predicates run only on demand"
        );
        for probe in row["probes"].as_array().unwrap() {
            let pos = number(&probe[0]);
            let side = if probe[1] == -1 {
                Assoc::Before
            } else {
                Assoc::After
            };
            let active = overridden.is_active_at(state, pos, side);
            assert_eq!(active, stock.is_active_at(state, pos, side));
            let data = state.language_data_at::<&str>("scope", pos, side);
            let brackets: Vec<_> = state
                .language_data_at::<CloseBracketData>("closeBrackets", pos, side)
                .into_iter()
                .map(|value| value.brackets)
                .collect();
            assert_eq!(
                &json!([pos, probe[1], active, data, brackets]),
                probe,
                "{step} at byte {pos} {side:?}"
            );
        }
        assert_eq!(row["calls"], calls.swap(0, Ordering::Relaxed), "{step}");
        let interior = state.text().as_str().find('😀').unwrap() + 1;
        assert!(!overridden.is_active_at(state, interior, Assoc::After));
        assert!(
            state
                .language_data_at::<&str>("scope", interior, Assoc::After)
                .is_empty()
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    };
    let switch = |state: &mut EditorState, ext| {
        state
            .update(Transaction::default().with_effects([parser.reconfigure(ext)]))
            .unwrap();
    };
    capture("stock", &state);
    switch(&mut state, stock_rules.extension());
    capture("stock root with rules", &state);
    switch(&mut state, overridden.extension());
    capture("override", &state);
    switch(&mut state, overridden.clone().extension());
    capture("same override", &state);
    let inside = original.find("module").unwrap() + 1;
    state
        .update(Transaction::default().with_selection(EditorSelection::single(inside, inside)))
        .unwrap();
    capture("selection changed", &state);
    state
        .update(
            Transaction::default()
                .with_effects([extras.reconfigure(replace.of(mark("replace-input")))]),
        )
        .unwrap();
    capture("replace entries changed", &state);
    let retained = state.clone();
    let changes = ChangeSet::single(Change::new(0..0, ";; 😀\n")).unwrap();
    state
        .update(Transaction::new(changes).with_selection(EditorSelection::single(0, 0)))
        .unwrap();
    capture("edit and select", &state);
    switch(&mut state, empty.extension());
    capture("empty root facet", &state);
    switch(&mut state, absent.extension());
    capture("no root facet", &state);
    switch(
        &mut state,
        Extension::group([overridden.extension(), json_extension()]),
    );
    capture("override before JSON", &state);
    switch(
        &mut state,
        Extension::group([
            json_extension().with_precedence(Precedence::High),
            overridden.extension(),
        ]),
    );
    capture("JSON before override", &state);
    switch(&mut state, stock.extension());
    capture("restored stock", &state);
    capture("retained state", &retained);

    let md = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(|name| {
                (name == "wat").then_some(NativeLanguage::Wast)
            }))),
            ..Default::default()
        },
    );
    let host = EditorState::with_extensions(
        "前 😀\n\n```wat\n(module) (func)\n```\n",
        Extension::group([md, data]),
    )
    .unwrap();
    capture("stock mount", &host);
    assert!(rows.next().is_none());
}

#[test]
fn predicates_see_only_committed_syntax_after_change_filters() {
    let selected = WastDataFacet::new(Some(mark("selected")));
    let rules = vec![WastSublanguage::new(
        WastSublanguageMode::Replace,
        selected,
        |node, state| {
            assert_eq!(state.text().as_str(), "(module) (func)");
            keyword(node)
        },
    )];
    let language = wast_language()
        .with_node_props(&[SUBLANGUAGES.add(move |row| row.is_top().then(|| rules.clone()))]);
    let protect = change_filter(|_, _| ChangePermission::Protect(std::iter::once(0..8).collect()));
    let mut state = EditorState::with_extensions(
        "(module)",
        Extension::group([language.extension(), protect]),
    )
    .unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(1..7, "rejected"),
        Change::new(8..8, " (func)"),
    ])
    .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(state.text().as_str(), "(module) (func)");
    assert_eq!(
        state.language_data_at::<&str>("scope", 11, Assoc::After),
        ["selected"]
    );
}

#[test]
fn viewport_metadata_uses_the_configured_root_before_view_bootstrap() {
    let root = WastDataFacet::new(Some(mark("viewport")));
    let rules = vec![WastSublanguage::new(
        WastSublanguageMode::Replace,
        WastDataFacet::new(Some(mark("keyword"))),
        |node, _| keyword(node),
    )];
    let language = wast_language().with_node_props(&[
        LANGUAGE_DATA.add(move |row| row.is_top().then(|| Some(root.clone()))),
        SUBLANGUAGES.add(move |row| row.is_top().then(|| rules.clone())),
    ]);
    let state =
        EditorState::with_extensions("(module)", language.viewport_extension().unwrap()).unwrap();
    let check = |state: &EditorState, scope| {
        assert!(attached_language_snapshot(state).is_none());
        assert!(!language.is_active_at(state, 2, Assoc::After));
        assert_eq!(
            language.find_regions(state),
            std::iter::once(0..8).collect::<Vec<_>>()
        );
        assert_eq!(
            state.language_data_at::<&str>("scope", 2, Assoc::After),
            [scope]
        );
    };
    check(&state, "viewport");
    assert!(state.wast_syntax_tree(0).is_none());
    let mut view = EditorView::<()>::new(state.clone());
    assert!(view.force_wast_parsing(8, 0).unwrap());
    check(view.state(), "keyword");
    check(&state, "viewport");
}
