//! Custom body and attribute routes from the pinned `vue({base: html(...)})`.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::CssDocument,
        html::{
            HtmlAttributes, HtmlCustomDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage,
            HtmlNestedAttribute, HtmlNestedLanguage, HtmlNestedParser, HtmlNesting, HtmlOptions,
        },
        json::JsonDocument,
        vue::VueDocument,
    },
};
use serde_json::{Value, json};

#[test]
fn a_vue_directive_hides_the_base_value_fold_and_indent() {
    #[derive(Debug)]
    struct BaseValue(usize);

    impl HtmlCustomDocument for BaseValue {
        fn foldable(&self, _line_start: usize, line_end: usize) -> Option<Range<usize>> {
            Some(line_end..self.0)
        }

        fn indentation(
            &self,
            _position: usize,
            _settings: StreamSettings,
            _options: IndentOptions<'_>,
        ) -> Option<usize> {
            Some(99)
        }
    }

    let text = Text::new("<Widget :payload='name +\r\n other +\r\n final' />");
    let doc = VueDocument::parse_with_nesting(
        text.clone(),
        HtmlOptions::default(),
        HtmlNesting {
            nested_attributes: &[HtmlNestedAttribute {
                name: ":payload",
                tag_name: None,
                parser: HtmlNestedParser {
                    name: "unreachable-value",
                    parse: &|text| Box::new(BaseValue(text.len())),
                },
            }],
            ..HtmlNesting::default()
        },
    );
    let stock = VueDocument::parse(text);
    let first = doc.text().line(1).unwrap().range;
    let second = doc.text().line(2).unwrap().range.start;
    let settings = StreamSettings::default();
    let options = IndentOptions::default();
    assert!(doc.host().foldable(first.start, first.end).is_some());
    assert_eq!(doc.host().indentation(second, settings, options), Some(99));
    assert_eq!(doc.mounts().count(), 0);
    assert_eq!(
        doc.foldable(first.start, first.end),
        stock.foldable(first.start, first.end)
    );
    assert_eq!(
        doc.indentation(second, settings, options),
        stock.indentation(second, settings, options)
    );
}

fn highlights(doc: &VueDocument, range: Range<usize>) -> Vec<(usize, usize, String)> {
    let mut runs: Vec<(usize, usize, String)> = Vec::new();
    for span in doc.highlights(range).unwrap() {
        let tags = span
            .tags
            .iter()
            .map(|tag| {
                let mut name = tag.name().to_owned();
                for modifier in tag.modifiers() {
                    name.push('.');
                    name.push_str(match modifier {
                        TagModifier::Definition => "definition",
                        TagModifier::Constant => "constant",
                        TagModifier::Function => "function",
                        TagModifier::Standard => "standard",
                        TagModifier::Local => "local",
                        TagModifier::Special => "special",
                    });
                }
                name
            })
            .collect::<Vec<_>>()
            .join("+");
        if let Some((_, end, previous)) = runs.last_mut() {
            assert!(*end <= span.range.start);
            if *end == span.range.start && *previous == tags {
                *end = span.range.end;
                continue;
            }
        }
        runs.push((span.range.start, span.range.end, tags));
    }
    runs
}

fn compare(doc: &VueDocument, expected: &Value, case: &str) {
    let mounts: Vec<_> = doc
        .mounts()
        .map(|mount| {
            let range = mount.range();
            let (language, root, body) = match mount.document() {
                HtmlEmbeddedDocument::Custom(document) => {
                    let json = document.downcast_ref::<JsonDocument>().unwrap();
                    let HtmlEmbeddedLanguage::Custom(language) = mount.language() else {
                        panic!("expected a custom mount");
                    };
                    (language.to_owned(), json.root().kind().name(), json.text())
                }
                HtmlEmbeddedDocument::Javascript(js) => (
                    format!("{:?}", mount.language()),
                    js.root().kind().name(),
                    js.text(),
                ),
                HtmlEmbeddedDocument::Css(css) => (
                    format!("{:?}", mount.language()),
                    css.root().kind().name(),
                    css.text(),
                ),
            };
            assert_eq!(
                doc.text().slice(range.clone()),
                Some(body.as_str()),
                "{case}"
            );
            json!([range.start, range.end, language, root])
        })
        .collect();
    assert_eq!(json!(mounts), expected["mounts"], "{case}");

    let expressions: Vec<_> = doc
        .expressions()
        .iter()
        .map(|expr| {
            let range = expr.range();
            assert_eq!(
                doc.text().slice(range.clone()),
                Some(expr.document().text().as_str()),
                "{case}"
            );
            json!([range.start, range.end, format!("{:?}", expr.kind())])
        })
        .collect();
    assert_eq!(json!(expressions), expected["expressions"], "{case}");

    let full = highlights(doc, 0..doc.text().len());
    assert_eq!(json!(full), expected["highlights"], "{case}");
    for mount in doc.mounts() {
        let range = mount.range();
        let clipped: Vec<_> = full
            .iter()
            .filter_map(|(start, end, tag)| {
                let start = range.start.max(*start);
                let end = range.end.min(*end);
                (start < end).then(|| (start, end, tag.clone()))
            })
            .collect();
        assert_eq!(highlights(doc, range), clipped, "{case}");
    }

    let folds: Vec<_> = (1..=doc.text().line_count())
        .map(|number| {
            let line = doc.text().line(number).unwrap().range;
            doc.foldable(line.start, line.end)
                .map(|range| [range.start, range.end])
        })
        .collect();
    assert_eq!(json!(folds), expected["folds"], "{case}");

    for indent in expected["indents"].as_array().unwrap() {
        let at = indent[0].as_u64().unwrap().try_into().unwrap();
        let actual = doc.indentation(at, StreamSettings::default(), IndentOptions::default());
        assert_eq!(json!(actual), indent[1], "{case}: indentation at {at}");
    }
}

