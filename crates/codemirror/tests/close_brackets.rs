//! Behavioral fixtures checked against the SRI-pinned `@codemirror/autocomplete`
//! 6.20.4 distribution in `docs/codemirror-upstream.json`. Offsets use UTF-8.

use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangePermission, ChangeSet, EditorSelection,
    EditorState, Extension, History, LINE_SEPARATOR, SelectionRange, Transaction,
    autocomplete::brackets::{
        CloseBracketConfig, CloseBracketSyntax, close_brackets, delete_bracket_pair,
        delete_bracket_pair_with, insert_bracket, insert_bracket_with,
    },
    change_filter,
    language::{LanguageData, language_data_provider},
    language_data::CloseBracketData,
};
use serde_json::{Value, json};

fn selection(ranges: &[(usize, usize)], main: usize) -> EditorSelection {
    EditorSelection::new(
        ranges
            .iter()
            .map(|&(anchor, head)| SelectionRange::new(anchor, head))
            .collect(),
        main,
    )
    .unwrap()
}

fn select(state: &mut EditorState, ranges: &[(usize, usize)], main: usize) {
    state
        .update(Transaction::default().with_selection(selection(ranges, main)))
        .unwrap();
}

fn state(doc: &str, ranges: &[(usize, usize)], main: usize) -> EditorState {
    let mut state = EditorState::with_extensions(doc, close_brackets()).unwrap();
    select(&mut state, ranges, main);
    state
}

fn check(state: &EditorState, text: &str, ranges: &[(usize, usize)], main: usize) {
    assert_eq!(state.text().as_str(), text);
    assert_eq!(state.selection(), &selection(ranges, main));
}

fn apply_input(state: &mut EditorState, token: &str) {
    let tr = insert_bracket(state, token)
        .unwrap()
        .expect("expected custom input");
    assert_eq!(tr.user_event(), Some("input.type"));
    assert!(tr.scroll_into_view());
    state.update(tr).unwrap();
}

#[test]
fn closes_before_whitespace_and_delimiters_but_not_before_words_or_commas() {
    for text in ["", " ", "\u{feff}", ")", "]", "}", ":", ";", ">", "\r\n"] {
        let mut state = state(text, &[(0, 0)], 0);
        apply_input(&mut state, "[");
        check(&state, &format!("[]{text}"), &[(1, 1)], 0);
    }
    for text in ["x", "é", "🙂", ",", "\u{0085}"] {
        assert!(
            insert_bracket(&state(text, &[(0, 0)], 0), "[")
                .unwrap()
                .is_none()
        );
    }
    let mut state = state(",", &[(0, 0)], 0);
    let config = CloseBracketConfig {
        brackets: &["<"],
        before: ",",
        ..Default::default()
    };
    assert!(insert_bracket_with(&state, "(", config).unwrap().is_none());
    state
        .update(insert_bracket_with(&state, "<", config).unwrap().unwrap())
        .unwrap();
    check(&state, "<>,", &[(1, 1)], 0);
}

