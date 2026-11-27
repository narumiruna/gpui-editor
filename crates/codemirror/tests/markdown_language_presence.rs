use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditorState, Extension, Text, Transaction,
    commands::CommentTokens,
    language::markdown::{
        MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown, markdown_snapshot,
        markdown_with_parser,
    },
    language_data::{NativeLanguage, SupportedDocument},
};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/markdown_language_presence.jsonl");

fn resolve(name: &str) -> Option<NativeLanguage> {
    match name {
        "markdown" => Some(NativeLanguage::Markdown),
        "wast" => Some(NativeLanguage::Wast),
        "json" => Some(NativeLanguage::Json),
        "html" => Some(NativeLanguage::Html),
        "javascript" | "js" => Some(NativeLanguage::Javascript(Default::default())),
        _ => None,
    }
}

fn extension(mode: &str) -> Extension {
    if mode == "stock" {
        return markdown(MarkdownConfig::default());
    }
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(resolve))),
            default_code_language: (mode == "fallback").then_some(NativeLanguage::Wast),
            ..Default::default()
        },
    )
}

fn check_pins(row: &Value) {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let packages = row["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 9);
    for package in packages {
        let pin = ["packages", "referenceDependencies", "runtimeDependencies"]
            .into_iter()
            .flat_map(|group| manifest[group].as_array().unwrap())
            .find(|pin| pin["name"] == package["name"])
            .unwrap();
        for key in ["version", "tarball", "integrity"] {
            assert_eq!(package[key], pin[key]);
        }
    }
}

// topNodeAt also widens two nested <script> boundaries. The query does not
// change the nested HTML policy, even inside a mounted HTML fence.
fn nested_gap(raw: &str, at: usize, side: Assoc) -> bool {
    raw.find("<script>let x=1;</script>")
        .is_some_and(|start| match side {
            Assoc::Before => at == start + "<script>".len(),
            Assoc::After => at == start + "<script>let x=1;".len(),
        })
}

fn widened_endpoint(row: &Value, at: usize, side: Assoc) -> bool {
    row["fences"].as_array().unwrap().iter().any(|fence| {
        let [start, end]: [usize; 2] = serde_json::from_value(fence["owner"].clone()).unwrap();
        fence["ranges"].as_array().unwrap().iter().any(|range| {
            let [from, to]: [usize; 2] = serde_json::from_value(range.clone()).unwrap();
            from < to
                && match side {
                    Assoc::Before => at == from && start < at && at <= end,
                    Assoc::After => at == to && start <= at && at < end,
                }
        })
    })
}

#[test]
fn markdown_language_presence_matches_the_pinned_state_facet() {
    let mut rows = FIXTURE
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    check_pins(&rows.next().unwrap());
    let mut state = EditorState::new("");
    for row in rows {
        let raw = row["raw"].as_str().unwrap();
        let name = row["name"].as_str().unwrap();
        let ext = extension(row["mode"].as_str().unwrap());
        let fresh = EditorState::with_extensions(raw, ext).unwrap();
        let edited = row["edit"].is_array();
        if edited {
            let (from, to, insert): (usize, usize, String) =
                serde_json::from_value(row["edit"].clone()).unwrap();
            let changes = ChangeSet::new(vec![Change::new(from..to, insert)]).unwrap();
            state.update(Transaction::new(changes)).unwrap();
        } else {
            state = fresh.clone();
        }
        assert_eq!(state.text().as_str(), raw, "{name}");
        let snapshot = markdown_snapshot(&state).unwrap();
        let cold = markdown_snapshot(&fresh).unwrap();
        for probe in row["probes"].as_array().unwrap() {
            let (at, before, after): (usize, Option<String>, Option<String>) =
                serde_json::from_value(probe.clone()).unwrap();
            for (side, pinned) in [(Assoc::Before, before), (Assoc::After, after)] {
                let label = format!("{name} at byte {at} {side:?}, edit {}", row["edit"]);
                let pinned = pinned.map(|name| resolve(&name).unwrap());
                let expected = if nested_gap(raw, at, side) {
                    assert!(
                        matches!(pinned, Some(NativeLanguage::Javascript(_))),
                        "{label}"
                    );
                    Some(NativeLanguage::Html)
                } else {
                    pinned
                };
                assert_eq!(
                    snapshot.markdown_language_presence_at(at, side),
                    expected,
                    "{label}"
                );
                if edited {
                    assert_eq!(
                        cold.markdown_language_presence_at(at, side),
                        expected,
                        "fresh: {label}"
                    );
                }

                let route = snapshot.language_at(at, side);
                if pinned.is_some() {
                    let legacy = if widened_endpoint(&row, at, side) {
                        Some(NativeLanguage::Markdown)
                    } else {
                        expected
                    };
                    assert_eq!(route, legacy, "existing route: {label}");
                } else {
                    // The old route still accepts the byte between CR and LF.
                    assert_eq!(
                        route.is_some(),
                        raw.is_char_boundary(at),
                        "existing route: {label}"
                    );
                }
                let tokens = |tokens: &CommentTokens<'static>| (tokens.line, tokens.block);
                let legacy_tokens = route
                    .map(NativeLanguage::comment_tokens)
                    .as_ref()
                    .map(tokens)
                    .filter(|(line, block)| line.is_some() || block.is_some());
                let direct = snapshot.data_at(at, side);
                let direct = direct
                    .iter()
                    .find_map(|data| data.get::<CommentTokens>("commentTokens"));
                let installed = state.language_data_at::<CommentTokens>("commentTokens", at, side);
                assert_eq!(direct.map(tokens), legacy_tokens, "direct data: {label}");
                let installed_tokens = if expected == Some(NativeLanguage::Wast) {
                    Some(tokens(&NativeLanguage::Wast.comment_tokens()))
                } else {
                    legacy_tokens
                };
                assert_eq!(
                    installed.first().map(tokens),
                    installed_tokens,
                    "installed data: {label}"
                );
            }
        }
    }
}

#[test]
fn non_markdown_keeps_native_language_routing() {
    let document: SupportedDocument = NativeLanguage::Html
        .support()
        .parse(Text::new("<script>let x=1;</script>\r\n"))
        .unwrap();
    for at in 0..=document.text().len() + 1 {
        for side in [Assoc::Before, Assoc::After] {
            assert_eq!(
                document.markdown_language_presence_at(at, side),
                document.language_at(at, side)
            );
        }
    }
}
