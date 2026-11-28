//! Oracle: @codemirror/commands 6.11.2, @codemirror/state 6.7.6,
//! @codemirror/view 6.44.0, @codemirror/language 6.13.1, and
//! @codemirror/lang-javascript 6.2.5, verified against docs/codemirror-upstream.json.
//! The fixture records actual dispatches from a headless Chrome EditorView.
//! It uses a 135 px container, 14 px monospace font, multiple selections, and
//! wrapping where requested. CRLF uses the lineSeparator facet. Positions are
//! UTF-8 byte lengths of sliceDoc(0, pos) in the respective documents.

use std::{
    borrow::Cow,
    num::NonZeroUsize,
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Assoc, EditorSelection, EditorState, READ_ONLY, SelectionRange, Transaction,
    commands::{
        IndentUnit, Indentation, indent_selection, insert_blank_line, insert_newline_and_indent,
    },
    language::{
        IndentOptions, StreamLanguage, StreamSettings,
        javascript::{JavascriptDocument, JavascriptOptions},
    },
    stream_parser::{StreamParser, StringStream},
};
use serde_json::{Value, json};

struct Plain;

impl StreamParser for Plain {
    type State = ();

    fn start_state(&self, _: usize) -> Self::State {}

    fn token(&self, stream: &mut StringStream<'_>, _: &mut Self::State) -> Option<Cow<'_, str>> {
        stream.skip_to_end();
        None
    }
}

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn ranges(selection: &EditorSelection) -> Vec<[usize; 2]> {
    selection
        .ranges()
        .iter()
        .map(|r| [r.anchor(), r.head()])
        .collect()
}

fn state_for(case: &Value) -> EditorState {
    let source = case["source"].as_str().unwrap();
    let mut state =
        EditorState::with_extensions(source, READ_ONLY.of(case["opts"]["readOnly"] == true))
            .unwrap();
    let ranges = case["before"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| SelectionRange::new(number(&r[0]), number(&r[1])))
        .collect();
    let selection = EditorSelection::new(ranges, number(&case["beforeMain"])).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state
}

fn bracket_gap(doc: &JavascriptDocument, at: usize) -> Option<Range<usize>> {
    doc.nodes().find_map(|node| {
        let mut children = node.children();
        let open = children.next()?;
        let close = children.next_back()?;
        let gap = open.range().end..close.range().start;
        (gap.start <= at
            && at <= gap.end
            && matches!(
                (open.kind().name(), close.kind().name()),
                ("(", ")") | ("[", "]") | ("{", "}")
            ))
        .then_some(gap)
    })
}

fn check_oracle(case: Value) {
    let mut state = state_for(&case);
    let name = case["name"].as_str().unwrap();
    let mode = case["mode"].as_str().unwrap().to_owned();
    let tab_size = NonZeroUsize::new(4).unwrap();
    let tabs = case["opts"]["tabs"] == true;
    let indent = if tabs {
        Indentation {
            unit: IndentUnit::Tab,
            tab_size,
        }
    } else {
        Indentation::default()
    };
    let expected_settings = StreamSettings {
        tab_size,
        indent_unit: if tabs { 4 } else { 2 },
    };
    let javascript = mode.starts_with("js").then(|| {
        JavascriptDocument::parse_with_options(
            state.text().clone(),
            JavascriptOptions {
                jsx: mode == "jsx",
                ..JavascriptOptions::default()
            },
        )
    });
    let queries = Arc::new(Mutex::new(Vec::new()));
    let trace = Arc::clone(&queries);
    let is_trace = mode == "trace";
    let language = StreamLanguage::new(Plain).with_indent(move |_, _, _, cx| {
        let at = cx.position();
        let line = cx.text().line_at(at).unwrap();
        let (query, result) = match mode.as_str() {
            "trace" => {
                let pre = cx.line_at(at, Assoc::Before).unwrap().text;
                let post = cx.text_after_pos(at, Assoc::After).unwrap();
                let doubled = pre.is_empty() && post.is_empty() && !line.text.is_empty();
                (
                    json!([at, cx.simulated_break(), doubled, pre, post]),
                    Some(if doubled { 4 } else { 2 }),
                )
            }
            "cascade" => {
                let previous = cx.text().line(line.number - 1).map_or(1, |line| {
                    cx.line_indent(line.range.start, Assoc::After).unwrap()
                });
                (json!([at, previous]), Some(previous + 2))
            }
            "two" => (json!([at]), Some(2)),
            _ => (json!([at]), None),
        };
        trace.lock().unwrap().push(query);
        result
    });
    let stream = language
        .parse(state.text().clone(), expected_settings)
        .unwrap();
    let expected_queries = case["queries"].as_array().unwrap();
    let mut trace_index = 0;
    let indentation_at = |at, settings, options: IndentOptions<'_>| {
        assert_eq!(settings, expected_settings, "{name}: settings");
        if is_trace {
            let expected = &expected_queries[trace_index];
            assert_eq!(json!(options.simulate_break), expected[1], "{name}: break");
            assert_eq!(
                json!(options.simulate_double_break),
                expected[2],
                "{name}: double break"
            );
            trace_index += 1;
        }
        if let Some(ref js) = javascript {
            js.indentation(at, settings, options)
        } else {
            stream.indentation(at, options).unwrap()
        }
    };
    let result = match case["command"].as_str().unwrap() {
        "newline" => insert_newline_and_indent(&state, indent, indentation_at, |at| {
            javascript.as_ref().and_then(|js| bracket_gap(js, at))
        }),
        "blank" => insert_blank_line(&state, indent, indentation_at, "\n"),
        "indent" => indent_selection(&state, indent, indentation_at),
        command => panic!("{command}"),
    }
    .unwrap();
    assert_eq!(
        state.text().as_str(),
        case["source"],
        "{name}: pure command"
    );
    assert_eq!(
        json!(ranges(state.selection())),
        case["before"],
        "{name}: original selection"
    );
    assert_eq!(
        &*queries.lock().unwrap(),
        expected_queries,
        "{name}: queries"
    );
    assert_eq!(result.is_some(), case["handled"] == true, "{name}: handled");
    if let Some(transaction) = result {
        let events = case["events"].as_array().unwrap();
        if events.is_empty() {
            assert!(transaction.changes().is_empty(), "{name}: no edits");
            assert!(
                transaction.selection().is_none(),
                "{name}: no selection update"
            );
            assert!(transaction.user_event().is_none(), "{name}: no user event");
        } else {
            let changes: Vec<_> = transaction
                .changes()
                .changes()
                .map(|(range, insert)| json!([range.start, range.end, insert]))
                .collect();
            assert_eq!(
                json!([
                    transaction.user_event(),
                    transaction.scroll_into_view(),
                    changes
                ]),
                events[0],
                "{name}: dispatch"
            );
        }
        state.update(transaction).unwrap();
    }
    assert_eq!(state.text().as_str(), case["text"], "{name}: result text");
    assert_eq!(
        json!(ranges(state.selection())),
        case["after"],
        "{name}: result selection"
    );
    assert_eq!(
        state.selection().main_index(),
        number(&case["afterMain"]),
        "{name}: main range"
    );
}

