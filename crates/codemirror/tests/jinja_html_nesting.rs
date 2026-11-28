use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::CssDocument,
        html::{
            HtmlAttributes, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlNestedAttribute,
            HtmlNestedLanguage, HtmlNestedParser, HtmlNesting,
        },
        jinja::{JinjaDocument, JinjaOptions},
        json::JsonDocument,
    },
};
use serde_json::{Value, json};

fn compare(doc: &JinjaDocument, expected: &Value, case: &str) {
    assert!(!doc.is_recovered(), "{case}");
    let html = doc.html().unwrap();
    let mounts: Vec<_> = html
        .document()
        .mounts()
        .iter()
        .map(|mount| {
            let (language, root, body) = match (mount.language(), mount.document()) {
                (HtmlEmbeddedLanguage::Custom(name), HtmlEmbeddedDocument::Custom(child)) => {
                    let json = child.downcast_ref::<JsonDocument>().unwrap();
                    (name, json.root().kind().name(), json.text())
                }
                (HtmlEmbeddedLanguage::JavaScript, HtmlEmbeddedDocument::Javascript(js)) => {
                    ("JavaScript", js.root().kind().name(), js.text())
                }
                mount => panic!("unexpected mount: {mount:?}"),
            };
            let ranges = html.source_ranges(mount.range()).unwrap();
            let source = ranges
                .iter()
                .map(|range| doc.text().slice(range.clone()).unwrap())
                .collect::<String>();
            assert_eq!(source, body.as_str(), "{case}: {language}");
            let parts: Vec<_> = ranges.into_iter().map(|r| [r.start, r.end]).collect();
            json!([parts, language, root])
        })
        .collect();
    assert_eq!(json!(mounts), expected["mounts"], "{case}: mounts");

    let mut highlights: Vec<(usize, usize, String)> = Vec::new();
    for span in doc.highlights(0..doc.text().len()).unwrap() {
        let tags = span
            .tags
            .iter()
            .map(|tag| {
                let mut name = tag.name().to_owned();
                for modifier in tag.modifiers() {
                    name.push_str(match modifier {
                        TagModifier::Definition => ".definition",
                        TagModifier::Constant => ".constant",
                        TagModifier::Function => ".function",
                        TagModifier::Standard => ".standard",
                        TagModifier::Local => ".local",
                        TagModifier::Special => ".special",
                    });
                }
                name
            })
            .collect::<Vec<_>>()
            .join("+");
        if let Some((_, end, previous)) = highlights.last_mut() {
            assert!(*end <= span.range.start, "{case}: overlapping highlights");
            if *end == span.range.start && *previous == tags {
                *end = span.range.end;
                continue;
            }
        }
        highlights.push((span.range.start, span.range.end, tags));
    }
    assert_eq!(
        json!(highlights),
        expected["highlights"],
        "{case}: highlights"
    );

    let folds: Vec<_> = (1..=doc.text().line_count())
        .map(|number| {
            let line = doc.text().line(number).unwrap().range;
            doc.foldable(line.start, line.end)
                .map(|range| [range.start, range.end])
        })
        .collect();
    assert_eq!(json!(folds), expected["folds"], "{case}: folds");
    for indent in expected["indents"].as_array().unwrap() {
        let at = indent[0].as_u64().unwrap().try_into().unwrap();
        let actual = doc.indentation(at, StreamSettings::default(), IndentOptions::default());
        assert_eq!(json!(actual), indent[1], "{case}: indentation at {at}");
    }
}

#[test]
fn matches_pinned_jinja_outer_html_routes() {
    let json_parser = HtmlNestedParser {
        name: "custom-json",
        parse: &|text| Box::new(JsonDocument::parse(text)),
    };
    let is_json_script =
        |attrs: &HtmlAttributes<'_>| attrs.get("type") == Some(&"application/json");
    let nesting = HtmlNesting {
        nested_languages: &[
            HtmlNestedLanguage {
                tag: "data-block",
                attrs: Some(&|attrs| attrs.get("lang") == Some(&"json")),
                parser: json_parser,
            },
            HtmlNestedLanguage {
                tag: "script",
                attrs: Some(&is_json_script),
                parser: json_parser,
            },
            HtmlNestedLanguage {
                tag: "script",
                attrs: Some(&is_json_script),
                parser: HtmlNestedParser {
                    name: "custom-css",
                    parse: &|text| Box::new(CssDocument::parse(text)),
                },
            },
        ],
        nested_attributes: &[HtmlNestedAttribute {
            name: "data-json",
            tag_name: Some("data-block"),
            parser: json_parser,
        }],
    };

    for line in include_str!("fixtures/jinja_html_nesting.jsonl")
        .lines()
        .skip(1)
    {
        let row: Value = serde_json::from_str(line).unwrap();
        let case = row["name"].as_str().unwrap();
        let source = Text::new(row["source"].as_str().unwrap());
        let doc =
            JinjaDocument::parse_with_nesting(source.clone(), JinjaOptions::default(), nesting);
        compare(&doc, &row, case);
        assert!(
            JinjaDocument::parse_with_nesting(source, JinjaOptions { html: None }, nesting)
                .html()
                .is_none()
        );
        if let Some(edit) = row.get("edit") {
            let from = edit["from"].as_u64().unwrap().try_into().unwrap();
            let to = edit["to"].as_u64().unwrap().try_into().unwrap();
            let insert = edit["insert"].as_str().unwrap();
            let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
            let updated = doc.reparse_with_nesting(&changes, nesting).unwrap();
            compare(
                &updated,
                &edit["expected"],
                "after changing the route predicate",
            );

            let fresh = JinjaDocument::parse_with_nesting(
                updated.text().clone(),
                JinjaOptions::default(),
                nesting,
            );
            compare(
                &fresh,
                &edit["expected"],
                "fresh parse after changing the route predicate",
            );

            let stock = doc.reparse(&changes).unwrap();
            assert_eq!(stock.text(), updated.text());
            assert!(stock.html().unwrap().document().mounts().is_empty());
        }
    }
}
