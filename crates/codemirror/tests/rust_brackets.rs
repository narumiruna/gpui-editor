use std::{ops::Range, ptr, sync::Arc};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Text, Transaction,
    language::{
        brackets::{
            BracketConfig,
            BracketDirection::{Backward, Forward},
            BracketMatch, BracketMatchingOptions, bracket_matching,
        },
        lezer::{BRACKET_MATCHING_HANDLE, CLOSED_BY, LrBracketHandle, OPENED_BY},
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_with_parser,
        },
        rust::{RustDocument, RustNodeKind},
    },
    language_data::{
        NativeSyntax, RustLanguage, SupportedDocument, attached_language_snapshot, rust_language,
    },
    setup::minimal_setup,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

#[test]
fn rust_brackets_match_the_pinned_editor() {
    for line in include_str!("fixtures/rust_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let doc = RustDocument::parse(Text::new(source));
        for check in row["checks"].as_array().unwrap() {
            let pos = check[0].as_u64().unwrap() as usize;
            let actual = doc.bracket_near(pos).map_or(Value::Null, |hit| {
                json!([
                    [hit.start.start, hit.start.end],
                    hit.end.map(|span| [span.start, span.end]),
                    hit.matched
                ])
            });
            assert_eq!(actual, check[1], "{source:?} at byte {pos}");
        }
        if let Some(pos) = source.find('😀') {
            assert!(doc.bracket_near(pos + 1).is_none());
        }
        assert!(doc.bracket_near(source.len() + 1).is_none());
    }
}

fn configured(props: &Value) -> RustLanguage {
    let closed = props["closedBy"].clone();
    let opened = props["openedBy"].clone();
    let handles = props["handles"].clone();
    let list = |value: &Value| serde_json::from_value::<Vec<String>>(value.clone()).ok();
    rust_language().with_node_props(&[
        CLOSED_BY.add(move |row| list(&closed[row.term().to_string()])),
        OPENED_BY.add(move |row| list(&opened[row.term().to_string()])),
        BRACKET_MATCHING_HANDLE.add(move |row| {
            let select: LrBracketHandle = match handles[row.term().to_string()].as_str()? {
                "first" => Arc::new(|node| node.children().next()),
                "second" => Arc::new(|node| node.children().nth(1)),
                "skip" => Arc::new(|_| None),
                _ => unreachable!(),
            };
            Some(Some(select))
        }),
    ])
}

fn options(row: &Value) -> BracketMatchingOptions {
    BracketMatchingOptions {
        after_cursor: row["afterCursor"].as_bool(),
        brackets: row["brackets"].as_str().map(Arc::from),
        max_scan_distance: serde_json::from_value(row["maxScanDistance"].clone()).unwrap(),
        ..Default::default()
    }
}

fn fixture() -> impl Iterator<Item = Value> {
    include_str!("fixtures/rust_configured_brackets.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn rust(snapshot: &SupportedDocument) -> &RustDocument {
    let NativeSyntax::Rust(doc) = snapshot.syntax() else {
        panic!("expected Rust syntax");
    };
    doc
}

fn encoded(hit: Option<BracketMatch>) -> Value {
    let span = |range: Range<usize>| [range.start, range.end];
    hit.map(|found| json!([span(found.start), found.end.map(span), found.matched]))
        .unwrap_or(Value::Null)
}

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap() as usize
}

fn selected(mut state: EditorState, query: &Value) -> EditorState {
    let head = offset(&query["head"]);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
        .unwrap();
    state
}

fn check(state: EditorState, config: BracketConfig<'_>, expected: &Value, label: &str) {
    let head = state.selection().main().head();
    assert_eq!(head, offset(&expected["head"]), "{label}");
    let snapshot = attached_language_snapshot(&state).unwrap();
    let doc = rust(&snapshot);
    assert_eq!(
        json!([Backward, Forward].map(|dir| encoded(doc.match_bracket_at(head, dir, config)))),
        expected["directional"],
        "{label}: directional at {head}",
    );
    assert_eq!(
        encoded(doc.bracket_near_with_config(head, config)),
        expected["caret"],
        "{label}: caret at {head}"
    );
    check_setup(state, expected, label);
}

