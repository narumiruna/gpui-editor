//! The published @codemirror/lang-json 6.0.2 linter was run with
//! @codemirror/state 6.7.6 in Node 26.10.0 (V8 14.6.202.34-node.34) and
//! headless Chrome 158.0.8127.37. These fixtures use the matching outputs
//! for raw input (CodeMirror `Text.of(source.split('\n'))`), with conversion
//! of diagnostic ranges from UTF-16 to UTF-8. Native exceptions are separate.

use gpui_editor_codemirror::{
    Text,
    language::json::{json_editor_linter, json_parse_linter},
    lint::Severity,
};
use serde_json::{Value, json};

fn check(source: &str, byte: usize, message: &str) {
    let diagnostics = json_parse_linter(&Text::new(source));
    let [error] = diagnostics.as_slice() else {
        panic!("{source:?}: {diagnostics:?}");
    };
    assert_eq!(error.severity, Severity::Error);
    assert_eq!(error.range, byte..byte, "{source:?}");
    assert!(source.is_char_boundary(byte));
    assert_eq!(error.message, message, "{source:?}");
    assert!(error.source.is_none());
    assert!(error.mark_class.is_none());
    assert!(error.actions.is_empty());
}

fn located(source: &str, byte: usize, position: usize, line: usize, column: usize, message: &str) {
    check(
        source,
        byte,
        &format!("{message} at position {position} (line {line} column {column})"),
    );
}

#[test]
fn accepts_strict_json_without_building_a_recovery_tree() {
    for source in [
        "0",
        "true",
        "false",
        "null",
        "1e9999",
        r#""\ud800""#,
        r#""\uDC00""#,
        "\"\u{2028}\u{2029}\"",
        "\r\n\t{\"名\":[\"😀\",-0.5E+2,true,false,null]}\r\n",
    ] {
        assert!(
            json_parse_linter(&Text::new(source)).is_empty(),
            "{source:?}"
        );
    }
    let depth = 12_000;
    let source = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    assert!(json_parse_linter(&Text::new(source)).is_empty());
}

