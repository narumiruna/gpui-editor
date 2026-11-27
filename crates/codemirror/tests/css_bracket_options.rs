use gpui_editor_codemirror::{
    Text,
    language::{
        brackets::{BracketConfig, BracketMatch},
        css::CssDocument,
    },
};
use serde_json::Value;

type Hit = ([usize; 2], Option<[usize; 2]>, bool);

#[test]
fn configured_css_brackets_match_the_pinned_plugin() {
    for row in include_str!("fixtures/css_bracket_options.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let mode = case["mode"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = match mode {
            "stylesheet" => CssDocument::parse(Text::new(source)),
            "styles" => CssDocument::parse_styles(Text::new(source)),
            _ => panic!("unknown CSS entry: {mode}"),
        };
        let options = &case["options"];
        let config = BracketConfig {
            brackets: options["brackets"].as_str().unwrap_or_default(),
            max_scan_distance: serde_json::from_value::<Option<usize>>(
                options["maxScanDistance"].clone(),
            )
            .unwrap()
            .unwrap_or_default(),
            after_cursor: options["afterCursor"].as_bool().unwrap_or(true),
        };
        let checks: Vec<(usize, Option<Hit>)> =
            serde_json::from_value(case["checks"].clone()).unwrap();
        for (head, expected) in checks {
            let range = |[from, to]: [usize; 2]| from..to;
            assert_eq!(
                doc.bracket_near_with_config(head, config),
                expected.map(|(start, end, matched)| BracketMatch {
                    start: range(start),
                    end: end.map(range),
                    matched,
                }),
                "{mode}: {name} at byte {head}"
            );
        }
        for head in 0..source.len() {
            if !source.is_char_boundary(head) {
                assert!(doc.bracket_near_with_config(head, config).is_none());
            }
        }
        assert!(
            doc.bracket_near_with_config(source.len() + 1, config)
                .is_none()
        );
    }
}
