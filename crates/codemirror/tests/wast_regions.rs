use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorState, Extension, Transaction,
    language::markdown::{
        MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown, markdown_snapshot,
        markdown_with_parser,
    },
    language_data::{
        LANGUAGES, LanguageDescription, NativeLanguage, SupportedDocument,
        attached_language_snapshot, wast_extension, wast_language,
    },
};
use serde_json::{Value, json};

fn markdown_with_wast(fallback: bool) -> Extension {
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(
                |name| match name {
                    "wast" | "wat" => Some(NativeLanguage::Wast),
                    "json" => Some(NativeLanguage::Json),
                    _ => None,
                },
            ))),
            default_code_language: fallback.then_some(NativeLanguage::Wast),
            ..Default::default()
        },
    )
}

fn regions(document: &SupportedDocument) -> Vec<Value> {
    document.wast_regions().into_iter().map(|range| {
        json!({"range": [range.start, range.end], "raw": document.text().slice(range).unwrap()})
    }).collect()
}

#[test]
fn wast_regions_match_pinned_root_catalog_and_markdown_ranges() {
    for line in include_str!("fixtures/wast_regions.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let document = match mode {
            "root" => {
                let state = EditorState::with_extensions(source, wast_extension()).unwrap();
                attached_language_snapshot(&state).unwrap()
            }
            "catalog" => {
                let file = row["filename"].as_str().unwrap();
                let description = LanguageDescription::match_filename(LANGUAGES, file).unwrap();
                let support = description.native_support_load().load().unwrap();
                assert_eq!(support.language(), NativeLanguage::Wast);
                let language = support.wast_language().unwrap();
                assert!(std::ptr::eq(language, wast_language()));
                let state = EditorState::with_extensions(source, language.extension()).unwrap();
                let document = attached_language_snapshot(&state).unwrap();
                assert_eq!(language.find_regions(&state), document.wast_regions());
                document
            }
            _ => {
                let extension = match mode {
                    "controlled" => markdown_with_wast(false),
                    "fallback" => markdown_with_wast(true),
                    "stock" => markdown(MarkdownConfig::default()),
                    _ => unreachable!(),
                };
                let state = EditorState::with_extensions(source, extension).unwrap();
                markdown_snapshot(&state).unwrap()
            }
        };
        assert_eq!(document.text().as_str(), source, "{}", row["name"]);
        assert_eq!(json!(regions(&document)), row["regions"], "{}", row["name"]);
    }
}

#[test]
fn wast_regions_follow_edits_and_mount_removal_without_rewriting_retained_snapshots() {
    let source = "前 😀\r\n\r\n> ```wat\r\n> (module\r>   (func))\r> ```";
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions(source, slot.of(markdown_with_wast(false))).unwrap();
    let original = markdown_snapshot(&state).unwrap();
    let projections = |document: &SupportedDocument| -> Vec<Range<usize>> {
        let ranges = document.wast_regions();
        assert_eq!(ranges.len(), 2);
        assert!(ranges[0].end < ranges[1].start);
        for range in &ranges {
            assert_eq!(
                document.markdown_language_presence_at(range.start, Assoc::Before),
                Some(NativeLanguage::Wast)
            );
            assert_eq!(
                document.language_at(range.start, Assoc::Before),
                Some(NativeLanguage::Markdown)
            );
        }
        ranges
    };
    let prior = projections(&original);
    let language = wast_language().configure(None, Some("renamed"));
    assert_eq!(language.find_regions(&state), prior);
    assert!(language.is_active_at(&state, prior[0].start, Assoc::Before));
    assert!(!language.is_active_at(&state, prior[0].end + 1, Assoc::After));
    assert_eq!(
        original.language_at(prior[0].end + 1, Assoc::After),
        Some(NativeLanguage::Markdown)
    );
    let insert = "Alpha\n\n";
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(0..0, insert)).unwrap(),
        ))
        .unwrap();
    let current = markdown_snapshot(&state).unwrap();
    let expected: Vec<_> = prior
        .iter()
        .map(|range| range.start + insert.len()..range.end + insert.len())
        .collect();
    assert_eq!(projections(&current), expected);
    state
        .update(
            Transaction::default()
                .with_effects([slot.reconfigure(markdown(MarkdownConfig::default()))]),
        )
        .unwrap();
    assert!(markdown_snapshot(&state).unwrap().wast_regions().is_empty());
    assert_eq!(original.text().as_str(), source);
    assert_eq!(projections(&original), prior);
}
