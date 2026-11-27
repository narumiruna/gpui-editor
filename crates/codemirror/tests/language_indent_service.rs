use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Assoc, EditError, EditorSelection, EditorState, Extension, Precedence, TAB_SIZE, Text,
    Transaction,
    commands::{indent_less, indent_more},
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle, get_indent_unit, get_indentation,
        indent_range, indent_service, indent_string,
    },
    language_data::NativeLanguage,
};
use serde_json::{Value, json};

// Ran the pinned @codemirror/language 6.13.1 and @codemirror/state 6.7.6
// after checking their archive hashes against docs/codemirror-upstream.json.
const ORACLE: &str = r#"
{"name":"order-and-overrides","doc":"\talpha\r\n  beta\r\n    \r\n   keep\r\n  gamma","style":"\t\t","tabSize":3,"endByte":38,"unit":6,"printed":"\t\t ","columns":[2,9,8,null,9],"result":"  alpha\r\n\t\t  beta\r\n\r\n   keep\r\n\t\t\tgamma"}
{"name":"line-end-included","doc":"  a\n b\n c","style":"  ","tabSize":4,"endByte":4,"unit":2,"printed":"       ","columns":[2,4,3],"result":"  a\n    b\n c"}
"#;

#[test]
fn services_styles_and_range_updates_match_the_pinned_language() {
    for row in ORACLE.trim().lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let id = row["name"].as_str().unwrap();
        let input = row["doc"].as_str().unwrap();
        let style = IndentStyle::parse(row["style"].as_str().unwrap()).unwrap();
        let tab_size = NonZeroUsize::new(row["tabSize"].as_u64().unwrap() as usize).unwrap();
        let high = indent_service(|state, cx| {
            let line = state.text().line_at(cx.position()).unwrap();
            if line.number == 1 {
                IndentDecision::Column(2)
            } else if line.text.trim() == "keep" {
                IndentDecision::Inherit
            } else {
                IndentDecision::Defer
            }
        });
        let previous = indent_service(|state, cx| {
            let number = state.text().line_at(cx.position()).unwrap().number;
            let Some(line) = state.text().line(number - 1) else {
                return IndentDecision::Defer;
            };
            IndentDecision::Column(
                cx.line_indent(line.range.start, Assoc::After).unwrap() + cx.unit(),
            )
        });
        let mut state = EditorState::with_extensions(
            input,
            Extension::group([
                TAB_SIZE.of(tab_size),
                INDENT_UNIT.of(style),
                previous,
                high.with_precedence(Precedence::High),
            ]),
        )
        .unwrap();
        assert_eq!(json!(get_indent_unit(&state)), row["unit"], "{id}");
        assert_eq!(indent_string(&state, 7), row["printed"], "{id}");
        let columns: Vec<_> = (1..=state.text().line_count())
            .map(|n| {
                let start = state.text().line(n).unwrap().range.start;
                get_indentation(&state, start, IndentOptions::default(), None).unwrap()
            })
            .collect();
        assert_eq!(json!(columns), row["columns"], "{id}");
        let range = 0..row["endByte"].as_u64().unwrap() as usize;
        let changes = indent_range(&state, range, None).unwrap();
        state.update(Transaction::new(changes)).unwrap();
        assert_eq!(state.text().as_str(), row["result"], "{id}");
    }
}

#[test]
fn syntax_fallback_uses_the_matching_document_and_a_service_can_stop_it() {
    let input = "{\n  \"a\": [\n     1\n      ]\n}";
    let state = EditorState::new(input);
    let at = state.text().line(4).unwrap().range.start;
    let syntax = NativeLanguage::Json.parse(Text::new(input)).unwrap();
    let stale = NativeLanguage::Json.parse(Text::new("[]")).unwrap();
    let options = IndentOptions::default();
    assert_eq!(
        get_indentation(&state, at, options, Some(&syntax)),
        Ok(Some(2))
    );
    assert_eq!(get_indentation(&state, at, options, Some(&stale)), Ok(None));
    let overridden =
        EditorState::with_extensions(input, indent_service(|_, _| IndentDecision::Inherit))
            .unwrap();
    assert_eq!(
        get_indentation(&overridden, at, options, Some(&syntax)),
        Ok(None)
    );
    let emoji = EditorState::new("😀");
    assert_eq!(
        indent_range(&emoji, 1..3, None),
        Err(EditError::InvalidRange)
    );
    for invalid in ["", " \t", "a", "\u{85}"] {
        assert_eq!(IndentStyle::parse(invalid), Err(EditError::InvalidRange));
    }
}

#[test]
fn projected_indent_range_uses_editor_lines_without_joining_raw_breaks() {
    let input = "{\r\"猫\": [\r\n   \"😀\"\r  ]\r}\r\n";
    let mut state = EditorState::with_extensions(
        input,
        indent_service(|state, cx| {
            if cx.position() == 2 {
                assert_eq!(state.text().as_str().as_bytes()[1], b'\r');
                assert_eq!(cx.line_at(cx.position(), Assoc::After).unwrap().number, 2);
            }
            IndentDecision::Defer
        }),
    )
    .unwrap();
    let syntax = NativeLanguage::Json.parse(state.text().clone()).unwrap();
    assert_eq!(
        get_indentation(&state, 2, IndentOptions::default(), Some(&syntax)),
        Ok(Some(2))
    );
    let changes = indent_range(&state, 0..input.len(), Some(&syntax)).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(
        state.text().as_str(),
        "{\r  \"猫\": [\r\n    \"😀\"\r  ]\r}\r\n"
    );

    // The pinned view removes the spaces between LF lines. In raw text that
    // edit would join the separate CR and LF, so decline the entire edit.
    let input = "[\r    \n1]\r\n";
    let state = EditorState::new(input);
    let syntax = NativeLanguage::Json.parse(state.text().clone()).unwrap();
    assert!(
        indent_range(&state, 0..input.len(), Some(&syntax))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn command_indent_units_follow_state_facets_and_keep_utf8_boundaries() {
    let unit = INDENT_UNIT.of(IndentStyle::parse("\t\t").unwrap());
    let mut state = EditorState::with_extensions("x", unit).unwrap();
    state.update(indent_more(&state).unwrap().unwrap()).unwrap();
    assert_eq!(state.text().as_str(), "\t\tx");
    state.update(indent_less(&state).unwrap().unwrap()).unwrap();
    assert_eq!(state.text().as_str(), "x");
    let mut state = EditorState::with_extensions(
        "\u{2002}\u{2002}x",
        INDENT_UNIT.of(IndentStyle::parse("\u{2003}").unwrap()),
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(6, 6)))
        .unwrap();
    state.update(indent_less(&state).unwrap().unwrap()).unwrap();
    assert_eq!(state.text().as_str(), "\u{2003}x");
}
