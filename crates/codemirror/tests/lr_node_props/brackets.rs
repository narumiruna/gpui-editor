use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    EditorSelection,
    language::brackets::{
        BracketConfig, BracketDirection, BracketMatch, BracketMatchingOptions, bracket_matching,
    },
    setup::minimal_setup,
    view::keymap::KeyPlatform,
};

use super::*;

fn configured(stock: &MarkdownCodeParser, props: &Value) -> MarkdownCodeParser {
    let closed = props["closedBy"].clone();
    let opened = props["openedBy"].clone();
    let names = |value: &Value| serde_json::from_value::<Vec<String>>(value.clone()).ok();
    with_props(
        stock,
        &[
            CLOSED_BY.add(move |row| names(&closed[row.name()])),
            OPENED_BY.add(move |row| names(&opened[row.name()])),
        ],
    )
}

fn directional(
    doc: &SupportedDocument,
    head: usize,
    dir: BracketDirection,
    config: BracketConfig<'_>,
) -> Option<BracketMatch> {
    match doc.syntax() {
        NativeSyntax::Json(doc) => doc.match_bracket_at(head, dir, config),
        NativeSyntax::LezerGrammar(doc) => doc.match_bracket_at(head, dir, config),
        _ => unreachable!(),
    }
}

fn caret(doc: &SupportedDocument, head: usize, config: BracketConfig<'_>) -> Option<BracketMatch> {
    match doc.syntax() {
        NativeSyntax::Json(doc) => doc.bracket_near_with_config(head, config),
        NativeSyntax::LezerGrammar(doc) => doc.bracket_near_with_config(head, config),
        _ => unreachable!(),
    }
}

fn encoded(hit: Option<BracketMatch>) -> Value {
    let span = |range: Range<usize>| [range.start, range.end];
    hit.map(|found| json!([span(found.start), found.end.map(span), found.matched]))
        .unwrap_or(Value::Null)
}

#[test]
fn configured_lr_matches_pinned_roles_and_setup_marks() {
    for line in include_str!("../fixtures/lr_bracket_roles.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let [stock, _] = handles(&row);
        let parser = configured(&stock, &row["props"]);
        let options = &row["options"];
        let options = BracketMatchingOptions {
            after_cursor: options["afterCursor"].as_bool(),
            brackets: options["brackets"].as_str().map(Arc::from),
            max_scan_distance: serde_json::from_value(options["maxScanDistance"].clone()).unwrap(),
            ..Default::default()
        };
        let config = options.config();
        let source = row["source"].as_str().unwrap();
        let base = EditorState::with_extensions(
            source,
            Extension::group([
                with_lr!(&parser, handle => handle.extension()),
                bracket_matching(options.clone()),
            ]),
        )
        .unwrap();
        let doc = attached_language_snapshot(&base).unwrap();
        for query in row["queries"].as_array().unwrap() {
            use BracketDirection::{Backward, Forward};
            let head = query["head"].as_u64().unwrap() as usize;
            let label = format!("{} {} at {head}", row["name"], row["label"]);
            assert_eq!(
                json!([Backward, Forward].map(|dir| encoded(directional(&doc, head, dir, config)))),
                query["directional"],
                "{label}: directional",
            );
            assert_eq!(
                encoded(caret(&doc, head, config)),
                query["caret"],
                "{label}: caret"
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
                query["caret"],
                "{label}: setup"
            );
            let marks: Vec<_> = editor
                .decorations()
                .unwrap()
                .bracket_matches
                .iter(0)
                .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
                .collect();
            assert_eq!(json!(marks), query["marks"], "{label}: setup marks");
        }
        for (head, _) in source.bytes().enumerate().filter(|&(head, byte)| {
            !source.is_char_boundary(head)
                || (byte == b'\n' && head > 0 && source.as_bytes()[head - 1] == b'\r')
        }) {
            assert_eq!(caret(&doc, head, config), None);
            for dir in [BracketDirection::Backward, BracketDirection::Forward] {
                assert_eq!(directional(&doc, head, dir, config), None);
            }
        }
    }
}

#[test]
fn typed_roles_refresh_setup_but_not_retained_snapshots() {
    for (name, source) in [("json", "{\"😀\":0}"), ("lezer", "@top Main { \"😀\" }")] {
        let head = source.find('{').unwrap();
        let end = source.rfind('}').unwrap();
        let [stock, _] = handles(&json!({"name": name}));
        let marked = configured(&stock, &json!({"closedBy": {"{": []}}));
        let slot = Compartment::new();
        let mut state = EditorState::with_extensions(
            source,
            Extension::group([
                slot.of(with_lr!(&stock, handle => handle.extension())),
                bracket_matching(Default::default()),
            ]),
        )
        .unwrap();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
            .unwrap();
        let old = attached_language_snapshot(&state).unwrap();
        let mut editor = minimal_setup::<()>(state, KeyPlatform::Linux);
        let matched = BracketMatch {
            start: head..head + 1,
            end: Some(end..end + 1),
            matched: true,
        };
        assert_eq!(editor.bracket_matches(), std::slice::from_ref(&matched));
        editor
            .view_mut()
            .update(
                Transaction::default().with_effects([
                    slot.reconfigure(with_lr!(&marked, handle => handle.extension()))
                ]),
                (),
                |_| (None, ()),
            )
            .unwrap();
        let current = attached_language_snapshot(editor.view().state()).unwrap();
        assert!(!Arc::ptr_eq(&old, &current));
        assert_eq!(
            editor.bracket_matches(),
            [BracketMatch {
                matched: false,
                ..matched.clone()
            }]
        );
        assert_eq!(caret(&old, head, BracketConfig::default()), Some(matched));
        assert_eq!(editor.view().revision(), 0);
        assert_eq!(editor.view().undo_depth(), 0);
    }
}
