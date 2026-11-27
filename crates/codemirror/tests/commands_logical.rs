//! Fixture from `@codemirror/commands` 6.11.2 in an unwrapped EditorView in
//! headless Google Chrome 158.0.8127.37. The commands, state, view, language,
//! lang-json and Lezer tarballs match the SHA-512 pins in the upstream manifest.
//! Bracket rows use plain text. Syntax rows use `@codemirror/lang-json` 6.0.2
//! and `@lezer/json` 1.0.3. The second fixture contains that grammar's tables.
//! The oracle measures UTF-8 byte lengths of slices, including explicit CRLF.

use std::sync::LazyLock;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, READ_ONLY, SelectionRange, Transaction,
    commands::{
        CommandResult, cursor_matching_bracket, delete_to_line_end, delete_to_line_start,
        select_matching_bracket, select_parent_syntax,
    },
    language::lezer::{GrammarSpec, Parser, SerializedTables},
};
use serde_json::{Value, json};

static JSON_PARSER: LazyLock<Parser> = LazyLock::new(|| {
    let grammar: Value =
        serde_json::from_str(include_str!("fixtures/commands_logical_json_grammar.json")).unwrap();
    let text = |key: &str| grammar[key].as_str().unwrap();
    let term = |value: &Value| u16::try_from(number(value)).unwrap();
    let tokenizers: Vec<_> = grammar["tokenizers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| number(value).try_into().unwrap())
        .collect();
    Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            version: number(&grammar["version"]).try_into().unwrap(),
            states: text("states"),
            state_data: text("stateData"),
            goto: text("goto"),
            token_data: text("tokenData"),
        },
        node_names: text("nodeNames"),
        max_term: term(&grammar["maxTerm"]),
        repeat_node_count: term(&grammar["repeatNodeCount"]),
        tokenizers: &tokenizers,
        top_rule: [term(&grammar["topRule"][0]), term(&grammar["topRule"][1])],
        token_prec: number(&grammar["tokenPrec"]),
    })
    .unwrap()
});

fn cases() -> impl Iterator<Item = Value> {
    include_str!("fixtures/commands_logical.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn ranges(selection: &EditorSelection) -> Value {
    json!(
        selection
            .ranges()
            .iter()
            .map(|range| [range.anchor(), range.head()])
            .collect::<Vec<_>>()
    )
}

fn check(case: &Value, command: impl FnOnce(&EditorState) -> CommandResult) {
    let name = case["name"].as_str().unwrap();
    let before = case["before"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| SelectionRange::new(number(&pair[0]), number(&pair[1])))
        .collect();
    let selection = EditorSelection::new(before, number(&case["beforeMain"])).unwrap();
    let source = case["source"].as_str().unwrap();
    let mut state =
        EditorState::with_extensions(source, READ_ONLY.of(case["readOnly"] == true)).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    assert_eq!(ranges(state.selection()), case["before"], "{name}: input");
    assert_eq!(
        state.selection().main_index(),
        number(&case["beforeMain"]),
        "{name}: input main"
    );

    let result = command(&state).unwrap();
    assert_eq!(result.is_some(), case["handled"] == true, "{name}: handled");
    let dispatch = case["dispatch"].as_array().unwrap();
    if let Some(transaction) = result {
        let [expected] = dispatch.as_slice() else {
            panic!("{name}: expected one dispatch");
        };
        assert_eq!(
            json!(transaction.user_event()),
            expected["event"],
            "{name}: event"
        );
        assert_eq!(
            transaction.scroll_into_view(),
            expected["scroll"] == true,
            "{name}: scroll"
        );
        assert_eq!(
            transaction.selection().is_some(),
            expected["selection"] == true,
            "{name}: selection update"
        );
        let changes: Vec<_> = transaction
            .changes()
            .changes()
            .map(|(range, insert)| json!([range.start, range.end, insert]))
            .collect();
        assert_eq!(json!(changes), expected["changes"], "{name}: changes");
        let announcements: Vec<_> = transaction
            .effects()
            .iter()
            .filter_map(|effect| effect.get(&gpui_editor_codemirror::view::ANNOUNCE))
            .collect();
        assert_eq!(
            json!(announcements),
            expected["announcements"],
            "{name}: announcements"
        );
        state.update(transaction).unwrap();
    } else {
        assert!(dispatch.is_empty(), "{name}: unexpected dispatch");
    }
    assert_eq!(state.text().as_str(), case["output"], "{name}: text");
    assert_eq!(
        ranges(state.selection()),
        case["after"],
        "{name}: selection"
    );
    assert_eq!(
        state.selection().main_index(),
        number(&case["afterMain"]),
        "{name}: main"
    );
}

#[test]
fn plain_matching_bracket_commands_match_pinned_oracle() {
    for case in cases() {
        match case["command"].as_str().unwrap() {
            "cursorMatchingBracket" => check(&case, cursor_matching_bracket),
            "selectMatchingBracket" => check(&case, select_matching_bracket),
            _ => {}
        }
    }
}

#[test]
fn parent_syntax_selection_matches_pinned_oracle() {
    for case in cases().filter(|case| case["command"] == "selectParentSyntax") {
        assert_eq!(case["syntax"], true);
        check(&case, |state| {
            let source = state.text().as_str();
            let tree = JSON_PARSER.parse_recovering(source).unwrap();
            select_parent_syntax(state, &tree)
        });
    }
}

#[test]
fn hard_line_deletion_matches_pinned_oracle() {
    for case in cases() {
        match case["command"].as_str().unwrap() {
            "deleteToLineStart" => check(&case, delete_to_line_start),
            "deleteToLineEnd" => check(&case, delete_to_line_end),
            _ => {}
        }
    }
}