#[test]
fn reports_the_first_grammar_error_in_its_actual_context() {
    const PROPERTY_OR_CLOSE: &str = "Expected property name or '}' in JSON";
    const QUOTED_PROPERTY: &str = "Expected double-quoted property name in JSON";
    const COLON: &str = "Expected ':' after property name in JSON";
    const OBJECT_SEPARATOR: &str = "Expected ',' or '}' after property value in JSON";
    const ARRAY_SEPARATOR: &str = "Expected ',' or ']' after array element in JSON";
    const TRAILING: &str = "Unexpected non-whitespace character after JSON";

    for (source, position, message) in [
        ("{", 1, PROPERTY_OR_CLOSE),
        ("{key:1}", 1, PROPERTY_OR_CLOSE),
        (r#"{"key":1,"#, 9, QUOTED_PROPERTY),
        (r#"{"a":1,}"#, 7, QUOTED_PROPERTY),
        (r#"{"key""#, 6, COLON),
        (r#"{"a" 1}"#, 5, COLON),
        (r#"{"key":1"#, 8, OBJECT_SEPARATOR),
        (r#"[{"a":1]"#, 7, OBJECT_SEPARATOR),
        (r#"{"a":1 "b":2}"#, 7, OBJECT_SEPARATOR),
        ("[1", 2, ARRAY_SEPARATOR),
        ("[true", 5, ARRAY_SEPARATOR),
        ("[1 true 3]", 3, ARRAY_SEPARATOR),
        ("[1 }", 3, ARRAY_SEPARATOR),
        (r#"{"a":[1,2}"#, 9, ARRAY_SEPARATOR),
        (r#"[0 "\q"]"#, 3, ARRAY_SEPARATOR),
        ("[0 1e+]", 3, ARRAY_SEPARATOR),
        ("nullx", 4, TRAILING),
        ("[] []", 3, TRAILING),
    ] {
        located(source, position, position, 1, position + 1, message);
    }
}

#[test]
fn reports_string_number_and_literal_errors() {
    const UNEXPECTED_NUMBER: &str = "Unexpected number in JSON";
    const BAD_ESCAPE: &str = "Bad escaped character in JSON";
    const BAD_UNICODE: &str = "Bad Unicode escape in JSON";
    for (source, byte, position, message) in [
        ("01", 1, 1, UNEXPECTED_NUMBER),
        ("-00", 2, 2, UNEXPECTED_NUMBER),
        ("[\"😀\", 0123]", 10, 8, UNEXPECTED_NUMBER),
        ("1.", 2, 2, "Unterminated fractional number in JSON"),
        ("42e+", 4, 4, "Exponent part is missing a number in JSON"),
        ("1e🦀", 2, 2, "Exponent part is missing a number in JSON"),
        ("-", 1, 1, "No number after minus sign in JSON"),
        ("\"hi", 3, 3, "Unterminated string in JSON"),
        ("\"x\\é\"", 3, 3, BAD_ESCAPE),
        ("\"😀\\q\"", 6, 4, BAD_ESCAPE),
        (r#"{"x\q":1}"#, 4, 4, BAD_ESCAPE),
        (r#""\u12💥""#, 5, 5, BAD_UNICODE),
        (r#""\u12"#, 5, 5, BAD_UNICODE),
        (
            "\"line\r\ntext\"",
            5,
            5,
            "Bad control character in string literal in JSON",
        ),
        ("tr3e", 2, 2, UNEXPECTED_NUMBER),
        ("tr-e", 2, 2, UNEXPECTED_NUMBER),
        ("tr\"e", 2, 2, "Unexpected string in JSON"),
    ] {
        located(source, byte, position, 1, position + 1, message);
    }
}

#[test]
fn uses_zero_for_errors_without_an_engine_position() {
    for source in ["", " \t\r\n", "[", "tr", "[false,", r#"{"key":"#, "\"x\\"] {
        check(source, 0, "Unexpected end of JSON input");
    }
    for (source, token) in [
        ("[1,]", ']'),
        ("[1,,3]", ','),
        (r#"{"a":}"#, '}'),
        (" /* hi */ {}", '/'),
        ("[true, #, 2], #", '#'),
        ("\"x\\Ġ\"", 'Ġ'),
        ("tr e", ' '),
        ("tr\re", '\r'),
        ("[fals]", ']'),
        ("\u{feff}null", '\u{feff}'),
    ] {
        check(
            source,
            0,
            &format!("Unexpected token '{token}', \"{source}\" is not valid JSON"),
        );
    }
    for source in ["undefined", "NaN", "Infinity", "[object Object]"] {
        check(source, 0, &format!("\"{source}\" is not valid JSON"));
    }
    for (source, excerpt) in [
        ("xxxxxxxxxxxxxxxxxxxxx", "\"xxxxxxxxxx\"..."),
        ("          x          ", "...\"          x         \"..."),
        ("           x         ", "...\"          x         \""),
    ] {
        check(
            source,
            0,
            &format!("Unexpected token 'x', {excerpt} is not valid JSON"),
        );
    }
}

#[test]
fn keeps_raw_line_endings_and_reports_utf8_ranges_with_utf16_messages() {
    const COLON: &str = "Expected ':' after property name in JSON";
    const ARRAY_SEPARATOR: &str = "Expected ',' or ']' after array element in JSON";
    located("\r\n{\"名😀\" 1}", 13, 9, 2, 8, COLON);
    located("\r{\"名😀\" 1}", 12, 8, 2, 8, COLON);
    located("\r\n[\"😀\"\r\n 2]", 12, 10, 3, 2, ARRAY_SEPARATOR);
    located("[\"a\"\u{2028},0]", 4, 4, 1, 5, ARRAY_SEPARATOR);
}

#[test]
fn matches_default_editor_lint_in_original_bytes() {
    for row in include_str!("fixtures/json_editor_lint.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let text = Text::new(case["source"].as_str().unwrap());
        let diagnostics: Vec<_> = json_editor_linter(&text)
            .iter()
            .map(|actual| {
                assert_eq!(actual.severity, Severity::Error, "{name}");
                assert!(text.slice(actual.range.clone()).is_some(), "{name}");
                json!([
                    actual.range.start,
                    actual.range.end,
                    "error",
                    actual.message
                ])
            })
            .collect();
        assert_eq!(json!(diagnostics), case["diagnostics"], "{name}");
    }
}

#[test]
fn editor_lint_does_not_treat_an_excerpt_as_position_metadata() {
    let diagnostic = &json_editor_linter(&Text::new("\r\n?at position 5"))[0];
    assert_eq!(diagnostic.range, 0..0);
    assert_eq!(
        diagnostic.message,
        "Unexpected token '?', \"\n?at position 5\" is not valid JSON"
    );
}

#[test]
fn normalizes_unpaired_surrogates_and_does_not_read_locations_from_excerpts() {
    check("💥", 0, "Unexpected token '�', \"💥\" is not valid JSON");
    check(
        "xaaaaaaaa😀bbbbbbbbbbb",
        0,
        "Unexpected token 'x', \"xaaaaaaaa�\"... is not valid JSON",
    );
    check(
        "   ?at position 5",
        0,
        "Unexpected token '?', \"   ?at position 5\" is not valid JSON",
    );
}
