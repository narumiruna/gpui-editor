use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Compartment, EditError, EditorSelection, EditorState, Extension, Text, Transaction,
    language::{
        brackets::{
            BracketConfig, BracketDirection::*, BracketMatch, BracketMatchingOptions,
            bracket_matching, configured_bracket_matches_in,
        },
        wast::{BRACKET_MATCHING_HANDLE, CLOSED_BY, OPENED_BY, WastBracketHandle, WastDocument},
    },
    language_data::{
        NativeSyntax, SupportedDocument, WastLanguage, attached_language_snapshot, wast_language,
    },
    setup::minimal_setup,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

fn wast(snapshot: &SupportedDocument) -> &WastDocument {
    let NativeSyntax::Wast(doc) = snapshot.syntax() else {
        panic!("expected attached WAST syntax");
    };
    doc
}

fn configured(props: &Value) -> WastLanguage {
    let [closed, opened, handles] =
        ["closedBy", "openedBy", "handle"].map(|key| props[key].clone());
    let names = |value: &Value| serde_json::from_value::<Vec<String>>(value.clone()).ok();
    wast_language().with_node_props(&[
        CLOSED_BY.add(move |row| names(&closed[row.name()])),
        OPENED_BY.add(move |row| names(&opened[row.name()])),
        BRACKET_MATCHING_HANDLE.add(move |row| {
            let value = handles.get(row.name())?;
            let callback: WastBracketHandle = match value.as_str() {
                Some("keyword") => Arc::new(|node| {
                    node.children()
                        .find(|child| child.kind().name() == "Keyword")
                }),
                Some("none") => Arc::new(|_| None),
                Some("lastChild") => Arc::new(|node| node.children().next_back()),
                None if value.is_null() => return Some(None),
                _ => panic!("unexpected fixture handle: {value}"),
            };
            Some(Some(callback))
        }),
    ])
}

fn encoded(hit: Option<BracketMatch>) -> Value {
    let span = |range: Range<usize>| [range.start, range.end];
    hit.map(|found| json!([span(found.start), found.end.map(span), found.matched]))
        .unwrap_or(Value::Null)
}

#[test]
fn configured_wast_matches_pinned_directional_and_caret_decisions() {
    for line in include_str!("fixtures/wast_bracket_props.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let language = configured(&row["props"]);
        let options = &row["options"];
        let options = BracketMatchingOptions {
            after_cursor: options["afterCursor"].as_bool(),
            brackets: options["brackets"].as_str().map(Arc::from),
            max_scan_distance: serde_json::from_value(options["maxScanDistance"].clone()).unwrap(),
            ..Default::default()
        };
        let config = options.config();
        let text = Text::new(source);
        let base = EditorState::create_from_text(
            text.clone(),
            None,
            Extension::group([language.extension(), bracket_matching(options.clone())]),
        )
        .unwrap();
        let snapshot = attached_language_snapshot(&base).unwrap();
        let doc = wast(&snapshot);
        for query in row["queries"].as_array().unwrap() {
            let head = query["head"].as_u64().unwrap() as usize;
            let label = format!("{name} at {head}");
            let mut expected_dirs = query["directional"].clone();
            let mut expected_hit = query["caret"].clone();
            let mut expected_marks = query["marks"].clone();
            if name == "empty endpoint is not paintable" {
                // The pinned matcher returns a zero-length end, and its stock
                // renderer throws. Keep the match, but omit the unpaintable end.
                assert_eq!(expected_dirs[1], json!([[9, 10], [16, 16], true]));
                assert_eq!(expected_hit, expected_dirs[1]);
                assert_eq!(expected_marks, "Mark decorations may not be empty");
                expected_dirs[1][1] = Value::Null;
                expected_hit[1] = Value::Null;
                expected_marks = json!([[9, 10, "cm-matchingBracket"]]);
            }
            assert_eq!(
                json!([Backward, Forward].map(|dir| encoded(doc.match_bracket(head, dir, config)))),
                expected_dirs,
                "{label}: directional",
            );
            assert_eq!(
                encoded(doc.bracket_near_with_config(head, config)),
                expected_hit,
                "{label}: caret",
            );
            let mut state = base.clone();
            state
                .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
                .unwrap();
            let editor = minimal_setup::<()>(state, KeyPlatform::Linux);
            let hits = editor.bracket_matches();
            assert!(hits.len() <= 1, "{label}");
            assert_eq!(
                encoded(hits.into_iter().next()),
                expected_hit,
                "{label}: setup"
            );
            let marks: Vec<_> = editor
                .decorations()
                .unwrap()
                .bracket_matches
                .iter(0)
                .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
                .collect();
            assert_eq!(json!(marks), expected_marks, "{label}: setup marks");
        }
    }
}

