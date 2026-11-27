use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    Text, Transaction, change_filter,
    commands::CommentTokens,
    language::javascript::JavascriptOptions,
    language_data::{CloseBracketData, NativeLanguage},
};
use serde_json::{Value, json};

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn metadata(state: &EditorState, at: usize, side: Assoc) -> Value {
    let comments = state
        .language_data_at::<CommentTokens>("commentTokens", at, side)
        .into_iter()
        .map(|tokens| {
            let mut value = json!({});
            if let Some(line) = tokens.line {
                value["line"] = json!(line);
            }
            if let Some((open, close)) = tokens.block {
                value["block"] = json!({"open": open, "close": close});
            }
            value
        })
        .collect::<Vec<_>>();
    let brackets = state
        .language_data_at::<CloseBracketData>("closeBrackets", at, side)
        .into_iter()
        .map(|config| {
            let mut value = json!({});
            if let Some(brackets) = config.brackets {
                value["brackets"] = json!(brackets);
            }
            if !config.string_prefixes.is_empty() {
                value["stringPrefixes"] = json!(config.string_prefixes);
            }
            value
        })
        .collect::<Vec<_>>();
    json!({
        "commentTokens": comments,
        "closeBrackets": brackets,
        "wordChars": state.language_data_at::<&str>("wordChars", at, side),
        "indentOnInput": state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side).len(),
    })
}

#[test]
fn installed_metadata_matches_pinned_states_across_edits() {
    let mut state = EditorState::new("");
    for line in include_str!("fixtures/tracked_language_data.jsonl").lines() {
        let step: Value = serde_json::from_str(line).unwrap();
        let source = step["source"].as_str().unwrap();
        let transaction = if let Some(language) = step["language"].as_str() {
            let support = match language {
                "wast" => NativeLanguage::Wast,
                "html" => NativeLanguage::Html,
                "jsx" => NativeLanguage::Javascript(JavascriptOptions {
                    jsx: true,
                    ..JavascriptOptions::default()
                }),
                _ => panic!("unknown fixture language: {language}"),
            }
            .support();
            state = EditorState::new(source);
            Transaction::default()
                .with_effects([support.tracked_language_data_extension().append()])
        } else if let Some(changes) = step["changes"].as_array() {
            let edits = changes
                .iter()
                .map(|edit| {
                    Change::new(
                        offset(&edit[0])..offset(&edit[1]),
                        edit[2].as_str().unwrap(),
                    )
                })
                .collect();
            Transaction::new(ChangeSet::new(edits).unwrap())
        } else {
            let at = offset(&step["selection"]);
            Transaction::default().with_selection(EditorSelection::single(at, at))
        };
        state.update(transaction).unwrap();
        assert_eq!(state.text().as_str(), source);
        for probe in step["data"].as_array().unwrap() {
            let at = offset(&probe["byte"]);
            let side = match probe["side"].as_i64().unwrap() {
                -1 => Assoc::Before,
                1 => Assoc::After,
                _ => panic!("invalid fixture side"),
            };
            assert_eq!(
                metadata(&state, at, side),
                probe["values"],
                "{source:?} at {at} {side:?}"
            );
        }
    }
}

#[test]
fn fixed_wast_snapshots_keep_their_text_and_scope_checks() {
    let support = NativeLanguage::Wast.support();
    let document = Arc::new(support.parse(Text::new("(module)")).unwrap());
    let brackets = |state: &EditorState, at| {
        state.language_data_at::<CloseBracketData>("closeBrackets", at, Assoc::Before)
    };
    let mut fixed = EditorState::new(document.text().as_str());
    fixed
        .update(Transaction::default().with_effects([document.language_data_extension().append()]))
        .unwrap();
    assert_eq!(brackets(&fixed, 0).len(), 1);
    fixed
        .update(Transaction::new(
            ChangeSet::single(Change::new(0..0, ";; 😀\r\n")).unwrap(),
        ))
        .unwrap();
    assert!(brackets(&fixed, 0).is_empty());

    let source = Text::new("😀(module)\r\n");
    let end = source.len() - 2;
    for invalid in [1..end, 0..end, 4..end + 1] {
        assert!(
            document
                .language_data_extension_in(&source, invalid)
                .is_none()
        );
    }
    let extension = document
        .language_data_extension_in(&source, 4..end)
        .unwrap();
    let mut projected = EditorState::with_extensions(source.as_str(), extension).unwrap();
    assert!(brackets(&projected, 0).is_empty());
    assert_eq!(brackets(&projected, 4).len(), 1);
    assert_eq!(brackets(&projected, end).len(), 1);
    assert!(brackets(&projected, end + 1).is_empty());
    let changes = ChangeSet::single(Change::new(0..0, "x")).unwrap();
    projected.update(Transaction::new(changes)).unwrap();
    assert!(brackets(&projected, 5).is_empty());
}

#[test]
fn tracked_mounts_follow_only_accepted_edits_and_keep_native_boundary_rules() {
    let source = "😀\r\n<script>/* probe */</script>";
    let start = source.find('<').unwrap();
    let open_tag = start..start + "<script>".len();
    let filter = change_filter(move |_, _| ChangePermission::Protect(vec![open_tag.clone()]));
    let tracked = NativeLanguage::Html
        .support()
        .tracked_language_data_extension();
    let mut state =
        EditorState::with_extensions(source, Extension::group([filter, tracked])).unwrap();
    let old = state.clone();
    let changes = ChangeSet::new(vec![
        Change::new(0..0, "<!-- 新 -->\r\n"),
        Change::new(
            start..start + "<script>".len(),
            "<script type=\"text/plain\">",
        ),
    ])
    .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(state.text().as_str(), format!("<!-- 新 -->\r\n{source}"));
    let at = state.text().as_str().find("/*").unwrap();
    let words =
        |state: &EditorState, at, side| state.language_data_at::<&str>("wordChars", at, side);
    assert_eq!(words(&state, at, Assoc::Before), ["-_"]);
    assert_eq!(words(&state, at, Assoc::After), ["$"]);
    assert_eq!(
        words(&old, source.find("probe").unwrap(), Assoc::Before),
        ["$"]
    );
    let split = state.text().as_str().find('😀').unwrap() + 1;
    assert!(words(&state, split, Assoc::Before).is_empty());
    assert!(words(&state, state.text().len() + 1, Assoc::After).is_empty());
    let invalid = ChangeSet::single(Change::new(split..split, "x")).unwrap();
    assert_eq!(
        state.update(Transaction::new(invalid)),
        Err(EditError::InvalidRange)
    );
    assert_eq!(words(&state, at, Assoc::After), ["$"]);
}

#[test]
fn wast_parser_recovery_does_not_reject_edits_or_remove_metadata() {
    let tracked = NativeLanguage::Wast
        .support()
        .tracked_language_data_extension();
    let mut state = EditorState::with_extensions("", tracked).unwrap();
    // Exceeds the WAST LR stack limit. The native parser has a local fallback.
    let input = "(".repeat(4_000);
    let changes = ChangeSet::single(Change::new(0..0, &input)).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(state.text().as_str(), input);
    let tokens =
        state.language_data_at::<CommentTokens>("commentTokens", input.len(), Assoc::Before);
    let [tokens] = tokens.as_slice() else {
        panic!("expected WAST comments after parser fallback");
    };
    assert_eq!(tokens.line, Some(";;"));
    assert_eq!(tokens.block, Some(("(;", ";)")));
}
