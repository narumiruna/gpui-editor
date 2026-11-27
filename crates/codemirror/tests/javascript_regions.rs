use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{StreamSettings, javascript::JavascriptOptions},
    language_data::{LANGUAGES, NativeLanguage, SupportedDocument},
};
use serde_json::Value;

fn language(mode: &str) -> NativeLanguage {
    match mode {
        "js" | "jsx" | "ts" | "tsx" => NativeLanguage::Javascript(JavascriptOptions {
            typescript: matches!(mode, "ts" | "tsx"),
            jsx: matches!(mode, "jsx" | "tsx"),
        }),
        "html" => NativeLanguage::Html,
        "markdown" | "markdownDefault" => NativeLanguage::Markdown,
        "none" | "json" => NativeLanguage::Json,
        _ => panic!("unknown language: {mode}"),
    }
}

fn snapshot(mode: &str, source: &str) -> SupportedDocument {
    language(mode)
        .support()
        .parse_with_code_languages(
            Text::new(source),
            StreamSettings::default(),
            if mode == "markdown" { LANGUAGES } else { &[] },
        )
        .unwrap()
}

fn byte_range(value: &Value) -> Range<usize> {
    let at = |i: usize| usize::try_from(value[i].as_u64().unwrap()).unwrap();
    at(0)..at(1)
}

#[test]
fn native_regions_match_pinned_source_ranges_and_dialects() {
    for line in include_str!("fixtures/javascript_regions.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let mode = row["mode"].as_str().unwrap();
        let source = row["doc"].as_str().unwrap();
        let doc = snapshot(mode, source);
        let expected: Vec<_> = row["regions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|region| {
                let dialect = language(region["dialect"].as_str()?);
                let range = byte_range(&region["range"]);
                assert_eq!(doc.text().slice(range.clone()), region["raw"].as_str());
                Some((dialect, range))
            })
            .collect();
        let actual = doc.javascript_regions();
        assert_eq!(actual, expected, "{mode}: {source:?}");
        assert!(
            actual
                .windows(2)
                .all(|pair| pair[0].1.end <= pair[1].1.start)
        );
        for (_, range) in actual {
            for at in [range.start, range.end] {
                assert!(at == 0 || source.as_bytes().get(at - 1..=at) != Some(b"\r\n"));
            }
        }
        assert_eq!(doc.text().as_str(), source);
    }
}

#[test]
fn native_regions_supply_existing_adapter_fixture_inputs() {
    for line in include_str!("fixtures/javascript_eslint.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let mode = row["mode"].as_str().unwrap();
        let doc = snapshot(mode, row["doc"].as_str().unwrap());
        let ranges: Vec<_> = doc
            .javascript_regions()
            .into_iter()
            .map(|(_, range)| range)
            .collect();
        let expected: Vec<_> = row["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(byte_range)
            .collect();
        assert_eq!(ranges, expected, "{mode}");
        for (range, call) in ranges.into_iter().zip(row["calls"].as_array().unwrap()) {
            let raw = doc.text().slice(range).unwrap();
            assert_eq!(raw.replace("\r\n", "\n").replace('\r', "\n"), call["input"]);
        }
    }
}

#[test]
fn unaudited_mixed_grammars_do_not_return_regions() {
    for host in [
        NativeLanguage::Angular,
        NativeLanguage::Vue,
        NativeLanguage::Jinja,
        NativeLanguage::Liquid,
        NativeLanguage::Php,
    ] {
        let doc = host
            .support()
            .parse(Text::new("<script>run()</script>"))
            .unwrap();
        assert!(doc.javascript_regions().is_empty(), "{host:?}");
    }
}

#[test]
fn markdown_region_survives_joining_crlf_without_including_text_after_the_fence() {
    let source = "```js\rlet a;\r```\rshouldNotLint;";
    let mut doc = snapshot("markdown", source);
    let check = |doc: &SupportedDocument, start| {
        let range = start..start + "let a;".len();
        assert_eq!(doc.javascript_regions(), [(language("js"), range.clone())]);
        assert_eq!(doc.text().slice(range), Some("let a;"));
    };
    check(&doc, 6);
    doc.apply_changes(&ChangeSet::single(Change::new(6..6, "\n")).unwrap())
        .unwrap();
    check(&doc, 7);
    assert_eq!(doc.text().as_str(), "```js\r\nlet a;\r```\rshouldNotLint;");
    doc.apply_changes(&ChangeSet::single(Change::new(3..5, "unknown")).unwrap())
        .unwrap();
    assert!(doc.javascript_regions().is_empty());
}

#[test]
fn accepted_html_edits_refresh_region_coordinates_and_parser_identity() {
    let source = "<p>😀</p><script>one\r\ntwo;</script>";
    let mut doc = snapshot("html", source);
    let initial = 19..28;
    assert_eq!(
        doc.javascript_regions(),
        [(language("js"), initial.clone())]
    );

    doc.apply_changes(&ChangeSet::single(Change::new(0..0, "α")).unwrap())
        .unwrap();
    let shifted = initial.start + 2..initial.end + 2;
    assert_eq!(
        doc.javascript_regions(),
        [(language("js"), shifted.clone())]
    );
    assert_eq!(doc.text().slice(shifted.clone()), Some("one\r\ntwo;"));

    let opening = shifted.start - "<script>".len()..shifted.start;
    doc.apply_changes(&ChangeSet::single(Change::new(opening, "<script type=unknown>")).unwrap())
        .unwrap();
    assert!(doc.javascript_regions().is_empty());
    assert_eq!(
        doc.text().as_str(),
        "α<p>😀</p><script type=unknown>one\r\ntwo;</script>"
    );
}
