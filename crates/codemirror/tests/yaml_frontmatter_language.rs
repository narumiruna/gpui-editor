//! Oracle fixtures from @codemirror/lang-yaml 6.1.3 and @codemirror/lang-json
//! 6.0.2, with the pinned CodeMirror and Lezer dependencies. Expected values use
//! syntaxTree, highlightTree, and foldable on the default editor document.
//! Oracle UTF-16 positions map back to the original UTF-8 bytes and line endings.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::yaml::frontmatter_language::{YamlFrontmatterDocument, YamlFrontmatterLanguage},
    language_data::{NativeLanguage, NativeSyntax, SyntaxUpdateError},
};
use serde_json::Value;

fn pos(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn range(value: &Value) -> Option<Range<usize>> {
    (!value.is_null()).then(|| pos(&value[0])..pos(&value[1]))
}

fn highlights(doc: &YamlFrontmatterDocument, range: Range<usize>) -> Vec<(Range<usize>, &str)> {
    doc.highlights(range)
        .unwrap()
        .map(|span| (span.range, span.tags[0].name()))
        .collect()
}

#[test]
fn frontmatter_and_body_mounts_match_pinned_editor() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Json);
    for row in include_str!("fixtures/yaml_frontmatter.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let text = Text::new(case["source"].as_str().unwrap());
        let doc = language.parse(text.clone()).unwrap();
        let front = doc.frontmatter();
        assert_eq!(doc.text(), &text, "{name}");
        assert_eq!(
            front.map(|f| f.opening()),
            range(&case["opening"]),
            "{name}"
        );
        assert_eq!(
            front.map(|f| f.yaml().root().range()),
            range(&case["yaml"]),
            "{name}"
        );
        assert_eq!(
            front.and_then(|f| f.closing()),
            range(&case["closing"]),
            "{name}"
        );
        let body = range(&case["body"]).unwrap();
        assert_eq!(doc.body_range(), body, "{name}");
        assert!(matches!(doc.body(), NativeSyntax::Json(_)), "{name}");
        assert_eq!(
            doc.body().text().as_str(),
            text.slice(body).unwrap(),
            "{name}"
        );
        if let Some(front) = front {
            assert_eq!(front.yaml().text(), &text, "{name}");
        }
        if let Some(expected) = case["highlights"].as_array() {
            assert_eq!(
                highlights(&doc, 0..text.len()),
                expected
                    .iter()
                    .map(|span| (range(span).unwrap(), span[2].as_str().unwrap()))
                    .collect::<Vec<_>>(),
                "{name}"
            );
        }
        if let Some(expected) = case["folds"].as_array() {
            let actual: Vec<_> = (1..=text.line_count())
                .filter_map(|number| {
                    let line = text.line(number)?.range;
                    let fold = doc.foldable(line.start, line.end)?;
                    Some([line.start, line.end, fold.start, fold.end])
                })
                .collect();
            assert_eq!(
                actual,
                expected
                    .iter()
                    .map(|fold| std::array::from_fn(|i| pos(&fold[i])))
                    .collect::<Vec<_>>(),
                "{name}"
            );
        }
    }
}

#[test]
fn clips_cross_mount_highlights_and_rejects_invalid_byte_positions() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Json);
    let text = Text::new("---\r\nname: \"😀\"\r\n---\r\n{\"array\":\r\n  [1, true]\r\n}\r\n");
    let len = text.len();
    let doc = language.parse(text).unwrap();
    assert_eq!(
        highlights(&doc, 7..29),
        [
            (7..9, "propertyName"),
            (9..10, "separator"),
            (11..17, "string"),
            (19..24, "meta"),
            (24..25, "brace"),
            (25..29, "propertyName"),
        ]
    );
    assert_eq!(highlights(&doc, 12..16), [(12..16, "string")]);
    assert!(highlights(&doc, 24..24).is_empty());
    assert!(highlights(&doc, len..len).is_empty());
    for range in [13..16, 12..15, 0..len + 1, Range { start: 25, end: 24 }] {
        assert!(doc.highlights(range.clone()).is_none());
        assert_eq!(doc.foldable(range.start, range.end), None);
    }
}

#[test]
fn reuses_both_mounts_and_redetects_delimiters_when_edited() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Json);
    let source = format!(
        "---\r\nentries:\r\n{}last: old\r\n---\r\n[\r\n{}  \"end😀\"\r\n]\r\n",
        "  - item: 😀\r\n".repeat(100),
        "  {\"value\": true},\r\n".repeat(100),
    );
    let mut doc = language.parse(Text::new(&source)).unwrap();
    let yaml = source.find("old").unwrap();
    let json = source.find("end😀").unwrap();
    let input = ChangeSet::new(vec![
        Change::new(yaml..yaml + 3, "新しい"),
        Change::new(json..json + 3, "終"),
    ])
    .unwrap();
    doc.apply_changes(&input).unwrap();
    assert!(doc.frontmatter().unwrap().yaml().reuse_stats().nodes_reused > 100);
    assert!(doc.body_reuse_stats().nodes_reused > 100);
    let check = |doc: &YamlFrontmatterDocument| {
        let fresh = language.parse(doc.text().clone()).unwrap();
        assert_eq!(doc.body_range(), fresh.body_range());
        assert_eq!(doc.body().text(), fresh.body().text());
        assert_eq!(
            highlights(doc, 0..doc.text().len()),
            highlights(&fresh, 0..fresh.text().len())
        );
        for number in 1..=doc.text().line_count() {
            let line = doc.text().line(number).unwrap();
            assert_eq!(
                doc.foldable(line.range.start, line.range.end),
                fresh.foldable(line.range.start, line.range.end)
            );
        }
    };
    check(&doc);

    let before_error = doc.text().clone();
    let inside = before_error.as_str().find('😀').unwrap() + 1;
    let invalid = ChangeSet::single(Change::new(inside..inside, "?")).unwrap();
    assert_eq!(
        doc.apply_changes(&invalid),
        Err(SyntaxUpdateError::Edit(EditError::InvalidRange))
    );
    assert_eq!(doc.text(), &before_error);
    assert!(doc.body_reuse_stats().nodes_reused > 100);
    check(&doc);

    let closing = doc.frontmatter().unwrap().closing().unwrap();
    let crossing =
        ChangeSet::single(Change::new(closing.start..closing.end + 1, "---\r\n[")).unwrap();
    doc.apply_changes(&crossing).unwrap();
    assert_eq!(doc.body_reuse_stats().nodes_reused, 0);
    check(&doc);

    let closing = doc.frontmatter().unwrap().closing().unwrap();
    doc.apply_changes(&ChangeSet::single(Change::new(closing.clone(), "")).unwrap())
        .unwrap();
    assert_eq!(doc.body_range(), doc.text().len()..doc.text().len());
    assert!(doc.frontmatter().unwrap().closing().is_none());
    check(&doc);

    doc.apply_changes(&ChangeSet::single(Change::new(0..3, "---x")).unwrap())
        .unwrap();
    assert!(doc.frontmatter().is_none());
    assert_eq!(doc.body_range(), 0..doc.text().len());
    check(&doc);
}

#[test]
fn uses_the_callers_body_language() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Markdown);
    let text = Text::new("---\ntitle: 😀\n---\n# Hello\n");
    let doc = language.parse(text.clone()).unwrap();
    assert!(matches!(doc.body(), NativeSyntax::Markdown(_)));
    let body = doc.body_range();
    assert_eq!(text.slice(body.clone()), Some("# Hello\n"));
    assert!(
        highlights(&doc, body)
            .iter()
            .any(|&(_, tag)| tag == "heading1")
    );
}
