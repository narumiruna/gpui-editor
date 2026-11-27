use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Transaction,
    commands::CommentTokens,
    language::markdown::{
        MarkdownCodeLanguages, MarkdownConfig, MarkdownDialect, MarkdownNodeKind,
        MarkdownParserConfig, markdown, markdown_completions, markdown_snapshot,
        markdown_with_parser,
    },
    language_data::{
        LanguageDescription, LoadInvocation, LoadSource, NativeLanguage, NativeSyntax,
    },
};
use serde_json::{Value, json};

fn alias(
    name: &'static str,
    aliases: &'static [&'static str],
    module: &'static str,
    export: &'static str,
) -> LanguageDescription {
    LanguageDescription {
        name,
        aliases,
        extensions: &[],
        filename: None,
        source: LoadSource {
            module,
            export,
            invocation: LoadInvocation::Call,
        },
    }
}

fn parser(mode: &str) -> Extension {
    let config = MarkdownConfig::default();
    let parser = match mode {
        "catalog" | "edit-info" => MarkdownParserConfig {
            base: MarkdownDialect::Extended,
            code_languages: Some(MarkdownCodeLanguages::Catalog(Arc::from([
                alias("Data JSON", &["data"], "@codemirror/lang-json", "json"),
                alias("Style CSS", &["style"], "@codemirror/lang-css", "css"),
            ]))),
            default_code_language: Some(NativeLanguage::Css),
        },
        "resolver" => MarkdownParserConfig {
            base: MarkdownDialect::CommonMark,
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(|kind| {
                matches!(kind, "DATA" | "style" | "").then_some(NativeLanguage::Css)
            }))),
            default_code_language: Some(NativeLanguage::Json),
        },
        "stock" | "restore-stock" => return markdown(config),
        _ => panic!("unknown oracle configuration {mode}"),
    };
    markdown_with_parser(config, parser)
}

#[test]
fn markdown_parser_options_match_the_pinned_mounted_services() {
    let mut rows = include_str!("fixtures/markdown_parser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .peekable();
    let first = rows.peek().unwrap()["source"].as_str().unwrap();
    let compartment = Compartment::new();
    let mut state = EditorState::with_extensions(first, compartment.of(parser("stock"))).unwrap();
    for row in rows {
        let mode = row["mode"].as_str().unwrap();
        let before = markdown_snapshot(&state).unwrap();
        if mode != "stock" {
            let changes = if row["change"].is_array() {
                let (from, to, insert): (usize, usize, String) =
                    serde_json::from_value(row["change"].clone()).unwrap();
                ChangeSet::single(Change::new(from..to, insert)).unwrap()
            } else {
                ChangeSet::default()
            };
            state
                .update(
                    Transaction::new(changes).with_effects([compartment.reconfigure(parser(mode))]),
                )
                .unwrap();
        }
        let snapshot = markdown_snapshot(&state).unwrap();
        if mode != "stock" {
            assert!(!Arc::ptr_eq(&before, &snapshot), "replaced parser: {mode}");
        }
        assert_eq!(state.text().as_str(), row["source"], "{mode}");
        assert_eq!(snapshot.text(), state.text(), "{mode}");
        let NativeSyntax::Markdown(syntax) = snapshot.syntax() else {
            panic!("expected Markdown")
        };
        assert_eq!(
            syntax
                .nodes()
                .any(|node| node.kind() == MarkdownNodeKind::TaskMarker),
            row["task"],
            "{mode}"
        );
        for probe in row["probes"].as_array().unwrap() {
            let at = serde_json::from_value::<usize>(probe["at"].clone()).unwrap();
            let language = match probe["language"].as_str().unwrap() {
                "html" => NativeLanguage::Html,
                "json" => NativeLanguage::Json,
                "css" => NativeLanguage::Css,
                "markdown" => NativeLanguage::Markdown,
                unexpected => panic!("unexpected language {unexpected}"),
            };
            assert_eq!(
                snapshot.language_at(at, Assoc::Before),
                Some(language),
                "{mode} at {at}"
            );
            let block = state
                .language_data_at::<CommentTokens<'static>>("commentTokens", at, Assoc::Before)
                .into_iter()
                .find_map(|tokens| tokens.block);
            assert_eq!(json!(block), probe["comment"], "{mode} at {at}");
            let variable = markdown_completions(&state, at, false)
                .iter()
                .any(|result| result.options.iter().any(|option| option.label == "--tone"));
            assert_eq!(variable, probe["variable"], "{mode} at {at}");
        }
    }
}

#[test]
fn accepted_edits_keep_the_active_parser_and_remap_the_fence_body() {
    let original = "```DATA attrs\r\n📚\r\n```\r\n";
    let mut state = EditorState::with_extensions(original, parser("resolver")).unwrap();
    let snapshot = markdown_snapshot(&state).unwrap();
    let inside = original.find('📚').unwrap();
    assert_eq!(
        snapshot.language_at(inside, Assoc::After),
        Some(NativeLanguage::Css)
    );
    state
        .update(Transaction::default().with_selection(EditorSelection::single(inside, inside)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot, &markdown_snapshot(&state).unwrap()));
    // The resolver is case-sensitive. Once the label changes, the fallback applies.
    let edits = ChangeSet::single(Change::new(3..7, "data")).unwrap();
    state.update(Transaction::new(edits)).unwrap();
    let updated = markdown_snapshot(&state).unwrap();
    assert_eq!(
        updated.language_at(inside, Assoc::After),
        Some(NativeLanguage::Json)
    );
    assert_eq!(updated.language_at(inside + 1, Assoc::After), None);
    assert_eq!(snapshot.text().as_str(), original);
    assert_eq!(updated.text().as_str(), original.replace("DATA", "data"));
}
