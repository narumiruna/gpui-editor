use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Assoc, EditError, EditorSelection, EditorState, Extension, LINE_SEPARATOR, Precedence,
    READ_ONLY, SelectionRange, Text, Transaction,
    view::{
        ClipboardLinewiseMemory, clipboard_input_filter, clipboard_output_filter,
        plan_clipboard_copy, plan_clipboard_paste, plan_linewise_copy,
        plan_linewise_copy_projected, plan_linewise_paste, plan_linewise_paste_projected,
    },
};
use serde_json::{Value, json};

type Log = Arc<Mutex<Vec<Value>>>;

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn byte_at(source: &str, position: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in source.char_indices() {
        if units == position {
            return byte;
        }
        units += ch.len_utf16();
    }
    assert_eq!(
        position, units,
        "oracle position must be a character boundary"
    );
    source.len()
}

fn filter(name: String, kind: &'static str, log: Log) -> Extension {
    let transform = move |text: String, state: &EditorState| {
        log.lock().unwrap().push(json!([
            kind,
            name,
            text,
            state.text().as_str(),
            state.selection().main_index()
        ]));
        match name.as_str() {
            "prefix" => format!(">{text}"),
            "main" => format!("{text}:{}", state.selection().main_index()),
            "upper" => text.to_uppercase(),
            "drop" => String::new(),
            "unprefix" => match text.strip_prefix('#') {
                Some(stripped) => stripped.to_owned(),
                None => text,
            },
            _ => panic!("unknown filter {name}"),
        }
    };
    if kind == "input" {
        clipboard_input_filter(transform)
    } else {
        clipboard_output_filter(transform)
    }
}

fn setup(spec: &Value) -> (EditorState, Log) {
    let log = Log::default();
    let mut extensions = Vec::new();
    if let Some(separator) = spec["sep"].as_str() {
        extensions.push(LINE_SEPARATOR.of(separator.to_owned()));
    }
    if spec["readOnly"] == true {
        extensions.push(READ_ONLY.of(true));
    }
    for kind in ["input", "output"] {
        let Some(filters) = spec[kind].as_array() else {
            continue;
        };
        let mut seen = HashMap::new();
        for spec in filters {
            let name = spec[0].as_str().unwrap();
            let extension = seen.entry(name).or_insert_with(|| {
                let filter = filter(name.to_owned(), kind, log.clone());
                match spec[1].as_str() {
                    Some("high") => filter.with_precedence(Precedence::High),
                    Some("low") => filter.with_precedence(Precedence::Low),
                    None => filter,
                    _ => panic!("unknown precedence"),
                }
            });
            extensions.push(extension.clone());
        }
    }
    let mut state = EditorState::create(
        spec["doc"].as_str().unwrap(),
        None,
        Extension::group(extensions),
    )
    .unwrap();
    let doc = state.text().as_str();
    let ranges = spec["ranges"].as_array().unwrap().iter().map(|range| {
        let anchor = byte_at(doc, number(&range[0]));
        let head = byte_at(doc, number(&range[1]));
        let assoc = match range[2].as_i64() {
            Some(-1) => Some(Assoc::Before),
            Some(1) => Some(Assoc::After),
            _ => None,
        };
        EditorSelection::range(anchor, head, None, None, assoc)
    });
    let main = spec["main"].as_u64().map_or(0, |_| number(&spec["main"]));
    let selection = EditorSelection::new(ranges.collect(), main).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    (state, log)
}

