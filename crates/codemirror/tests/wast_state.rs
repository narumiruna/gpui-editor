use std::{
    collections::HashMap,
    num::NonZeroUsize,
    ops::Range,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    LINE_SEPARATOR, Precedence, TAB_SIZE, Text, Transaction, change_filter,
    commands::CommentTokens,
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle,
        fold::{
            FOLD_GUTTER_CLASS, FoldGutterConfig, fold_gutter, fold_service, foldable,
            foldable_in_state, folded_ranges,
        },
        get_indentation, indent_range, indent_service,
    },
    language_data::{CloseBracketData, NativeLanguage, NativeSyntax, wast_extension},
    setup::{SetupEditor, SetupKeyContext, basic_setup},
    view::{
        EditorView,
        gutters::{GutterSlot, gutter_slots},
        keymap::{KeyEvent, KeyModifiers, KeyPlatform},
    },
};
use serde_json::{Value, json};

fn oracle() -> HashMap<String, Value> {
    include_str!("fixtures/wast_state.jsonl")
        .lines()
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["name"].as_str().unwrap().into(), row)
        })
        .collect()
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn fold_pair(range: Option<Range<usize>>) -> Value {
    json!(range.map(|range| [range.start, range.end]))
}

fn indentation(state: &EditorState, at: usize, syntax: Option<&NativeSyntax>) -> Option<usize> {
    get_indentation(state, at, IndentOptions::default(), syntax).unwrap()
}

fn assert_snapshot(state: &EditorState, row: &Value) {
    let name = row["name"].as_str().unwrap();
    assert_eq!(state.text().as_str(), row["source"], "{name}");
    assert_eq!(
        state.selection().main().head(),
        number(&row["head"]),
        "{name}"
    );
    for line in row["lines"].as_array().unwrap() {
        let (from, to) = (number(&line[0]), number(&line[1]));
        assert_eq!(
            json!(indentation(state, from, None)),
            line[2],
            "{name} at {from}"
        );
        assert_eq!(
            fold_pair(foldable_in_state(state, from, to)),
            line[3],
            "{name} at {from}"
        );
    }
    let comments = state.language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After);
    let [comments] = comments.as_slice() else {
        panic!("expected one WAST metadata provider")
    };
    let (open, close) = comments.block.unwrap();
    assert_eq!(
        json!([{ "line": comments.line, "block": { "open": open, "close": close } }]),
        row["commentTokens"]
    );
    let brackets = state.language_data_at::<CloseBracketData>("closeBrackets", 0, Assoc::After);
    let [brackets] = brackets.as_slice() else {
        panic!("expected one bracket provider")
    };
    assert_eq!(
        json!([{ "brackets": brackets.brackets }]),
        row["closeBrackets"]
    );
}