fn check_setup(state: EditorState, expected: &Value, label: &str) {
    let head = state.selection().main().head();
    let editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    let hits = editor.bracket_matches();
    assert!(hits.len() <= 1, "{label}: setup at {head}");
    assert_eq!(
        encoded(hits.into_iter().next()),
        expected["caret"],
        "{label}: setup at {head}"
    );
    let marks: Vec<_> = editor
        .decorations()
        .unwrap()
        .bracket_matches
        .iter(0)
        .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
        .collect();
    assert_eq!(
        json!(marks),
        expected["marks"],
        "{label}: setup marks at {head}"
    );
}

#[test]
fn configured_rust_brackets_use_pinned_roles_handles_and_setup_marks() {
    let stock = rust_language();
    for row in fixture() {
        match row["case"].as_str().unwrap() {
            "pins" => {
                let roles: Vec<_> = stock
                    .node_types()
                    .filter_map(|ty| {
                        assert!(ty.prop(&BRACKET_MATCHING_HANDLE).is_none());
                        let (opened, closed) = (ty.prop(&OPENED_BY), ty.prop(&CLOSED_BY));
                        (opened.is_some() || closed.is_some())
                            .then(|| json!([ty.term(), ty.name(), opened, closed]))
                    })
                    .collect();
                assert_eq!(json!(roles), row["rows"]);
            }
            "query" => {
                let language = configured(&row["props"]);
                let options = options(&row["options"]);
                let config = options.config();
                let source = row["source"].as_str().unwrap();
                let label = row["label"].as_str().unwrap();
                let state = EditorState::with_extensions(
                    source,
                    Extension::group([language.extension(), bracket_matching(options.clone())]),
                )
                .unwrap();
                let snapshot = attached_language_snapshot(&state).unwrap();
                let doc = rust(&snapshot);
                assert!(ptr::eq(
                    doc.root().node_type().unwrap(),
                    language.node_types().nth(8).unwrap()
                ));
                for expected in row["queries"].as_array().unwrap() {
                    check(selected(state.clone(), expected), config, expected, label);
                }
                for head in (1..source.len())
                    .filter(|&head| {
                        !source.is_char_boundary(head)
                            || source.as_bytes()[head - 1..=head] == *b"\r\n"
                    })
                    .chain([source.len() + 1])
                {
                    assert!(
                        doc.bracket_near_with_config(head, config).is_none(),
                        "{label}: caret at {head}"
                    );
                    for dir in [Backward, Forward] {
                        assert!(
                            doc.match_bracket_at(head, dir, config).is_none(),
                            "{label}: {dir:?} at {head}"
                        );
                    }
                }
            }
            "retained" | "markdown" => (),
            other => panic!("unknown fixture case {other}"),
        }
    }
}