#[test]
fn plain_text_clipboard_matches_pinned_browser_oracle() {
    for row in include_str!("fixtures/view_clipboard.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let mut editors: Vec<_> = row["editors"]
            .as_array()
            .unwrap()
            .iter()
            .map(setup)
            .collect();
        let mut memory = ClipboardLinewiseMemory::default();
        let ops = row["ops"].as_array().unwrap();
        let results = row["results"].as_array().unwrap();
        assert_eq!(ops.len(), results.len());
        for (op, expected) in ops.iter().zip(results) {
            let (state, log) = &mut editors[number(&op[0])];
            let kind = op[1].as_str().unwrap();
            let mut clipboard = None;
            let transaction = if kind == "paste" {
                plan_clipboard_paste(state, op[2].as_str().unwrap(), &memory).unwrap()
            } else if let Some(copy) = plan_clipboard_copy(state).unwrap() {
                memory.record_copy(&copy);
                clipboard = Some(copy.text().to_owned());
                if kind == "cut" {
                    copy.cut_transaction(state).unwrap()
                } else {
                    None
                }
            } else {
                None
            };
            let event = transaction
                .as_ref()
                .and_then(Transaction::user_event)
                .map(str::to_owned);
            let scroll = transaction
                .as_ref()
                .is_some_and(Transaction::scroll_into_view);
            if let Some(transaction) = transaction {
                state.update(transaction).unwrap();
            }
            let doc = state.text().as_str();
            let ranges: Vec<_> = state
                .selection()
                .ranges()
                .iter()
                .map(|range| {
                    let anchor = doc[..range.anchor()].encode_utf16().count();
                    let head = doc[..range.head()].encode_utf16().count();
                    let assoc = match range.assoc() {
                        Some(Assoc::Before) => -1,
                        Some(Assoc::After) => 1,
                        None => 0,
                    };
                    json!([anchor, head, assoc])
                })
                .collect();
            let log = std::mem::take(&mut *log.lock().unwrap());
            let mut expected = expected.as_object().unwrap().clone();
            let handled = expected.remove("handled").unwrap();
            if kind != "paste" {
                assert_eq!(handled, clipboard.is_some());
            }
            // Native paste plans edits but does not consume host events.
            let actual = json!({ "doc": doc, "ranges": ranges, "main": state.selection().main_index(),
                "event": event, "scroll": scroll, "clipboard": clipboard, "log": log });
            assert_eq!(actual, Value::Object(expected), "{}: {op}", row["case"]);
        }
    }
}

#[test]
fn scoped_linewise_clipboard_never_cuts_the_following_separator() {
    let source = "```txt\r\n猫\r\nsnow\r\n```";
    let first = source.find('猫').unwrap();
    let last = source.find("snow").unwrap();
    let body = first..last + 4;
    let mut state = EditorState::new(source);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(last + 1, last + 1)))
        .unwrap();
    assert!(
        plan_linewise_copy(&state, first..last + 1, "\r\n")
            .unwrap()
            .is_none()
    );
    let copy = plan_linewise_copy(&state, body, "\r\n").unwrap().unwrap();
    assert_eq!(copy.text(), "snow");
    state
        .update(copy.cut_transaction(&state).unwrap().unwrap())
        .unwrap();
    assert_eq!(state.text().as_str(), "```txt\r\n猫\r\n\r\n```");
    state
        .update(
            Transaction::default().with_selection(EditorSelection::single(
                first + '猫'.len_utf8(),
                first + '猫'.len_utf8(),
            )),
        )
        .unwrap();
    let mut memory = ClipboardLinewiseMemory::default();
    memory.record_copy(&copy);
    assert!(
        plan_linewise_paste(&state, "other", &memory, "\r\n")
            .unwrap()
            .is_none()
    );
    let paste = plan_linewise_paste(&state, "snow", &memory, "\r\n")
        .unwrap()
        .unwrap();
    state.update(paste).unwrap();
    assert_eq!(state.text().as_str(), "```txt\r\nsnow\r\n猫\r\n\r\n```");
    assert_eq!(state.selection().main().head(), first + "snow\r\n猫".len());
}

