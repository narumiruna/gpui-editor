use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::CssPropertySnapshot,
        sass::{
            SassDialect, SassDocument, sass_completion_source,
            sass_completion_source_with_properties,
        },
    },
    language_data::{NativeLanguage, NativeSyntax},
};
use serde_json::{Value, json};

fn completion(result: Option<SourceResult>) -> Value {
    json!(result.map(|result| {
        let mut hash = 2_166_136_261u32;
        for item in result.options.iter() {
            for byte in item
                .label
                .bytes()
                .chain([0])
                .chain(item.kind.as_deref().unwrap().bytes())
                .chain([0])
                .chain(item.insert.as_deref().unwrap_or_default().bytes())
                .chain([b'\n'])
            {
                hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
            }
        }
        let property = result
            .options
            .first()
            .is_some_and(|item| item.kind.as_deref() == Some("property"));
        (result.from, result.to, result.options.len(), hash, property)
    }))
}

fn nodes(doc: &SassDocument) -> Value {
    json!(
        doc.nodes()
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
                let range = node.range();
                (node.name(), range.start, range.end, depth)
            })
            .collect::<Vec<_>>()
    )
}

#[test]
fn matches_pinned_call_recovery_and_controls() {
    let mut rows = include_str!("fixtures/sass_call_recovery.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let header = rows.next().unwrap();
    let properties = CssPropertySnapshot::from_style_keys(
        header["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap()),
    );
    let mut edited = None::<SassDocument>;
    let mut failures = Vec::new();
    for expected in rows {
        let source = expected["source"].as_str().unwrap();
        let dialect = match expected["dialect"].as_str().unwrap() {
            "scss" => SassDialect::Scss,
            "indented" => SassDialect::Indented,
            other => panic!("unknown dialect: {other}"),
        };
        let name = expected["name"].as_str().unwrap();
        let support = NativeLanguage::Sass(dialect).support();
        let fresh = support.parse(Text::new(source)).unwrap();
        let NativeSyntax::Sass(doc) = fresh.syntax() else {
            unreachable!()
        };
        let edit = if let Some(change) = expected["change"].as_array() {
            let from = usize::try_from(change[0].as_u64().unwrap()).unwrap();
            let to = usize::try_from(change[1].as_u64().unwrap()).unwrap();
            let insert = change[2].as_str().unwrap();
            let prior = edited.as_ref().unwrap();
            let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
            prior.reparse(&changes).unwrap()
        } else {
            SassDocument::parse(Text::new(source), dialect)
        };
        assert_eq!(edit.text().as_str(), source, "{dialect:?} {name}");
        assert_eq!(edit.dialect(), dialect);
        assert_eq!(
            nodes(&edit),
            nodes(doc),
            "fresh and edited: {dialect:?} {name}"
        );
        edited = Some(edit);

        let mut check = |service: &str, actual: Value, want: &Value| {
            if &actual != want {
                failures.push(format!(
                    "{dialect:?} {name} {service}: got {actual}, expected {want}"
                ));
            }
        };
        check("nodes", nodes(doc), &expected["nodes"]);
        let highlights: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| {
                let [tag] = span.tags else {
                    panic!("expected one Sass tag")
                };
                let name = match (tag.name(), tag.modifiers()) {
                    ("variableName", [TagModifier::Special]) => "variableName.special",
                    (name, []) => name,
                    (_, mods) => panic!("unexpected tag modifiers: {mods:?}"),
                };
                (span.range.start, span.range.end, name)
            })
            .collect();
        check("highlights", json!(highlights), &expected["highlights"]);
        let lines: Vec<_> = (1..=doc.text().line_count())
            .map(|number| {
                let line = doc.text().line(number).unwrap().range;
                (
                    doc.indentation(
                        line.start,
                        StreamSettings::default(),
                        IndentOptions::default(),
                    ),
                    doc.foldable(line.start, line.end)
                        .map(|range| (range.start, range.end)),
                )
            })
            .collect();
        check("line services", json!(lines), &expected["lines"]);
        for probe in expected["completion"].as_array().unwrap() {
            let at = usize::try_from(probe[0].as_u64().unwrap()).unwrap();
            for (idx, explicit) in [(1, false), (2, true)] {
                let label = format!("completion at {at}, explicit={explicit}");
                let direct = completion(sass_completion_source_with_properties(
                    doc,
                    at,
                    explicit,
                    &properties,
                ));
                let mut installed =
                    fresh.indexed_completions_with_css_properties(at, explicit, Some(&properties));
                assert_eq!(
                    direct,
                    completion(installed.next().map(|(slot, result)| {
                        assert_eq!(slot, 0);
                        result
                    })),
                    "installed and direct: {dialect:?} {name} {label}"
                );
                assert!(installed.next().is_none());
                check(&label, direct, &probe[idx]);

                let stock = completion(sass_completion_source(doc, at, explicit));
                let want = if probe[idx][4] == true {
                    // The shared pinned Chromium capture, in stock label order.
                    json!([probe[idx][0], null, 741, 283_478_746u32, true])
                } else {
                    probe[idx].clone()
                };
                check(&format!("stock {label}"), stock, &want);
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