#[test]
fn configured_rust_bracket_handles_use_mapped_markdown_ranges() {
    for row in fixture().filter(|row| row["case"] == "markdown") {
        let source = row["source"].as_str().unwrap();
        let label = row["label"].as_str().unwrap();
        let install = |lang: &RustLanguage| {
            let parser = MarkdownCodeParser::Rust(lang.clone());
            markdown_with_parser(
                MarkdownConfig::default(),
                MarkdownParserConfig {
                    code_languages: Some(MarkdownCodeLanguages::Configured {
                        fallback: (label == "quoted fallback").then(|| parser.clone()),
                        resolve: Arc::new(move |name| (name == "rr").then(|| parser.clone())),
                    }),
                    ..Default::default()
                },
            )
        };
        let slot = Compartment::new();
        let state = EditorState::with_extensions(
            source,
            Extension::group([
                slot.of(install(&configured(&row["props"]))),
                bracket_matching(options(&row["options"])),
            ]),
        )
        .unwrap();
        for query in row["queries"].as_array().unwrap() {
            check_setup(selected(state.clone(), query), query, label);
        }
        let at = source.find("12").unwrap() + 1;
        let previous = row["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|query| offset(&query["head"]) == at)
            .unwrap();
        let active = selected(state, previous);
        let mut restored = active.clone();
        restored
            .update(
                Transaction::default().with_effects([slot.reconfigure(install(rust_language()))]),
            )
            .unwrap();
        assert!(
            minimal_setup::<()>(restored, KeyPlatform::Linux)
                .bracket_matches()
                .is_empty()
        );
        check_setup(active, previous, label);
    }
}

#[test]
fn rust_role_reconfiguration_updates_headless_matches_but_not_retained_states() {
    let row = fixture().find(|row| row["case"] == "retained").unwrap();
    let language = configured(&row["props"]);
    let options = options(&row["options"]);
    let config = options.config();
    let slot = Compartment::new();
    let state = EditorState::with_extensions(
        row["source"].as_str().unwrap(),
        Extension::group([
            slot.of(language.extension()),
            bracket_matching(options.clone()),
        ]),
    )
    .unwrap();
    let initial = attached_language_snapshot(&state).unwrap();
    let selected = selected(state, &row["selected"]);
    assert!(Arc::ptr_eq(
        &initial,
        &attached_language_snapshot(&selected).unwrap()
    ));
    check(selected.clone(), config, &row["selected"], "selection only");

    let mut edited = selected.clone();
    let edit = &row["edit"];
    let range = offset(&edit["from"])..offset(&edit["to"]);
    let insert = edit["insert"].as_str().unwrap();
    edited
        .update(Transaction::new(
            ChangeSet::single(Change::new(range, insert)).unwrap(),
        ))
        .unwrap();
    let edited_snapshot = attached_language_snapshot(&edited).unwrap();
    assert!(!Arc::ptr_eq(&initial, &edited_snapshot));
    check(edited.clone(), config, &row["edited"], "edited");

    let stock = rust_language();
    let mut removed = edited.clone();
    removed
        .update(Transaction::default().with_effects([slot.reconfigure(stock.extension())]))
        .unwrap();
    let removed_snapshot = attached_language_snapshot(&removed).unwrap();
    assert!(!Arc::ptr_eq(&edited_snapshot, &removed_snapshot));
    for (snapshot, expected) in [
        (&initial, &language),
        (&edited_snapshot, &language),
        (&removed_snapshot, stock),
    ] {
        let actual = rust(snapshot)
            .nodes()
            .find(|node| node.kind() == RustNodeKind::OpenAngle)
            .unwrap()
            .node_type()
            .unwrap();
        assert!(ptr::eq(actual, expected.node_types().nth(2).unwrap()));
    }
    assert!(
        stock
            .node_types()
            .nth(2)
            .unwrap()
            .prop(&CLOSED_BY)
            .is_none()
    );
    check(
        removed,
        config,
        &row["removed"],
        "removed via stock reconfigure",
    );
    check(
        selected,
        config,
        &row["retainedSelected"],
        "retained selection",
    );
    check(edited, config, &row["retainedEdited"], "retained edit");
}

#[test]
fn local_rust_fallback_keeps_stock_delimiters_without_a_row() {
    // The long malformed suffix exhausts LR recovery, as in the fold control.
    let source = format!("fn f() {{ \r\n  1\r\n}}\r\n{}", "r#\"x\";".repeat(2048));
    let select: LrBracketHandle = Arc::new(|_| panic!("selector on rowless delimiter"));
    let language = rust_language().with_node_props(&[
        CLOSED_BY.add(|row| (row.term() == 42).then(Vec::new)),
        BRACKET_MATCHING_HANDLE.add(move |row| (row.term() == 42).then(|| Some(select.clone()))),
    ]);
    let state = EditorState::with_extensions(source.as_str(), language.extension()).unwrap();
    let snapshot = attached_language_snapshot(&state).unwrap();
    let doc = rust(&snapshot);
    assert!(doc.is_recovered());
    let open = doc
        .nodes()
        .find(|node| node.kind() == RustNodeKind::OpenBrace)
        .unwrap();
    assert!(open.node_type().is_none());
    let expected = Some(BracketMatch {
        start: 7..8,
        end: Some(16..17),
        matched: true,
    });
    let config = BracketConfig {
        brackets: "<>",
        max_scan_distance: 1,
        ..Default::default()
    };
    assert_eq!(doc.match_bracket_at(7, Forward, config), expected);
    assert_eq!(doc.bracket_near_with_config(7, config), expected);
    assert_eq!(doc.bracket_near(7), expected);
}