#[test]
fn projected_clipboard_checks_the_original_scope_and_line_bytes() {
    let source = "```json\r\n[\r猫,\r\n犬\r]\r\n```";
    let start = source.find('[').unwrap();
    let end = source.find(']').unwrap() + 1;
    let cat = source.find('猫').unwrap();
    let dog = source.find('犬').unwrap();
    let scope = start..end;
    let state = EditorState::create_from_text(
        Text::new(source),
        Some(EditorSelection::single(cat, cat)),
        clipboard_output_filter(move |text, state| {
            assert_eq!(state.text().as_str(), source);
            text
        }),
    )
    .unwrap();
    let valid = Text::new("[\n猫,\r\n犬\n]");
    let copy = plan_linewise_copy_projected(&state, scope.clone(), "\n", &valid)
        .unwrap()
        .unwrap();
    assert_eq!(copy.text(), "猫,");
    assert_eq!(copy.ranges().len(), 1);
    assert_eq!(copy.ranges()[0], cat..dog);
    let mut memory = ClipboardLinewiseMemory::default();
    memory.record_copy(&copy);
    for invalid in [
        Text::new("[\n鳥,\r\n犬\n]"),
        Text::new("[\n猫,\n\n犬\n]"),
        Text::new("[\n猫,\r\n犬\r]"),
        Text::new("[\n猫,\r\n犬\n]x"),
    ] {
        assert!(
            plan_linewise_copy_projected(&state, scope.clone(), "\n", &invalid)
                .unwrap()
                .is_none()
        );
        assert!(
            plan_linewise_paste_projected(
                &state,
                copy.text(),
                &memory,
                "\n",
                scope.clone(),
                &invalid
            )
            .unwrap()
            .is_none()
        );
    }
    let split_crlf = start - 1..end;
    let invalid = Text::new(format!("\n{}", valid.as_str()));
    assert!(
        plan_linewise_copy_projected(&state, split_crlf, "\n", &invalid)
            .unwrap()
            .is_none()
    );

    let mut outside = state.clone();
    outside
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    assert!(
        plan_linewise_paste_projected(&outside, copy.text(), &memory, "\n", scope.clone(), &valid)
            .unwrap()
            .is_none()
    );
    let paste = plan_linewise_paste_projected(&state, copy.text(), &memory, "\n", scope, &valid)
        .unwrap()
        .unwrap();
    let mut pasted = state;
    pasted.update(paste).unwrap();
    assert_eq!(
        pasted.text().as_str(),
        format!("{}猫,\n{}", &source[..cat], &source[cat..])
    );
}

#[test]
fn projected_clipboard_rejects_joining_separate_line_breaks() {
    let mut state = EditorState::new("a\rb\n\nc");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(2, 2)))
        .unwrap();
    let copy =
        plan_linewise_copy_projected(&state, 0..state.text().len(), "\n", &Text::new("a\nb\n\nc"))
            .unwrap()
            .unwrap();
    assert_eq!(copy.text(), "b");
    assert_eq!(
        copy.cut_transaction(&state).unwrap_err(),
        EditError::InvalidRange
    );
    assert_eq!(state.text().as_str(), "a\rb\n\nc");

    let mut state = EditorState::new("a\r\rx");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(2, 2)))
        .unwrap();
    let lines = Text::new("a\n\nx");
    let scope = 0..state.text().len();
    let blank = plan_linewise_copy_projected(&state, scope.clone(), "\n", &lines)
        .unwrap()
        .unwrap();
    assert_eq!(blank.text(), "");
    let mut memory = ClipboardLinewiseMemory::default();
    memory.record_copy(&blank);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    let error =
        plan_linewise_paste_projected(&state, "", &memory, "\n", scope, &lines).unwrap_err();
    assert_eq!(error, EditError::InvalidRange);
    assert_eq!(state.text().as_str(), "a\r\rx");
}

#[test]
fn linewise_cut_deletes_full_native_crlf_and_preserves_preceding_break_at_eof() {
    let selection = EditorSelection::new(
        vec![SelectionRange::new(3, 3), SelectionRange::new(9, 9)],
        0,
    )
    .unwrap();
    let mut state = EditorState::create_from_text(
        Text::new("é\r\nkeep\n🦀"),
        Some(selection),
        Extension::default(),
    )
    .unwrap();
    let copy = plan_clipboard_copy(&state).unwrap().unwrap();
    assert!(copy.is_linewise());
    assert_eq!(copy.text(), "é\n🦀");
    assert_eq!(copy.ranges(), &[0..4, 9..13]);
    let cut = copy.cut_transaction(&state).unwrap().unwrap();
    assert!(cut.selection().is_none());
    state.update(cut).unwrap();
    assert_eq!(state.text().as_str(), "keep\n");
}