fn language_data_cases(kind: &'static str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/close_bracket_language_data.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |case| case["kind"] == kind)
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn typed_input_honors_before_and_word_characters_at_each_caret() {
    let mut mismatches = Vec::new();
    for case in language_data_cases("input") {
        let before = match case["before"].as_str() {
            None => None,
            Some(",") => Some(","),
            Some("") => Some(""),
            _ => unreachable!(),
        };
        let string_prefixes: &[_] = if case["prefixes"].is_null() {
            &[]
        } else {
            &["r"]
        };
        let words = case["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| (byte(&item[0]), Arc::<str>::from(item[1].as_str().unwrap())))
            .collect::<Vec<_>>();
        let provider = language_data_provider(move |_, head, side| {
            assert_eq!(side, Assoc::Before);
            let mut data = LanguageData::default().with(
                "closeBrackets",
                CloseBracketData {
                    before,
                    string_prefixes,
                    ..Default::default()
                },
            );
            if let Some((_, chars)) = words.iter().find(|(at, _)| *at == head) {
                data = data.with("wordChars", Arc::clone(chars));
            }
            vec![Arc::new(data)]
        });
        let mut state = EditorState::with_extensions(
            case["source"].as_str().unwrap(),
            Extension::group([
                close_brackets(),
                provider,
                ALLOW_MULTIPLE_SELECTIONS.of(true),
            ]),
        )
        .unwrap();
        let carets: Vec<_> = case["heads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pos| (byte(pos), byte(pos)))
            .collect();
        select(&mut state, &carets, byte(&case["main"]));
        let input = case["input"].as_str().unwrap();
        let result = insert_bracket(&state, input).unwrap();
        let (text, heads) = if let Some(transaction) = result {
            state.update(transaction).unwrap();
            (
                json!(state.text().as_str()),
                json!(
                    state
                        .selection()
                        .ranges()
                        .iter()
                        .map(|range| range.head())
                        .collect::<Vec<_>>()
                ),
            )
        } else {
            (Value::Null, Value::Null)
        };
        if text != case["result"] || heads != case["selection"] {
            mismatches.push(format!(
                "{}: got {text} {heads}; expected {} {}",
                case["name"], case["result"], case["selection"]
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn close_tracking_prunes_other_logical_lines_at_selection_updates() {
    for case in language_data_cases("track") {
        let base = close_brackets();
        let extension = if let Some(separator) = case["separator"].as_str() {
            Extension::group([base, LINE_SEPARATOR.of(separator.into())])
        } else {
            base
        };
        let source = case["source"].as_str().unwrap();
        let mut state = EditorState::with_extensions(source, extension).unwrap();
        apply_input(&mut state, "(");
        let tail = state.text().len();
        select(&mut state, &[(tail, tail)], 0);
        select(&mut state, &[(1, 1)], 0);
        let skip = insert_bracket(&state, ")").unwrap();
        assert_eq!(
            skip.is_some(),
            case["keep"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Some(transaction) = skip {
            state.update(transaction).unwrap();
            check(&state, &format!("(){source}"), &[(2, 2)], 0);
        }
    }
}

#[test]
fn surrounds_a_reversed_utf8_range_and_another_caret() {
    let mut state = state("é 🙂", &[(2, 0), (7, 7)], 1);
    apply_input(&mut state, "[");
    check(&state, "[é] 🙂[]", &[(3, 1), (10, 10)], 1);
    select(&mut state, &[(3, 3), (10, 10)], 0);
    apply_input(&mut state, "]");
    check(&state, "[é] 🙂[]", &[(4, 4), (11, 11)], 0);
}

#[test]
fn adjacent_selection_ranges_map_both_closing_effects() {
    let mut state = state("ab", &[(0, 1), (1, 2)], 1);
    apply_input(&mut state, "(");
    check(&state, "(a)(b)", &[(1, 2), (4, 5)], 1);
    for (at, end) in [(2, 3), (5, 6)] {
        select(&mut state, &[(at, at)], 0);
        apply_input(&mut state, ")");
        check(&state, "(a)(b)", &[(end, end)], 0);
    }
}

#[test]
fn declines_the_entire_operation_when_any_range_cannot_handle_it() {
    let mut state = state(" a", &[(0, 0), (1, 1)], 0);
    assert!(insert_bracket(&state, "(").unwrap().is_none());
    check(&state, " a", &[(0, 0), (1, 1)], 0);
    select(&mut state, &[(0, 0)], 0);
    apply_input(&mut state, "(");
    select(&mut state, &[(1, 1), (3, 3)], 0);
    assert!(insert_bracket(&state, ")").unwrap().is_none());
    assert!(delete_bracket_pair(&state).unwrap().is_none());
    select(&mut state, &[(1, 1), (2, 3)], 0);
    assert!(delete_bracket_pair(&state).unwrap().is_none());
}

#[test]
fn only_the_main_asymmetric_close_has_to_be_tracked() {
    let mut state = state(") ", &[(2, 2)], 0);
    apply_input(&mut state, "(");
    select(&mut state, &[(0, 0), (3, 3)], 0);
    assert!(insert_bracket(&state, ")").unwrap().is_none());
    select(&mut state, &[(0, 0), (3, 3)], 1);
    apply_input(&mut state, ")");
    check(&state, ") ()", &[(1, 1), (4, 4)], 1);
}

#[test]
fn markers_follow_edits_at_their_edges_and_drop_when_deleted() {
    let mut state = state("", &[(0, 0)], 0);
    apply_input(&mut state, "(");
    state
        .update(Transaction::new(
            ChangeSet::new(vec![Change::new(1..1, "é"), Change::new(2..2, ")")]).unwrap(),
        ))
        .unwrap();
    select(&mut state, &[(4, 4)], 0);
    assert!(insert_bracket(&state, ")").unwrap().is_none());
    select(&mut state, &[(3, 3)], 0);
    apply_input(&mut state, ")");
    check(&state, "(é))", &[(4, 4)], 0);

    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(3..4, "")).unwrap(),
        ))
        .unwrap();
    select(&mut state, &[(3, 3)], 0);
    assert!(insert_bracket(&state, ")").unwrap().is_none());
}

#[test]
fn selection_updates_prune_old_markers_off_the_main_line_after_mapping() {
    let mut state = state("\n", &[(0, 0), (1, 1)], 0);
    apply_input(&mut state, "(");
    check(&state, "()\n()", &[(1, 1), (4, 4)], 0);
    // New markers on other lines are registered after pruning old markers.
    select(&mut state, &[(4, 4)], 0);
    apply_input(&mut state, ")");
    select(&mut state, &[(1, 1)], 0);
    assert!(insert_bracket(&state, ")").unwrap().is_none());
}

#[test]
fn deletes_existing_pairs_at_multiple_carets_without_the_extension() {
    let mut state = EditorState::new("{}é''");
    select(&mut state, &[(1, 1), (5, 5)], 1);
    let transaction = delete_bracket_pair(&state).unwrap().unwrap();
    assert_eq!(transaction.user_event(), Some("delete.backward"));
    assert!(transaction.scroll_into_view());
    state.update(transaction).unwrap();
    check(&state, "é", &[(0, 0), (2, 2)], 1);
}

#[test]
fn custom_pairs_use_unicode_scalars_and_correct_byte_lengths() {
    for (open, close) in [
        ("［", "］"),
        ("»", "«"),
        ("\u{07ff}", "\u{0800}"),
        ("🙈", "🙉"),
    ] {
        let config = CloseBracketConfig {
            brackets: &[open],
            ..Default::default()
        };
        let mut state = state("é", &[(2, 2)], 0);
        let input = insert_bracket_with(&state, open, config).unwrap().unwrap();
        state.update(input).unwrap();
        let between = 2 + open.len();
        let at_end = between + close.len();
        check(&state, &format!("é{open}{close}"), &[(between, between)], 0);
        let skip = insert_bracket_with(&state, close, config).unwrap().unwrap();
        state.update(skip).unwrap();
        check(&state, &format!("é{open}{close}"), &[(at_end, at_end)], 0);
        select(&mut state, &[(between, between)], 0);
        state
            .update(delete_bracket_pair_with(&state, config).unwrap().unwrap())
            .unwrap();
        check(&state, "é", &[(2, 2)], 0);
    }
}

#[test]
fn quotes_use_word_context_prefixes_and_tracked_closes() {
    for (doc, pos) in [("word", 4), ("é!", 2), (" word", 1), ("\"", 0), ("_", 1)] {
        assert!(
            insert_bracket(&state(doc, &[(pos, pos)], 0), "\"")
                .unwrap()
                .is_none()
        );
    }
    let config = CloseBracketConfig {
        string_prefixes: &["r", "π"],
        word_chars: "$",
        ..Default::default()
    };
    for doc in ["br", "$r", "aπ", "$"] {
        assert!(
            insert_bracket_with(&state(doc, &[(doc.len(), doc.len())], 0), "\"", config)
                .unwrap()
                .is_none()
        );
    }
    let mut state = state(" π", &[(3, 3)], 0);
    state
        .update(insert_bracket_with(&state, "\"", config).unwrap().unwrap())
        .unwrap();
    check(&state, " π\"\"", &[(4, 4)], 0);
    apply_input(&mut state, "\"");
    check(&state, " π\"\"", &[(5, 5)], 0);
}

#[test]
fn quote_word_rules_use_the_same_utf16_units_as_the_pinned_command() {
    for (doc, byte, extra, pairs) in [
        ("𝟙 ", "𝟙".len(), "", true),
        (" 𝟙", 1, "", false),
        (" 😀", 1, "🐍", false),
        ("😀 ", "😀".len(), "🐍", true),
        (" 😀", 1, "\u{1fa00}", false),
        ("😀 ", "😀".len(), "\u{1fa00}", false),
        ("\u{1fa00}r ", "\u{1fa00}r".len(), "😀", false),
        (" 😀", 1, "🦀", true),
        ("😀 ", "😀".len(), "🦀", true),
    ] {
        let mut state = state(doc, &[(byte, byte)], 0);
        let config = CloseBracketConfig {
            string_prefixes: &["r"],
            word_chars: extra,
            ..Default::default()
        };
        let transaction = insert_bracket_with(&state, "\"", config).unwrap();
        assert_eq!(transaction.is_some(), pairs, "{doc:?} at {byte}, {extra}");
        if let Some(transaction) = transaction {
            state.update(transaction).unwrap();
            let expected = format!("{}\"\"{}", &doc[..byte], &doc[byte..]);
            check(&state, &expected, &[(byte + 1, byte + 1)], 0);
        }
    }
}

#[test]
fn unlike_asymmetric_closes_every_quote_caret_has_to_be_tracked() {
    let mut state = state("\" ", &[(2, 2)], 0);
    apply_input(&mut state, "\"");
    select(&mut state, &[(0, 0), (3, 3)], 1);
    assert!(insert_bracket(&state, "\"").unwrap().is_none());
}

struct Node {
    range: Range<usize>,
    parent: Option<usize>,
    first_child: Option<usize>,
}

struct Syntax([Node; 3]);

impl CloseBracketSyntax for Syntax {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        self.0.iter().rposition(|node| {
            let Range { start, end } = node.range;
            start < end
                && (pos > start || pos == start && side == Some(Assoc::After))
                && (pos < end || pos == end && side == Some(Assoc::Before))
        })
    }

    fn range(&self, node: usize) -> Range<usize> {
        self.0[node].range.clone()
    }
    fn parent(&self, node: usize) -> Option<usize> {
        self.0[node].parent
    }
    fn first_child(&self, node: usize) -> Option<usize> {
        self.0[node].first_child
    }
}

#[test]
fn syntax_recognizes_node_starts_and_string_prefixes() {
    let syntax = Syntax([
        Node {
            range: 0..9,
            parent: None,
            first_child: Some(1),
        },
        Node {
            range: 0..9,
            parent: Some(0),
            first_child: Some(2),
        },
        Node {
            range: 2..7,
            parent: Some(1),
            first_child: None,
        },
    ]);
    let config = CloseBracketConfig {
        string_prefixes: &["r"],
        syntax: Some(&syntax),
        ..Default::default()
    };
    let inside = state("r\"word !\"", &[(7, 7)], 0);
    assert!(
        insert_bracket_with(&inside, "\"", config)
            .unwrap()
            .is_none()
    );

    let syntax = Syntax([
        Node {
            range: 0..6,
            parent: None,
            first_child: Some(1),
        },
        Node {
            range: 0..6,
            parent: Some(0),
            first_child: Some(2),
        },
        Node {
            range: 0..3,
            parent: Some(1),
            first_child: None,
        },
    ]);
    let config = CloseBracketConfig {
        syntax: Some(&syntax),
        ..Default::default()
    };
    let mut at_start = state("\"a\"  ;", &[(0, 0)], 0);
    at_start
        .update(
            insert_bracket_with(&at_start, "\"", config)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    check(&at_start, "\"\"\"a\"  ;", &[(1, 1)], 0);
    // A completed first-child string does not make the rest of its parent a string.
    let mut state = state("\"a\"  ;", &[(4, 4)], 0);
    state
        .update(insert_bracket_with(&state, "\"", config).unwrap().unwrap())
        .unwrap();
    check(&state, "\"a\" \"\" ;", &[(5, 5)], 0);
}

#[test]
fn forms_and_skips_a_syntax_aware_triple_quote() {
    let syntax = Syntax([
        Node {
            range: 0..2,
            parent: None,
            first_child: Some(1),
        },
        Node {
            range: 0..2,
            parent: Some(0),
            first_child: None,
        },
        Node {
            range: 0..0,
            parent: None,
            first_child: None,
        },
    ]);
    let mut config = CloseBracketConfig {
        brackets: &["\"", "\"\"\""],
        ..Default::default()
    };
    let mut state = state("", &[(0, 0)], 0);
    for _ in 0..2 {
        state
            .update(insert_bracket_with(&state, "\"", config).unwrap().unwrap())
            .unwrap();
    }
    check(&state, "\"\"", &[(2, 2)], 0);
    config.syntax = Some(&syntax);
    state
        .update(insert_bracket_with(&state, "\"", config).unwrap().unwrap())
        .unwrap();
    check(&state, "\"\"\"\"\"\"", &[(3, 3)], 0);
    config.syntax = None;
    state
        .update(insert_bracket_with(&state, "\"", config).unwrap().unwrap())
        .unwrap();
    check(&state, "\"\"\"\"\"\"", &[(6, 6)], 0);
}

#[test]
fn rejected_edits_do_not_install_a_marker_on_existing_text() {
    let extension = Extension::group([
        close_brackets(),
        change_filter(|_, _| ChangePermission::Reject),
    ]);
    let mut state = EditorState::with_extensions(")", extension).unwrap();
    apply_input(&mut state, "(");
    check(&state, ")", &[(0, 0)], 0);
    assert!(insert_bracket(&state, ")").unwrap().is_none());
}

#[test]
fn pair_insert_and_skip_use_native_history() {
    let mut history = History::new(state("", &[(0, 0)], 0));
    for token in ["(", ")"] {
        let transaction = insert_bracket(history.state(), token).unwrap().unwrap();
        history.update(transaction, (), |_| (None, ())).unwrap();
    }
    check(history.state(), "()", &[(2, 2)], 0);
    assert!(history.undo().is_some());
    check(history.state(), "", &[(0, 0)], 0);
    assert!(!history.can_undo());
}