#[test]
fn pinned_commands_oracle() {
    for line in include_str!("fixtures/commands_syntax_indent.jsonl").lines() {
        check_oracle(serde_json::from_str(line).unwrap());
    }
}

#[test]
fn blank_line_keeps_existing_multiline_selection_content() {
    for line in include_str!("fixtures/commands_blank_line.jsonl").lines() {
        check_oracle(serde_json::from_str(line).unwrap());
    }
    // The pin can use the first selected line's whitespace to clear existing
    // text through the last line. Do not delete text for this insert command.
    let mut state = EditorState::new(" \nx");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(0, 2)))
        .unwrap();
    let transaction = insert_blank_line(&state, Indentation::default(), |_, _, _| None, "\n")
        .unwrap()
        .unwrap();
    assert_eq!(
        transaction.changes().changes().collect::<Vec<_>>(),
        [(&(3..3), "\n")]
    );
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), " \nx\n");
}

#[test]
fn newline_uses_local_endings_and_does_not_split_crlf() {
    let mut state = EditorState::new("a\r\nb\nc\r\nd");
    let selection = EditorSelection::new(
        [(2, 2), (4, 4), (6, 6), (9, 9)]
            .into_iter()
            .map(|(anchor, head)| SelectionRange::new(anchor, head))
            .collect(),
        2,
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    let mut queries = Vec::new();
    let transaction = insert_newline_and_indent(
        &state,
        Indentation::default(),
        |at, _, _| {
            queries.push(at);
            Some(0)
        },
        |_| None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(queries, [1, 4, 6, 9]);
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), "a\r\nb\n\nc\r\n\r\nd\r\n");
    assert_eq!(
        ranges(state.selection()),
        [[3, 3], [5, 5], [9, 9], [14, 14]]
    );
    assert_eq!(state.selection().main_index(), 2);
}