#[test]
fn matches_pinned_vue_custom_routes_and_edits() {
    let json_parser = HtmlNestedParser {
        name: "custom-json",
        parse: &|text| Box::new(JsonDocument::parse(text)),
    };
    let is_json_script =
        |attrs: &HtmlAttributes<'_>| attrs.get("type") == Some(&"application/json");
    let routes = [
        HtmlNestedLanguage {
            tag: "i18n",
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
    ];

    let attributes = ["payload", ":payload"].map(|name| HtmlNestedAttribute {
        name,
        tag_name: Some("Widget"),
        parser: json_parser,
    });
    let nesting = HtmlNesting {
        nested_languages: &routes,
        nested_attributes: &attributes,
    };

    for line in include_str!("fixtures/vue_custom_blocks.jsonl")
        .lines()
        .skip(1)
    {
        let row: Value = serde_json::from_str(line).unwrap();
        let case = row["name"].as_str().unwrap();
        let source = Text::new(row["source"].as_str().unwrap());
        let doc = VueDocument::parse_with_nesting(source.clone(), HtmlOptions::default(), nesting);
        compare(&doc, &row, case);
        if source.as_str().contains(":payload=") {
            assert_eq!(
                doc.host().mounts().len(),
                2,
                "base HTML retains both custom routes"
            );
            assert_eq!(doc.mounts().count(), 1, "Vue owns the directive value");
        }

        if let Some(edit) = row.get("edit") {
            let from = edit["from"].as_u64().unwrap().try_into().unwrap();
            let to = edit["to"].as_u64().unwrap().try_into().unwrap();
            let insert = edit["insert"].as_str().unwrap();
            let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
            let updated = doc.reparse_with_nesting(&changes, nesting).unwrap();
            assert_eq!(
                updated.text().as_str(),
                source.replace(from..to, insert).unwrap().as_str()
            );
            compare(
                &updated,
                &edit["expected"],
                "after reparse with the edited routes",
            );
            let fresh = VueDocument::parse_with_nesting(
                updated.text().clone(),
                HtmlOptions::default(),
                nesting,
            );
            compare(
                &fresh,
                &edit["expected"],
                "after fresh parse with the edited routes",
            );
            assert!((1..=from).contains(&updated.reuse_stats().resumed_at));
            assert!(updated.reuse_stats().nodes_reused > 0);

            let reverse = ChangeSet::single(Change::new(
                from..from + insert.len(),
                source.slice(from..to).unwrap(),
            ))
            .unwrap();
            let restored = updated.reparse_with_nesting(&reverse, nesting).unwrap();
            assert_eq!(restored.text(), &source);
            compare(&restored, &row, "after restoring the original routes");

            let stock = doc.reparse(&ChangeSet::default()).unwrap();
            assert_eq!(stock.mounts().count(), 0);
            let fresh_stock = VueDocument::parse(source.clone());
            assert_eq!(
                highlights(&stock, 0..source.len()),
                highlights(&fresh_stock, 0..source.len())
            );

            let inside_character = source.as_str().find('é').unwrap() + 1;
            assert!(
                doc.highlights(inside_character..inside_character + 1)
                    .is_none()
            );
        }
    }
}
