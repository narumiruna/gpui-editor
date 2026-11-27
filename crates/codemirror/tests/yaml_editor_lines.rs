use gpui_editor_codemirror::{
    Text,
    language::{IndentOptions, StreamSettings, yaml::YamlDocument},
};
use serde_json::{Value, json};

#[test]
fn yaml_indents_and_folds_mixed_line_endings_in_original_bytes() {
    for row in include_str!("fixtures/yaml_editor_lines.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = YamlDocument::parse(Text::new(source));
        let settings = StreamSettings::default();
        let options = IndentOptions::default();
        assert_eq!(doc.text().as_str(), source);
        for line in case["lines"].as_array().unwrap() {
            let (start, end, fold, indent, end_indent): (
                usize,
                usize,
                Option<[usize; 2]>,
                Option<usize>,
                Option<usize>,
            ) = serde_json::from_value(line.clone()).unwrap();
            assert_eq!(
                doc.foldable(start, end),
                fold.map(|[a, b]| a..b),
                "{source:?} byte {start}"
            );
            assert_eq!(
                doc.indentation(start, settings, options),
                indent,
                "{source:?} byte {start}"
            );
            assert_eq!(
                doc.indentation(end, settings, options),
                end_indent,
                "{source:?} byte {end}"
            );
        }
        let lines: Vec<_> = (1..=doc.syntax_lines().line_count())
            .map(|number| {
                let range = doc.syntax_lines().line(number).unwrap().range;
                json!([range.start, range.end])
            })
            .collect();
        let pinned: Vec<_> = case["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| json!([line[0], line[1]]))
            .collect();
        assert_eq!(lines, pinned, "{source:?}");
    }
}
