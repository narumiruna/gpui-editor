use std::{
    future::Future,
    pin::Pin,
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
        lezer::{LANGUAGE_DATA, ParseError, SyntaxNode},
        yaml::YamlNode,
    },
    language_data::{
        JsonLanguage, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
        SupportedDocument, YamlLanguage, attached_language_snapshot, json_language, yaml_extension,
        yaml_language,
    },
};
use serde_json::{Value, json};

fn fixture(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/yaml_language_handle.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn raw_rows(source: &str, root: SyntaxNode<'_>) -> Vec<Value> {
    fn visit(source: &str, node: SyntaxNode<'_>, depth: usize, rows: &mut Vec<Value>) {
        let range = node.range();
        assert!(
            source.get(range.clone()).is_some(),
            "invalid {} range {range:?}",
            node.name()
        );
        rows.push(json!([depth, node.name(), range.start, range.end]));
        for child in node.children() {
            let child_range = child.range();
            assert!(range.start <= child_range.start && child_range.end <= range.end);
            visit(source, child, depth + 1, rows);
        }
    }

    assert_eq!(root.range(), 0..source.len());
    let mut rows = Vec::new();
    visit(source, root, 0, &mut rows);
    rows
}

fn doc_rows(node: &YamlNode, depth: usize, rows: &mut Vec<Value>) {
    let range = node.range();
    rows.push(json!([depth, node.kind().name(), range.start, range.end]));
    for child in node.children() {
        doc_rows(child, depth + 1, rows);
    }
}

fn rows(snapshot: &SupportedDocument) -> Vec<Value> {
    let NativeSyntax::Yaml(doc) = snapshot.syntax() else {
        panic!("expected a YAML snapshot");
    };
    let mut rows = Vec::new();
    doc_rows(doc.root(), 0, &mut rows);
    rows
}

fn failure(error: ParseError) -> Value {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("unexpected YAML error: {error}");
    };
    json!({"offset": offset})
}