#[test]
fn stock_rows_and_legacy_queries_preserve_parentheses_but_do_not_match_quotes() {
    let stock = wast_language();
    for row in stock.node_types() {
        assert_eq!(
            row.prop(&CLOSED_BY).map(Vec::as_slice),
            (row.name() == "(").then_some([String::from(")")].as_slice())
        );
        assert_eq!(
            row.prop(&OPENED_BY).map(Vec::as_slice),
            (row.name() == ")").then_some([String::from("(")].as_slice())
        );
        assert!(row.prop(&BRACKET_MATCHING_HANDLE).is_none());
    }
    let source = "(module \"text\" )";
    let doc = WastDocument::parse(Text::new(source));
    let last = source.len() - 1;
    let hit = Some(BracketMatch {
        start: 0..1,
        end: Some(last..last + 1),
        matched: true,
    });
    assert_eq!(doc.bracket_near(0), hit);
    assert_eq!(
        doc.bracket_near_with_config(0, BracketConfig::default()),
        hit
    );
    for head in [8, 9, 13, 14] {
        assert_eq!(doc.bracket_near(head), None);
        assert_eq!(
            doc.bracket_near_with_config(head, BracketConfig::default()),
            None
        );
    }
}

#[test]
fn role_reconfiguration_updates_setup_and_retains_old_snapshots() {
    let source = "(module $x)";
    let stock = wast_language();
    let configured = configured(&json!({"closedBy": {"(": []}}));
    let raw = configured.parse(source).unwrap();
    let open = raw
        .root()
        .children()
        .next()
        .unwrap()
        .children()
        .next()
        .unwrap();
    assert_eq!(open.name(), "(");
    assert!(open.prop(&CLOSED_BY).unwrap().is_empty());
    let slot = Compartment::new();
    let state = EditorState::with_extensions(
        source,
        Extension::group([
            slot.of(stock.extension()),
            bracket_matching(Default::default()),
        ]),
    )
    .unwrap();
    let old = attached_language_snapshot(&state).unwrap();
    let mut editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    let stock_hit = editor.bracket_matches();
    assert_eq!(
        encoded(stock_hit.first().cloned()),
        json!([[0, 1], [10, 11], true])
    );
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([slot.reconfigure(configured.extension())]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    let new = attached_language_snapshot(editor.view().state()).unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(
        encoded(editor.bracket_matches().into_iter().next()),
        json!([[0, 1], [10, 11], false])
    );
    assert_eq!(
        wast(&old).bracket_near_with_config(0, BracketConfig::default()),
        stock_hit.first().cloned()
    );
    assert_eq!(
        wast(&new).bracket_near(0),
        stock_hit.first().cloned(),
        "legacy stock query"
    );
}

#[test]
fn invalid_positions_and_foreign_handles_do_not_make_marks() {
    let foreign = Box::leak(Box::new(WastDocument::parse(Text::new("(func 0)"))));
    let foreign = foreign
        .nodes()
        .find(|node| node.kind().name() == "Keyword")
        .unwrap();
    let handle: WastBracketHandle = Arc::new(move |_| Some(foreign));
    let language = wast_language().with_node_props(&[
        CLOSED_BY.add(|row| (row.name() == "Keyword").then(Vec::new)),
        BRACKET_MATCHING_HANDLE
            .add(move |row| (row.name() == "Keyword").then(|| Some(handle.clone()))),
    ]);
    let source = "(module \"😀\")\r\n";
    let state = EditorState::with_extensions(source, language.extension()).unwrap();
    let snapshot = attached_language_snapshot(&state).unwrap();
    let doc = wast(&snapshot);
    let config = BracketConfig::default();
    for head in [
        source.find('😀').unwrap() + 1,
        source.find('\n').unwrap(),
        source.len() + 1,
    ] {
        assert_eq!(doc.bracket_near_with_config(head, config), None);
        for dir in [Backward, Forward] {
            assert_eq!(doc.match_bracket(head, dir, config), None);
        }
    }
    // The foreign node covers this position, and its bytes are valid in both trees.
    assert_eq!(doc.match_bracket(2, Forward, config), None);
}

#[test]
fn plain_text_options_cannot_match_half_a_crlf() {
    let source = "(; [\r\n ;)";
    let doc = WastDocument::parse(Text::new(source));
    for brackets in ["[\r", "[\n"] {
        let config = BracketConfig {
            brackets,
            ..BracketConfig::default()
        };
        assert_eq!(
            doc.match_bracket(3, Forward, config),
            Some(BracketMatch {
                start: 3..4,
                end: None,
                matched: false,
            })
        );
        assert_eq!(doc.match_bracket(6, Backward, config), None);
    }
}

#[test]
fn scoped_plain_queries_do_not_use_configured_roles_or_scan_past_scope() {
    let source = "(module $x)";
    let language = configured(&json!({"closedBy": {"(": ["Identifier"]}}));
    let state = EditorState::with_extensions(
        source,
        Extension::group([language.extension(), bracket_matching(Default::default())]),
    )
    .unwrap();
    let editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    assert_eq!(
        encoded(editor.bracket_matches().into_iter().next()),
        json!([[0, 1], [8, 10], true])
    );
    let state = editor.view().state();
    let scoped = |scope| configured_bracket_matches_in(state, scope, |_, _| true);
    assert_eq!(
        scoped(0..7).unwrap(),
        [BracketMatch {
            start: 0..1,
            end: None,
            matched: false
        }]
    );
    assert!(scoped(1..7).unwrap().is_empty());
    assert_eq!(scoped(0..source.len() + 1), Err(EditError::InvalidRange));
}
