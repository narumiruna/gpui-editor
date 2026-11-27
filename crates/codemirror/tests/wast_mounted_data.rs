use std::{collections::HashMap, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorState, Extension, Precedence, Transaction,
    commands::CommentTokens,
    language::{
        LanguageData,
        markdown::{
            MarkdownCodeLanguages, MarkdownCompletionSource, MarkdownConfig, MarkdownParserConfig,
            markdown, markdown_snapshot, markdown_with_parser,
        },
    },
    language_data::{CloseBracketData, NativeLanguage, wast_language},
};
use serde_json::{Value, json};

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn comments(state: &EditorState, at: usize, side: Assoc) -> Vec<Value> {
    state
        .language_data_at::<CommentTokens>("commentTokens", at, side)
        .into_iter()
        .map(|tokens| {
            let mut data = serde_json::Map::new();
            if let Some(line) = tokens.line {
                data.insert("line".into(), json!(line));
            }
            if let Some((open, close)) = tokens.block {
                data.insert("block".into(), json!({"open": open, "close": close}));
            }
            Value::Object(data)
        })
        .collect()
}

#[test]
fn installed_markdown_resolves_wast_facet_data_at_the_pinned_mount_boundaries() {
    let rows: HashMap<String, Value> = include_str!("fixtures/wast_mounted_data.jsonl")
        .lines()
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["step"].as_str().unwrap().into(), row)
        })
        .collect();
    let source = rows["configured mounts"]["source"].as_str().unwrap();
    let parser = Compartment::new();
    let extras = Compartment::new();
    let md = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(|name| {
                matches!(name, "wat" | "wast").then_some(NativeLanguage::Wast)
            }))),
            ..Default::default()
        },
    );
    let wast = wast_language();
    let renamed = wast.configure(None, Some("named-wast"));
    let high = renamed
        .data(LanguageData::default().with("wordChars", "@").with(
            "closeBrackets",
            CloseBracketData {
                brackets: Some(&["["]),
                ..Default::default()
            },
        ))
        .with_precedence(Precedence::High);
    let low = wast.data(LanguageData::default().with("wordChars", "$"));
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([parser.of(md), extras.of(Extension::group([high, low]))]),
    )
    .unwrap();
    let retained = state.clone();
    let mut differences = Vec::new();
    let mut widened = 0;
    let mut check = |state: &EditorState, step| {
        let row = &rows[step];
        let snapshot = markdown_snapshot(state).unwrap();
        assert_eq!(
            snapshot.text().as_str(),
            row["source"].as_str().unwrap(),
            "{step}"
        );
        let regions: Vec<_> = renamed
            .find_regions(state)
            .into_iter()
            .map(|range| [range.start, range.end])
            .collect();
        assert_eq!(json!(regions), row["regions"], "{step}");
        for probe in row["probes"].as_array().unwrap() {
            let at = byte(&probe[0]);
            for (side, expected) in [(Assoc::Before, &probe[1]), (Assoc::After, &probe[2])] {
                let active = renamed.is_active_at(state, at, side);
                assert_eq!(
                    json!(active),
                    expected["active"],
                    "{step} byte {at} {side:?}"
                );
                let token_lists =
                    state.language_data_at::<CloseBracketData>("closeBrackets", at, side);
                let actual = json!({
                    "active": active,
                    "words": state.language_data_at::<&str>("wordChars", at, side),
                    "comments": comments(state, at, side),
                    "brackets": token_lists.iter().map(|data| data.brackets).collect::<Vec<_>>(),
                    "completionSources": state.language_data_at::<MarkdownCompletionSource>("autocomplete", at, side).len(),
                });
                if actual != *expected {
                    differences.push(format!(
                        "{step} byte {at} {side:?}: received {actual}; expected {expected}"
                    ));
                }
                if active && snapshot.language_at(at, side) != Some(NativeLanguage::Wast) {
                    widened += 1;
                    assert_eq!(
                        snapshot.language_at(at, side),
                        Some(NativeLanguage::Markdown)
                    );
                    assert!(
                        snapshot
                            .data_at(at, side)
                            .iter()
                            .all(|data| data.get::<CloseBracketData>("closeBrackets").is_none())
                    );
                }
            }
        }
    };
    check(&state, "configured mounts");
    state
        .update(Transaction::default().with_effects([extras.reconfigure(Extension::default())]))
        .unwrap();
    check(&state, "remove shared metadata");
    let changes = ChangeSet::single(Change::new(0..0, "Lead\n\n")).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    check(&state, "shift the mounts");
    state
        .update(
            Transaction::default()
                .with_effects([parser.reconfigure(markdown(MarkdownConfig::default()))]),
        )
        .unwrap();
    check(&state, "remove WAST mounts");
    check(&retained, "retained");
    assert!(widened > 0);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
