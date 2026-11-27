use std::{collections::HashMap, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Compartment, EditorState, Extension, Precedence, Transaction,
    commands::CommentTokens,
    language::{IndentOptions, fold::foldable_in_state, get_indentation},
    language_data::{
        CloseBracketData, NativeSyntax, attached_language_snapshot, json_extension, wast_extension,
    },
    word_characters,
};
use serde_json::{Value, json};

fn attachment(name: &str) -> Extension {
    match name {
        "off" => Extension::default(),
        "json" => json_extension(),
        "wast" => wast_extension(),
        "high-json" => json_extension().with_precedence(Precedence::High),
        "high-wast" => wast_extension().with_precedence(Precedence::High),
        _ => unreachable!(),
    }
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn positional_language_data_follows_the_same_attachment_as_syntax_queries() {
    let mut rows = include_str!("fixtures/attached_language_precedence.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .peekable();
    let source = rows.peek().unwrap()["source"].as_str().unwrap().to_owned();
    let compartments = [Compartment::new(), Compartment::new()];
    let mut state = EditorState::with_extensions(
        &source,
        Extension::group(
            std::iter::once(word_characters(|_, _| Some(Arc::from("@")))).chain(
                compartments
                    .iter()
                    .map(|slot| slot.of(Extension::default())),
            ),
        ),
    )
    .unwrap();
    let mut latest = HashMap::new();
    for row in rows {
        let name = &row["name"];
        state
            .update(
                Transaction::default().with_effects(
                    compartments
                        .iter()
                        .zip(row["attachments"].as_array().unwrap())
                        .map(|(slot, extension)| {
                            slot.reconfigure(attachment(extension.as_str().unwrap()))
                        }),
                ),
            )
            .unwrap();
        assert_eq!(state.text().as_str(), source);
        let snapshot = attached_language_snapshot(&state);
        let (language, tree) = match snapshot.as_deref().map(|snapshot| snapshot.syntax()) {
            Some(NativeSyntax::Json(doc)) => (Some("json"), doc.root().kind().name()),
            Some(NativeSyntax::Wast(doc)) => (Some("wast"), doc.root().kind().name()),
            None => (None, ""),
            _ => unreachable!(),
        };
        assert_eq!(json!(language), row["language"], "{name}");
        assert_eq!(json!(tree), row["tree"], "{name}");
        if let Some(snapshot) = &snapshot {
            assert_eq!(snapshot.text(), state.text());
            assert!(Arc::ptr_eq(
                snapshot,
                &attached_language_snapshot(&state).unwrap()
            ));
        }
        let before = latest.insert(language, snapshot.clone());
        let reused = before.as_ref().map(|before| match (before, &snapshot) {
            (Some(before), Some(after)) => Arc::ptr_eq(before, after),
            (None, None) => true,
            _ => false,
        });
        assert_eq!(json!(reused), row["reused"], "{name}");
        assert_eq!(
            json!(
                get_indentation(
                    &state,
                    byte(&row["indentAt"]),
                    IndentOptions::default(),
                    None
                )
                .unwrap()
            ),
            row["indent"],
            "{name}"
        );
        let fold = foldable_in_state(&state, 0, byte(&row["lineEnd"]));
        assert_eq!(
            json!(fold.map(|range| [range.start, range.end])),
            row["fold"],
            "{name}"
        );
        for check in row["metadata"].as_array().unwrap() {
            let head = byte(&check[0]);
            let expected = &check[1];
            let tokens =
                state.language_data_at::<CommentTokens>("commentTokens", head, Assoc::Before);
            let comments: Vec<_> = tokens
                .iter()
                .map(|tokens| {
                    let (open, close) = tokens.block.unwrap();
                    json!({"line": tokens.line, "block": {"open": open, "close": close}})
                })
                .collect();
            assert_eq!(
                json!(comments),
                expected["commentTokens"],
                "{name} at byte {head}"
            );
            let brackets =
                state.language_data_at::<CloseBracketData>("closeBrackets", head, Assoc::Before);
            assert_eq!(
                json!(
                    brackets
                        .iter()
                        .map(|data| data.brackets)
                        .collect::<Vec<_>>()
                ),
                expected["closeBrackets"],
                "{name} at byte {head}"
            );
            let words = state.language_data_at::<Arc<str>>("wordChars", head, Assoc::Before);
            assert_eq!(
                json!(words.iter().map(AsRef::as_ref).collect::<Vec<_>>()),
                expected["wordChars"],
                "{name} at byte {head}"
            );
        }
    }
}