fn attached(source: &str, extension: Extension) -> Value {
    match EditorState::with_extensions(source, extension) {
        Ok(state) => {
            assert!(!close_brackets_enabled(&state).unwrap());
            json!({"tree": rows(&snapshot(&state))})
        }
        Err(ConfigurationError::StrictSyntax(error)) => failure(error),
        Err(error) => panic!("unexpected configuration error: {error:?}"),
    }
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

#[test]
fn yaml_raw_and_editor_parses_match_pinned_trees_and_errors() {
    let stock = yaml_language();
    let strict = stock.configure(Some(true), Some("strict YAML"));
    let inherited = strict.configure(None, Some(""));
    let reset = inherited.configure(Some(false), None);
    assert_eq!(strict.name(), inherited.name());
    assert_eq!(strict.name(), reset.name());
    assert!(strict.clone() == strict);
    assert!(inherited != strict);
    for row in fixture("parse") {
        let source = row["source"].as_str().unwrap();
        for (handle, field) in [
            (stock, "raw"),
            (&strict, "strictRaw"),
            (&inherited, "strictRaw"),
            (&reset, "raw"),
        ] {
            let actual = match handle.parse(source) {
                Ok(tree) => {
                    let root = tree.root();
                    assert_eq!(root.name(), "Stream");
                    assert!(
                        root.prop(&LANGUAGE_DATA).and_then(Option::as_ref)
                            == Some(stock.data_facet())
                    );
                    json!({"tree": raw_rows(source, root)})
                }
                Err(error) => failure(error),
            };
            assert_eq!(actual, row[field], "{}: {field}", row["name"]);
        }
        for (extension, field) in [
            (yaml_extension(), "editor"),
            (strict.extension(), "strictEditor"),
            (reset.extension(), "editor"),
        ] {
            assert_eq!(
                attached(source, extension),
                row[field],
                "{}: {field}",
                row["name"]
            );
        }
    }
}

#[test]
fn yaml_raw_ranges_do_not_wrap_like_the_default_packed_pin() {
    let stock = yaml_language();
    let strict = stock.configure(Some(true), None);
    let cases: Vec<_> = fixture("rawPacking").collect();
    assert_eq!(cases.len(), 4);
    for row in cases {
        let eol = row["eol"].as_str().unwrap();
        let tail_len = usize::try_from(row["tailLength"].as_u64().unwrap()).unwrap();
        let tail = format!("#{}", "x".repeat(tail_len));
        let source = ["", "# a", "# b", "k: v", &tail].join(eol);
        // These are default-pin measurements in UTF-16, not native ranges.
        let packed = &row["packed"];
        assert_eq!(packed["relativeStart"], (1 << 16) - eol.len());
        if tail_len == 4 {
            assert_eq!(packed["hiddenNodes"], 8);
            assert_eq!(packed["reversedUtf16"], json!([]));
        } else {
            assert_eq!(packed["hiddenNodes"], 0);
            let end = source.rfind(eol).unwrap();
            assert_eq!(
                packed["reversedUtf16"],
                json!([
                    [1, "Document", 1 << 16, end],
                    [2, "BlockMapping", 1 << 16, end],
                ])
            );
        }
        for handle in [stock, &strict] {
            let tree = handle.parse(&source).unwrap();
            assert_eq!(json!(raw_rows(&source, tree.root())), row["raw"]);
        }
    }
}

#[test]
fn yaml_native_limits_do_not_fall_back_in_strict_mode() {
    let stock = yaml_language();
    let strict = stock.configure(Some(true), None);
    for row in fixture("limit") {
        let source = match row["name"].as_str().unwrap() {
            "deep editor tree" => format!("{}x{}", "[".repeat(160), "]".repeat(160)),
            "large comment" => format!("#{}", "x".repeat(1 << 20)),
            _ => panic!("unknown limit probe"),
        };
        assert_eq!(row["bytes"], source.len());
        assert_eq!(row["editorRoot"], "Stream");
        let Err(ConfigurationError::StrictSyntax(ParseError::LimitExceeded { offset })) =
            EditorState::with_extensions(&source, strict.extension())
        else {
            panic!("expected a strict YAML parse limit");
        };
        if row["name"] == "large comment" {
            assert_eq!(offset, 0);
            for handle in [stock, &strict] {
                assert!(matches!(
                    handle.parse(&source),
                    Err(ParseError::LimitExceeded { offset: 0 })
                ));
            }
        } else {
            assert!(offset > 0 && offset < source.len());
            assert_eq!(strict.parse(&source).unwrap().root().name(), row["rawRoot"]);
        }
        let state = EditorState::with_extensions(&source, stock.extension()).unwrap();
        let document = snapshot(&state);
        let NativeSyntax::Yaml(doc) = document.syntax() else {
            panic!("expected a YAML snapshot");
        };
        assert_eq!(doc.root().range(), 0..source.len());
        assert!(doc.is_recovered());
    }
}

fn reject(state: &mut EditorState, transaction: Transaction, expected: &Value) {
    let before = snapshot(state);
    let text = state.text().clone();
    let selection = state.selection().clone();
    let active = YamlLanguage::active_name(state);
    let EditError::Configuration(ConfigurationError::StrictSyntax(error)) =
        state.update(transaction).unwrap_err()
    else {
        panic!("expected a strict YAML rejection");
    };
    assert_eq!(&failure(error), expected);
    assert_eq!(state.text(), &text);
    assert_eq!(state.selection(), &selection);
    assert_eq!(YamlLanguage::active_name(state), active);
    assert!(Arc::ptr_eq(&snapshot(state), &before));
}

#[test]
fn yaml_strict_checks_proposed_bytes_and_accepts_repair_with_reconfiguration() {
    let row = fixture("edit").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = usize::try_from(row["removeAt"].as_u64().unwrap()).unwrap();
    let strict = yaml_language().configure(Some(true), Some("strict YAML"));
    let recovery = strict.configure(Some(false), Some("recovery YAML"));
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let original = snapshot(&state);
    let retained = state.clone();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &original));

    let cut = || Transaction::new(ChangeSet::single(Change::new(at..at + 1, "")).unwrap());
    let install_strict = || slot.reconfigure(strict.extension());
    reject(
        &mut state,
        cut().with_selection(EditorSelection::single(0, 0)),
        &row["rejected"],
    );
    state
        .update(cut().with_effects([slot.reconfigure(recovery.extension())]))
        .unwrap();
    assert_eq!(state.text().as_str(), row["invalid"].as_str().unwrap());
    assert_eq!(json!(rows(&snapshot(&state))), row["recovered"]);
    let recovery_state = state.clone();
    let recovered = snapshot(&state);
    reject(
        &mut state,
        Transaction::default().with_effects([install_strict()]),
        &row["rejected"],
    );
    let insert = |text| Transaction::new(ChangeSet::single(Change::new(at..at, text)).unwrap());
    let proposed = row["proposed"].as_str().unwrap();
    assert_eq!(row["rejectedProposed"]["offset"], proposed.len());
    assert_ne!(proposed.len(), state.text().len());
    reject(
        &mut state,
        insert(row["insert"].as_str().unwrap()).with_effects([install_strict()]),
        &row["rejectedProposed"],
    );
    state
        .update(insert("]").with_effects([install_strict()]))
        .unwrap();
    assert_eq!(state.text().as_str(), source);
    assert_eq!(
        YamlLanguage::active_name(&state).as_deref(),
        Some(strict.name())
    );
    assert_eq!(json!(rows(&snapshot(&state))), row["repaired"]);
    assert!(!Arc::ptr_eq(&snapshot(&state), &original));
    assert!(Arc::ptr_eq(&snapshot(&retained), &original));
    assert_eq!(retained.text().as_str(), source);
    assert!(Arc::ptr_eq(&snapshot(&recovery_state), &recovered));
    assert_eq!(
        YamlLanguage::active_name(&recovery_state).as_deref(),
        Some(recovery.name())
    );
    assert_eq!(
        recovery_state.text().as_str(),
        row["invalid"].as_str().unwrap()
    );
}

