use gpui_editor_codemirror::{
    EditorSelection, Text,
    language_data::{LANGUAGES, LanguageDescription, NativeLanguage},
};
use serde_json::{Value, json};

fn byte_at(text: &str, pos: &Value) -> usize {
    let target = usize::try_from(pos.as_u64().unwrap()).unwrap();
    let mut units = 0;
    for (offset, ch) in text.char_indices() {
        if target == units {
            return offset;
        }
        units += ch.len_utf16();
    }
    assert_eq!(units, target);
    text.len()
}

#[test]
fn selects_parents_using_pinned_native_and_mounted_node_paths() {
    let mut differences = Vec::new();
    for line in include_str!("fixtures/native_parent_selection.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["name"].as_str().unwrap();
        let text = case["doc"].as_str().unwrap();
        let language = LanguageDescription::match_name(LANGUAGES, name, false)
            .unwrap()
            .built_in()
            .unwrap();
        let doc = language.support().parse(Text::new(text)).unwrap();
        let at = |pos| byte_at(text, pos);
        let mut selection = EditorSelection::single(at(&case["from"]), at(&case["to"]));
        let mut found = Vec::new();
        for _ in 0..12 {
            let Some(next) = doc.select_parent_syntax(&selection) else {
                break;
            };
            selection = next;
            let range = selection.main();
            found.push(json!([range.anchor(), range.head(), range.undirectional()]));
        }
        let expected: Vec<_> = case["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| json!([at(&row[0]), at(&row[1]), row[2]]))
            .collect();
        if found != expected {
            let extract = |values: Vec<Value>| {
                values
                    .into_iter()
                    .map(|v| {
                        let from = v[0].as_u64().unwrap() as usize;
                        let to = v[1].as_u64().unwrap() as usize;
                        (from, to, &text[from..to])
                    })
                    .collect::<Vec<_>>()
            };
            differences.push(format!(
                "{name} {text:?}:\n  actual: {:?}\n  expected: {:?}",
                extract(found),
                extract(expected)
            ));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn preserves_main_range_when_expanding_multiple_utf8_selections() {
    let text = "{\"a\":\"😀\",\"b\":false}";
    let doc = NativeLanguage::Json
        .support()
        .parse(Text::new(text))
        .unwrap();
    let a = text.find('😀').unwrap();
    let b = text.find("false").unwrap();
    let selection = EditorSelection::new(
        vec![
            EditorSelection::cursor(a, None, None, None),
            EditorSelection::cursor(b + 2, None, None, None),
        ],
        1,
    )
    .unwrap();
    let expanded = doc.select_parent_syntax(&selection).unwrap();
    assert_eq!(expanded.main_index(), 1);
    assert_eq!(&text[expanded.ranges()[0].range()], "\"😀\"");
    assert_eq!(&text[expanded.main().range()], "false");
    assert!(expanded.ranges().iter().all(|range| range.undirectional()));
    assert!(
        doc.select_parent_syntax(&EditorSelection::single(a + 1, a + 1))
            .is_none()
    );
}
