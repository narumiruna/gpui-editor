use std::{collections::HashMap, num::NonZeroUsize, sync::Arc};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeSet, CharCategory, Compartment, EditError,
    EditorSelection, EditorState, Extension, LINE_SEPARATOR, PHRASES, PhraseMap, Precedence,
    READ_ONLY, SelectionRange, StateField, TAB_SIZE, Text, Transaction,
    commands::{self, CommandResult, CommentMode, CommentStyle, CommentTokens},
    language::{LanguageData, language_data_provider},
};
use serde_json::{Value, json};

fn oracle() -> Value {
    // Recorded from the verified npm tarball with Node in external scratch.
    serde_json::from_str(include_str!("fixtures/state_config.json")).unwrap()
}

#[test]
fn oracle_fixture_matches_the_state_pin() {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let pin = manifest["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pin| pin["name"] == "@codemirror/state")
        .unwrap();
    let recorded = oracle();
    for key in ["name", "version", "integrity"] {
        assert_eq!(recorded["pin"][key], pin[key]);
    }
}

fn line_separator(separator: &str) -> Extension {
    LINE_SEPARATOR.of(separator.to_owned())
}

fn assert_document(state: &EditorState, expected: &Value, id: &str) {
    let text = state.text();
    let range = 0..text.len();
    assert_eq!(
        json!(&*state.context().facet(&LINE_SEPARATOR).unwrap()),
        expected["configured"],
        "{id}"
    );
    assert_eq!(state.line_break(), expected["line_break"], "{id}");
    assert_eq!(json!(text.to_lines()), expected["lines"], "{id}");
    assert_eq!(
        text.slice_with_line_separator(range.clone(), "\n").unwrap(),
        expected["internal"],
        "{id}"
    );
    assert_eq!(state.slice_doc(range).unwrap(), expected["display"], "{id}");
}

#[test]
fn pinned_line_separator_splits_explicit_inputs_and_joins_output() {
    let fixture = oracle();
    for case in fixture["lines"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let insert = case["insert"].as_str().unwrap();
        let extension = Extension::group(case["inputs"].as_array().unwrap().iter().map(|input| {
            let extension = line_separator(input["value"].as_str().unwrap());
            if input["high"] == true {
                extension.with_precedence(Precedence::High)
            } else {
                extension
            }
        }));
        let policy = EditorState::with_extensions(source, extension.clone()).unwrap();
        assert_eq!(policy.text().as_str(), source, "{id}");
        let parsed = policy.to_text(source);
        let insertion = policy.to_text(insert);
        if id == "unmatched-lf-is-content" {
            // The pinned CRLF policy does not split an unmatched LF. Native
            // Text always breaks on LF, including a LF that was not parsed.
            assert_eq!(case["created"]["lines"], json!(["a\nb", "c"]));
            assert_eq!(parsed.to_lines(), ["a", "b", "c"]);
            assert_eq!(insertion.to_lines(), ["", "末"]);
            continue;
        }
        assert_eq!(json!(insertion.to_lines()), case["parsed"], "{id}");
        assert_eq!(json!(parsed.to_lines()), case["created"]["lines"], "{id}");
        let mut state = match case["created"]["configured"].as_str() {
            Some(separator) => EditorState::with_line_separator(source, separator, extension),
            None => EditorState::with_extensions(parsed.as_str(), extension),
        }
        .unwrap();
        assert_document(&state, &case["created"], id);
        let end = state.text().len();
        let changes = state.changes([(end..end, insert)]).unwrap();
        state.update(Transaction::new(changes)).unwrap();
        if id == "cr-at-edit-seam" {
            // A native CR immediately before LF becomes part of the ending.
            assert_eq!(case["edited"]["lines"], json!(["a\r", "b"]));
            assert_eq!(state.text().as_str(), "a\r\nb");
            assert_eq!(state.text().to_lines(), ["a", "b"]);
        } else {
            assert_document(&state, &case["edited"], id);
        }
    }
}

#[test]
fn pinned_line_separator_reconfiguration_keeps_text_and_raw_changes() {
    let fixture = oracle();
    let steps = &fixture["line_reconfigure"];
    let compartment = Compartment::new();
    let mut state =
        EditorState::with_extensions("a\n字", compartment.of(line_separator("|"))).unwrap();
    assert_document(&state, &steps[0], "before reconfigure");
    let end = state.text().len();
    state
        .update(
            Transaction::new(state.changes([(end..end, "|b::x")]).unwrap())
                .with_effects([compartment.reconfigure(line_separator("::"))]),
        )
        .unwrap();
    assert_document(&state, &steps[1], "reconfigure and edit");
    let end = state.text().len();
    state
        .update(Transaction::new(
            state.changes([(end..end, "::末")]).unwrap(),
        ))
        .unwrap();
    assert_document(&state, &steps[2], "parsed edit");
    let end = state.text().len();
    let changes = ChangeSet::single(Change::new(end..end, "::raw")).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_document(&state, &steps[3], "raw edit");
}

#[test]
fn explicit_line_policy_leaves_raw_text_and_byte_checks_intact() {
    let source = "😀\r\nx\ry";
    assert_eq!(Text::new(source).as_str(), source);
    assert_eq!(EditorState::new(source).text().as_str(), source);
    let state = EditorState::with_extensions(source, line_separator("|")).unwrap();
    assert_eq!(state.text().as_str(), source);
    assert_eq!(state.slice_doc(0..state.text().len()).unwrap(), "😀|x\ry");
    assert_eq!(state.slice_doc(4..5).unwrap(), "\r");
    assert_eq!(state.slice_doc(5..6).unwrap(), "\n");
    assert!(state.slice_doc(1..4).is_none());
    assert_eq!(state.changes([(1..4, "|b")]), Err(EditError::InvalidRange));
    assert_eq!(state.changes([(1..1, "")]), Err(EditError::InvalidRange));
    let field = StateField::new(
        |ctx| Ok((ctx.text().as_str().to_owned(), ctx.facet(&LINE_SEPARATOR)?)),
        |previous, _| Ok(previous.clone()),
    );
    let initial = EditorState::with_line_separator(
        "😀|末",
        "|",
        Extension::group([
            field.extension(),
            line_separator("!").with_precedence(Precedence::Highest),
        ]),
    )
    .unwrap();
    let value = initial.context().field(&field).unwrap().unwrap();
    assert_eq!(value.0, "😀\n末");
    assert_eq!(value.1.as_deref(), Some("|"));
}

#[test]
fn pinned_single_selection_uses_the_start_policy_and_preserves_the_main_range() {
    let fixture = oracle();
    let steps = &fixture["selection_steps"];
    let selection = EditorSelection::new(
        vec![SelectionRange::new(1, 1), SelectionRange::new(5, 3)],
        1,
    )
    .unwrap();
    let mut default_state = EditorState::new("abcdef");
    default_state
        .update(Transaction::default().with_selection(selection.clone()))
        .unwrap();
    assert_eq!(default_state.selection(), &selection);
    assert!(default_state.allows_multiple_selections());
    let compartment = Compartment::new();
    let only_main = ALLOW_MULTIPLE_SELECTIONS.of(false);
    let mut state =
        EditorState::with_extensions("abcdef", compartment.of(only_main.clone())).unwrap();
    let assert_step = |state: &EditorState, step: usize| {
        assert_eq!(state.allows_multiple_selections(), steps[step]["enabled"]);
        assert_eq!(state.selection().to_json(), steps[step]["selection"]);
    };
    state
        .update(Transaction::default().with_selection(selection.clone()))
        .unwrap();
    assert_step(&state, 0);
    state
        .update(
            Transaction::default()
                .with_selection(selection.clone())
                .with_effects([compartment.reconfigure(Extension::group([
                    only_main.clone().with_precedence(Precedence::High),
                    ALLOW_MULTIPLE_SELECTIONS.of(true),
                ]))]),
        )
        .unwrap();
    assert_step(&state, 1);
    state
        .update(
            Transaction::default()
                .with_selection(selection.clone())
                .without_filtering(),
        )
        .unwrap();
    assert_step(&state, 2);
    state
        .update(
            Transaction::default()
                .with_selection(selection)
                .with_effects([compartment.reconfigure(only_main)]),
        )
        .unwrap();
    assert_step(&state, 3);
    state.update(Transaction::default()).unwrap();
    assert_step(&state, 4);
}

#[test]
fn pinned_read_only_is_mutable_and_does_not_block_programmatic_edits() {
    let fixture = oracle();
    let steps = &fixture["read_only_steps"];
    let compartment = Compartment::new();
    let mut state = EditorState::with_extensions(
        "a",
        Extension::group([
            READ_ONLY.of(true),
            compartment.of(READ_ONLY.of(false).with_precedence(Precedence::High)),
        ]),
    )
    .unwrap();
    let assert_step = |state: &EditorState, step: usize| {
        assert_eq!(state.read_only(), steps[step]["read_only"]);
        assert_eq!(state.text().as_str(), steps[step]["text"]);
        assert_eq!(json!(state.selection().main().head()), steps[step]["head"]);
    };
    assert_step(&state, 0);
    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    assert_step(&state, 1);
    state
        .update(
            Transaction::new(ChangeSet::single(Change::new(0..0, "X")).unwrap())
                .with_selection(EditorSelection::single(1, 1)),
        )
        .unwrap();
    assert_step(&state, 2);
}

// The phrase and language-data results were observed in the manifest-pinned
// @codemirror/state 6.7.6 artifact running in Node. Rust positions use bytes.
#[test]
fn phrase_maps_use_precedence_and_format_the_selected_template() {
    let phrases = |entries: &[(&str, &str)]| {
        PHRASES.of(Arc::new(
            entries
                .iter()
                .copied()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect::<PhraseMap>(),
        ))
    };
    let high = phrases(&[
        (
            "greeting",
            "Hi $1, $2, $ and $$ [$0, $01, $3, $999999999999999999999999] $域",
        ),
        ("empty", ""),
    ]);
    let state = EditorState::with_extensions(
        "",
        Extension::group([
            phrases(&[("greeting", "low"), ("empty", "low")]),
            high.with_precedence(Precedence::High),
            PHRASES.of(Arc::new(HashMap::new())),
        ]),
    )
    .unwrap();
    assert_eq!(
        state.phrase("greeting", &["値$2", "語"]),
        "Hi 値$2, 語, 値$2 and $ [$0, 値$2, $3, $999999999999999999999999] 値$2域"
    );
    assert_eq!(state.phrase("empty", &[]), "");
    assert_eq!(
        state.phrase("not mapped $ and $$", &["字"]),
        "not mapped 字 and $"
    );
    assert_eq!(state.phrase("no $$", &[]), "no $$");
}

#[test]
fn language_data_providers_keep_order_and_respect_the_requested_side() {
    let data = |label, word| {
        Arc::new(
            LanguageData::default()
                .with("label", label)
                .with("wordChars", word),
        )
    };
    let left = data("left", "-");
    let right = data("right", "$");
    let a = language_data_provider(move |state, pos, side| {
        assert_eq!(state.text().as_str(), "😀x-y");
        if pos < 4 || (pos == 4 && side == Assoc::Before) {
            vec![
                left.clone(),
                Arc::new(LanguageData::default().with("label", "second-left")),
            ]
        } else {
            vec![right.clone()]
        }
    });
    let global = data("global", "!");
    let b = language_data_provider(move |_, _, _| vec![global.clone()]);
    let state = EditorState::with_extensions(
        "😀x-y",
        Extension::group([b, a.with_precedence(Precedence::High)]),
    )
    .unwrap();
    assert_eq!(
        state.language_data_at::<&str>("label", 4, Assoc::Before),
        ["left", "second-left", "global"]
    );
    assert_eq!(
        state.language_data_at::<&str>("label", 4, Assoc::After),
        ["right", "global"]
    );
    assert!(
        state
            .language_data_at::<String>("label", 4, Assoc::After)
            .is_empty()
    );
    assert!(
        state
            .language_data_at::<&str>("label", 1, Assoc::After)
            .is_empty()
    );
    let cat = state.char_categorizer(4);
    assert_eq!(
        ["-", "$", " ", "👋"].map(|text| cat.category(text)),
        [
            CharCategory::Word,
            CharCategory::Other,
            CharCategory::Space,
            CharCategory::Other
        ]
    );
    assert_eq!(state.word_range_at(4), Some(4..7));
}

#[test]
fn comment_commands_read_positional_language_data_at_line_starts() {
    let data = |line| {
        Arc::new(LanguageData::default().with(
            "commentTokens",
            CommentTokens {
                line: Some(line),
                block: None,
            },
        ))
    };
    let (js, py) = (data("//"), data("#"));
    let mut state = EditorState::with_extensions(
        "abc\r\n字",
        language_data_provider(move |state, pos, side| {
            assert_eq!(side, Assoc::After);
            let is_first = state.text().line_at(pos).unwrap().number == 1;
            vec![if is_first { js.clone() } else { py.clone() }]
        }),
    )
    .unwrap();
    let selection = EditorSelection::new(
        vec![SelectionRange::new(0, 0), SelectionRange::new(5, 5)],
        1,
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    for expected in ["// abc\r\n# 字", "abc\r\n字"] {
        let tx = commands::comment_with_language_data(
            &state,
            CommentMode::Toggle,
            CommentStyle::PreferLine,
        )
        .unwrap()
        .unwrap();
        state.update(tx).unwrap();
        assert_eq!(state.text().as_str(), expected);
    }
}

#[test]
fn facet_values_follow_first_input_and_can_reconfigure() {
    let tab = |size| TAB_SIZE.of(NonZeroUsize::new(size).unwrap());
    assert_eq!(EditorState::new("").tab_size().get(), 4);
    assert!(!EditorState::new("").read_only());
    let compartment = Compartment::new();
    let mut state = EditorState::with_extensions(
        "a",
        Extension::group([
            tab(6),
            tab(8),
            READ_ONLY.of(true),
            compartment
                .of(Extension::group([tab(3), READ_ONLY.of(false)])
                    .with_precedence(Precedence::High)),
        ]),
    )
    .unwrap();
    assert_eq!(state.tab_size().get(), 3);
    assert!(!state.read_only());
    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    assert_eq!(state.tab_size().get(), 6);
    assert!(state.read_only());
}

#[test]
fn pinned_read_only_guards_edit_commands_not_selections_or_transactions() {
    let mut state = EditorState::with_extensions("a \nb", READ_ONLY.of(true)).unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    let guarded: [fn(&EditorState) -> CommandResult; 12] = [
        commands::delete_char_backward,
        commands::delete_char_forward,
        commands::delete_group_backward,
        commands::delete_group_forward,
        commands::delete_line,
        commands::delete_trailing_whitespace,
        commands::split_line,
        commands::transpose_chars,
        commands::copy_line_down,
        commands::move_line_down,
        commands::indent_more,
        commands::indent_less,
    ];
    for command in guarded {
        assert!(command(&state).unwrap().is_none());
    }
    assert!(commands::select_all(&state).unwrap().is_some());
    // The pinned newline builders and empty-selection Tab do not check read-only.
    assert!(commands::insert_newline(&state).unwrap().is_some());
    assert!(
        commands::insert_newline_keep_indent(&state)
            .unwrap()
            .is_some()
    );
    assert!(commands::insert_tab(&state).unwrap().is_some());
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(0..0, "X")).unwrap(),
        ))
        .unwrap();
    assert_eq!(state.text().as_str(), "Xa \nb");
}

#[test]
fn indent_less_uses_tab_size_configured_for_the_state() {
    let mut state =
        EditorState::with_extensions("\tfoo", TAB_SIZE.of(NonZeroUsize::new(8).unwrap())).unwrap();
    state
        .update(commands::indent_less(&state).unwrap().unwrap())
        .unwrap();
    assert_eq!(state.text().as_str(), "      foo");
}