#[test]
fn yaml_catalog_and_direct_install_share_stock_identity_and_data_precedence() {
    let row = fixture("data").next().unwrap();
    let entry = LanguageDescription::match_name(LANGUAGES, "yaml", false).unwrap();
    let mut load = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(support)) =
        Pin::new(&mut load).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native YAML catalog support");
    };
    let catalog = support.extension().unwrap();
    let stock = yaml_language();
    assert_eq!(stock.name(), row["stock"].as_str().unwrap());
    assert!(std::ptr::eq(stock, support.yaml_language().unwrap()));
    assert!(NativeLanguage::Json.support().yaml_language().is_none());
    let named = stock.configure(None, row["named"].as_str());
    let distinct = named.configure(None, Some(""));
    assert!(distinct != named);
    assert_eq!(distinct.name(), named.name());
    assert!(stock.data_facet() == named.data_facet());
    assert!(stock.data_facet() != json_language().data_facet());

    let slot = Compartment::new();
    let source = "{}\r\n# 😀";
    let data = |label| LanguageData::default().with("probe", label);
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([
            named.data(data("normal")),
            stock.data(data("high")).with_precedence(Precedence::High),
            json_language()
                .data(data("json only"))
                .with_precedence(Precedence::Highest),
            slot.of(yaml_extension()),
        ]),
    )
    .unwrap();
    let stock_snapshot = snapshot(&state);
    let swap = |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    state.update(swap(catalog.clone())).unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &stock_snapshot));
    state
        .update(swap(Extension::group([
            yaml_extension(),
            catalog,
            stock.clone().extension(),
        ])))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &stock_snapshot));
    let probe = |state: &EditorState, at, side| state.language_data_at::<&str>("probe", at, side);
    for at in [0, source.len()] {
        for side in [Assoc::Before, Assoc::After] {
            assert!(stock.is_active_at(&state, at, side));
            assert_eq!(json!(probe(&state, at, side)), row["probe"]);
            let tokens = state.language_data_at::<CommentTokens>("commentTokens", at, side);
            assert!(tokens.iter().all(|token| token.block.is_none()));
            assert_eq!(
                json!(tokens.iter().map(|token| token.line).collect::<Vec<_>>()),
                row["comments"]
            );
            let rules = state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
            assert_eq!(rules.len(), 1);
            for case in row["indent"].as_array().unwrap() {
                assert_eq!(
                    rules[0](case[0].as_str().unwrap()),
                    case[1].as_bool().unwrap()
                );
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
    let regions = |state: &EditorState| named.find_regions(state);
    assert!(
        regions(&state)
            .into_iter()
            .eq(std::iter::once(0..source.len()))
    );
    state.update(swap(named.extension())).unwrap();
    let renamed = snapshot(&state);
    assert!(!Arc::ptr_eq(&stock_snapshot, &renamed));
    assert_eq!(rows(&stock_snapshot), rows(&renamed));
    state.update(swap(distinct.extension())).unwrap();
    let distinct_snapshot = snapshot(&state);
    assert!(!Arc::ptr_eq(&renamed, &distinct_snapshot));
    state
        .update(swap(Extension::group([
            distinct.extension(),
            named.extension().with_precedence(Precedence::High),
        ])))
        .unwrap();
    assert_eq!(
        YamlLanguage::active_name(&state).as_deref(),
        row["primary"].as_str()
    );
    assert!(!Arc::ptr_eq(&distinct_snapshot, &snapshot(&state)));
    assert_eq!(json!(probe(&state, 0, Assoc::After)), row["twoProviders"]);

    state
        .update(swap(Extension::group([
            named.configure(Some(true), None).extension(),
            json_language()
                .extension()
                .with_precedence(Precedence::High),
        ])))
        .unwrap();
    assert_eq!(
        JsonLanguage::active_name(&state).as_deref(),
        row["other"].as_str()
    );
    assert!(YamlLanguage::active_name(&state).is_none());
    assert!(regions(&state).is_empty());
    assert!(!stock.is_active_at(&state, 0, Assoc::After));
    let probes = probe(&state, 0, Assoc::After);
    assert!(!probes.is_empty() && probes.iter().all(|probe| *probe == "json only"));
    state.update(swap(Extension::default())).unwrap();
    assert!(attached_language_snapshot(&state).is_none());
    assert!(probe(&state, 0, Assoc::After).is_empty());
}