#[test]
fn attached_queries_follow_accepted_edits_and_history_at_original_byte_offsets() {
    let oracle = oracle();
    let source = oracle["initial"]["source"].as_str().unwrap();
    let protected = 9..16;
    let filter = {
        let protected = protected.clone();
        change_filter(move |_, _| {
            ChangePermission::Protect(std::iter::once(protected.clone()).collect())
        })
    };
    let old =
        EditorState::with_extensions(source, Extension::group([wast_extension(), filter])).unwrap();
    let mut view = EditorView::new(old.clone());
    assert_snapshot(view.state(), &oracle["initial"]);
    let old_syntax = NativeLanguage::Wast.parse(old.text().clone()).unwrap();
    let edit = &oracle["change"];
    let changes = ChangeSet::new(vec![
        Change::new(protected, "ignored"),
        Change::new(
            number(&edit["from"])..number(&edit["to"]),
            edit["insert"].as_str().unwrap(),
        ),
    ])
    .unwrap();
    let head = number(&oracle["edited"]["head"]);
    view.update(
        Transaction::new(changes).with_selection(EditorSelection::single(head, head)),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert_snapshot(view.state(), &oracle["edited"]);
    assert_eq!(indentation(view.state(), 25, Some(&old_syntax)), Some(4));
    view.update(
        Transaction::default()
            .with_selection(EditorSelection::single(0, 0))
            .with_user_event("select"),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert_eq!(view.undo_selection(()), Some(()));
    assert_snapshot(view.state(), &oracle["undoSelection"]);
    assert_eq!(view.redo_selection(()), Some(()));
    assert_snapshot(view.state(), &oracle["redoSelection"]);
    assert_eq!(view.undo(), Some(()));
    assert_snapshot(view.state(), &oracle["undo"]);
    assert_eq!(view.redo(), Some(()));
    assert_snapshot(view.state(), &oracle["redo"]);
    assert_snapshot(&old, &oracle["retained"]);
    let split_emoji = source.find('😀').unwrap() + 1;
    assert_eq!(
        get_indentation(&old, split_emoji, IndentOptions::default(), None),
        Err(EditError::InvalidRange)
    );
    assert_eq!(foldable_in_state(&old, split_emoji, 16), None);
}

#[test]
fn indent_options_and_services_see_the_tracked_line_bounds() {
    let oracle = oracle();
    let row = &oracle["ordinary"];
    let source = row["source"].as_str().unwrap();
    let (pos, root) = (number(&row["pos"]), number(&row["root"]));
    let override_at = |at| (at == root).then_some(9);
    let config = || {
        [
            wast_extension(),
            TAB_SIZE.of(NonZeroUsize::new(4).unwrap()),
            INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
        ]
    };
    for (name, single, double, overrides) in [
        ("ordinary", false, false, false),
        ("single", true, false, false),
        ("double", true, true, false),
        ("overrideSingle", true, false, true),
        ("overrideDouble", true, true, true),
    ] {
        let row = oracle[name].clone();
        let expected = row["column"].clone();
        let options = IndentOptions {
            simulate_break: single.then_some(pos),
            simulate_double_break: double,
            override_indentation: overrides.then_some(&override_at),
        };
        let probe = indent_service(move |_, cx| {
            assert_eq!(cx.unit(), number(&row["unit"]));
            assert_eq!(json!(cx.line_indent(root, Assoc::After)), row["lineIndent"]);
            for (bias, key) in [(Assoc::Before, "before"), (Assoc::After, "after")] {
                let line = cx.line_at(pos, bias).unwrap();
                assert_eq!(json!([line.range.start, line.text]), row[key]);
            }
            IndentDecision::Defer
        });
        let state = EditorState::with_extensions(
            source,
            Extension::group(config().into_iter().chain([probe])),
        )
        .unwrap();
        assert_eq!(
            json!(get_indentation(&state, pos, options, None).unwrap()),
            expected,
            "{name}"
        );
    }
    let options = IndentOptions {
        simulate_break: Some(pos),
        simulate_double_break: true,
        override_indentation: Some(&override_at),
    };
    for (name, decision) in [
        ("Defer", IndentDecision::Defer),
        ("Column", IndentDecision::Column(31)),
        ("Inherit", IndentDecision::Inherit),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let high = Arc::clone(&calls);
        let answer = Arc::clone(&calls);
        let services = [
            indent_service(move |_, _| {
                answer.lock().unwrap().push("answer");
                decision
            }),
            indent_service(move |_, _| {
                high.lock().unwrap().push("high");
                IndentDecision::Defer
            })
            .with_precedence(Precedence::High),
        ];
        let state = EditorState::with_extensions(
            source,
            Extension::group(config().into_iter().chain(services)),
        )
        .unwrap();
        assert_eq!(
            json!(get_indentation(&state, pos, options, None).unwrap()),
            oracle[name]["column"]
        );
        assert_eq!(json!(*calls.lock().unwrap()), oracle[name]["calls"]);
    }
    let row = &oracle["indentRange"];
    let mut state =
        EditorState::with_extensions(row["source"].as_str().unwrap(), wast_extension()).unwrap();
    let changes = indent_range(&state, 0..state.text().len(), None).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(state.text().as_str(), row["result"]);
    let state = EditorState::with_extensions("(module\r    \nnop\r)", wast_extension()).unwrap();
    assert!(
        indent_range(&state, 0..state.text().len(), None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fold_services_run_before_syntax_and_first_invalid_answer_stops_the_query() {
    let oracle = oracle();
    for (name, result, expected) in [
        ("foldDefer", None, oracle["foldDefer"]["fold"].clone()),
        (
            "foldCustom",
            Some(2..5),
            oracle["foldCustom"]["fold"].clone(),
        ),
        ("invalid", Some(1..99), Value::Null),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let high = Arc::clone(&calls);
        let answer = Arc::clone(&calls);
        let state = EditorState::with_extensions(
            "(module\n(func)\n)",
            Extension::group([
                wast_extension(),
                fold_service(move |_, _, _| {
                    answer.lock().unwrap().push("answer");
                    result.clone()
                }),
                fold_service(move |_, _, _| {
                    high.lock().unwrap().push("high");
                    None
                })
                .with_precedence(Precedence::High),
            ]),
        )
        .unwrap();
        assert_eq!(foldable_in_state(&state, 0, 99), None);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            fold_pair(foldable_in_state(&state, 0, 7)),
            expected,
            "{name}"
        );
        assert_eq!(json!(*calls.lock().unwrap()), oracle["foldDefer"]["calls"]);
    }
}

fn open_fold_lines(slots: &[GutterSlot]) -> Vec<usize> {
    slots
        .iter()
        .find(|slot| slot.class == FOLD_GUTTER_CLASS)
        .unwrap()
        .rows
        .iter()
        .filter(|row| {
            row.markers
                .iter()
                .any(|mark| mark.content.as_deref() == Some("⌄"))
        })
        .map(|row| row.line_number)
        .collect()
}

#[test]
fn setup_and_fold_gutter_use_attachment_unless_explicit_syntax_is_current() {
    let source = ";; 😀\r\n(module\r(func)\r\n)";
    let state = EditorState::with_extensions(source, wast_extension()).unwrap();
    let current = NativeLanguage::Json.parse(state.text().clone()).unwrap();
    let stale = NativeLanguage::Json.parse(Text::new("{}")).unwrap();
    assert_eq!(indentation(&state, 17, None), Some(2));
    assert_eq!(indentation(&state, 17, Some(&current)), Some(0));
    assert_eq!(foldable(&state, 9, 16, |_, _, _| None), None);
    let mut editor: SetupEditor<()> = basic_setup(state, KeyPlatform::Linux).unwrap();
    assert!(open_fold_lines(&editor.gutters(1..=4, Some(&current)).unwrap()).is_empty());
    assert_eq!(
        open_fold_lines(&editor.gutters(1..=4, Some(&stale)).unwrap()),
        [2]
    );
    assert_eq!(open_fold_lines(&editor.gutters(1..=4, None).unwrap()), [2]);
    assert!(
        !editor
            .toggle_line_fold(2, Some(&current), Duration::ZERO, ())
            .unwrap()
    );
    assert!(
        editor
            .toggle_line_fold(2, None, Duration::ZERO, ())
            .unwrap()
    );
    let ranges = |editor: &SetupEditor<()>| {
        folded_ranges(editor.view().state())
            .iter(0)
            .map(|fold| (fold.from, fold.to))
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(&editor), [(10, 25)]);
    assert!(
        editor
            .toggle_line_fold(2, Some(&current), Duration::ZERO, ())
            .unwrap()
    );
    let modifiers = KeyModifiers {
        control: true,
        alt: true,
        ..Default::default()
    };
    let fold_all = |editor: &mut SetupEditor<()>, syntax| {
        editor
            .key_event_with(
                KeyEvent::new("[", modifiers),
                Duration::ZERO,
                (),
                SetupKeyContext {
                    syntax,
                    ..Default::default()
                },
            )
            .unwrap()
            .key
            .handled
    };
    assert!(!fold_all(&mut editor, Some(&current)));
    assert!(fold_all(&mut editor, None));
    assert_eq!(ranges(&editor), [(10, 25)]);

    let state = EditorState::with_extensions(source, wast_extension()).unwrap();
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    editor
        .view_mut()
        .update(
            Transaction::default().with_selection(EditorSelection::single(19, 19)),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert!(
        editor
            .key_event(KeyEvent::new("\\", modifiers), Duration::ZERO, ())
            .unwrap()
            .0
            .handled
    );
    assert_eq!(
        editor.view().state().text().as_str(),
        ";; 😀\r\n(module\r  (func)\r\n)"
    );

    let state = EditorState::create(
        source,
        None,
        Extension::group([wast_extension(), fold_gutter(FoldGutterConfig::default())]),
    )
    .unwrap();
    assert_eq!(open_fold_lines(&gutter_slots(&state, 1..=4).unwrap()), [2]);
}

#[test]
fn setup_enter_uses_the_attached_wast_delimiter_gap() {
    for line in include_str!("fixtures/wast_newline.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let head = row["head"].as_u64().unwrap() as usize;
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(EditorSelection::single(head, head)),
            Extension::group([wast_extension(), LINE_SEPARATOR.of("\r\n".into())]),
        )
        .unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        assert!(
            editor
                .key_event(
                    KeyEvent::new("Enter", KeyModifiers::default()),
                    Duration::ZERO,
                    ()
                )
                .unwrap()
                .0
                .handled
        );
        let state = editor.view().state();
        assert_eq!(
            state.text().as_str(),
            row["after"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        assert_eq!(
            json!(state.selection().main().head()),
            row["position"],
            "{}",
            row["name"]
        );
        assert_eq!(editor.view_mut().undo(), Some(()));
        assert_eq!(editor.view().state().text().as_str(), source);
    }
}

#[test]
fn plain_and_metadata_only_states_keep_their_defaults_and_custom_breaks_are_not_syntax() {
    let source = "(module\n(func)\n)";
    let plain = EditorState::new(source);
    assert!(
        plain
            .language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After)
            .is_empty()
    );
    let metadata_only = EditorState::with_extensions(
        source,
        NativeLanguage::Wast
            .support()
            .tracked_language_data_extension(),
    )
    .unwrap();
    assert_eq!(
        metadata_only.language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After)[0].line,
        Some(";;")
    );
    for state in [plain, metadata_only] {
        assert_eq!(indentation(&state, 0, None), Some(0));
        assert_eq!(indentation(&state, 8, None), None);
        assert_eq!(foldable_in_state(&state, 0, 7), None);
    }
    let extension = || Extension::group([wast_extension(), LINE_SEPARATOR.of("|".into())]);
    let normalized = EditorState::create("(module|(func)|)", None, extension()).unwrap();
    assert_eq!(normalized.text().as_str(), source);
    assert_eq!(indentation(&normalized, 8, None), Some(2));
    assert_eq!(foldable_in_state(&normalized, 0, 7), Some(1..15));
    let raw = EditorState::with_extensions("(module|(func)|)", extension()).unwrap();
    assert_eq!(raw.text().line_count(), 1);
    assert_eq!(foldable_in_state(&raw, 0, raw.text().len()), None);
    assert_eq!(raw.line_break(), "|");
}
