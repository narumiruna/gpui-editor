use gpui_editor_codemirror::{
    Text,
    language::{
        brackets::{BracketConfig, BracketMatch},
        go::GoDocument,
        rust::RustDocument,
    },
};
use serde_json::{Value, json};

// The pinned fixture uses selected byte carets, not parser parity.
#[test]
fn configured_raw_hits_match_pinned_cases() {
    for line in include_str!("fixtures/go_rust_bracket_options.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let options = &row["options"];
        let config = BracketConfig {
            brackets: options["brackets"].as_str().unwrap_or_default(),
            max_scan_distance: options["maxScanDistance"].as_u64().unwrap_or_default() as usize,
            after_cursor: options["afterCursor"].as_bool().unwrap_or(true),
        };
        let doc = match row["mode"].as_str().unwrap() {
            "go" => Document::Go(GoDocument::parse(Text::new(source))),
            "rust" => Document::Rust(RustDocument::parse(Text::new(source))),
            mode => panic!("unknown mode {mode}"),
        };
        for check in row["checks"].as_array().unwrap() {
            let head = check[0].as_u64().unwrap() as usize;
            let hit = doc.find(head, config).map_or(Value::Null, |hit| {
                json!([
                    [hit.start.start, hit.start.end],
                    hit.end.map(|end| [end.start, end.end]),
                    hit.matched
                ])
            });
            assert_eq!(hit, check[1], "{} at byte {head}", row["name"]);
        }
        for head in 1..source.len() {
            if !source.is_char_boundary(head) || source.as_bytes()[head - 1..=head] == *b"\r\n" {
                assert_eq!(doc.find(head, config), None, "{source:?} at byte {head}");
            }
        }
        assert_eq!(doc.find(source.len() + 1, config), None, "{source:?}");
    }
}

#[test]
fn stock_marks_distinguish_default_and_configured_hits() {
    let go = Document::Go(GoDocument::parse(Text::new(
        "func f(a int, b int, c int) {}",
    )));
    let rust = Document::Rust(RustDocument::parse(Text::new("fn f<T>(x: Vec<T>) {}")));
    let cap = Document::Go(GoDocument::parse(Text::new("/*(«😀»\r\n)*/")));
    let default = BracketConfig::default();
    let commas = BracketConfig {
        brackets: ",)",
        ..default
    };
    let angles = BracketConfig {
        brackets: "<>",
        ..default
    };
    let limited = BracketConfig {
        brackets: "()[]{}«»",
        max_scan_distance: 1,
        ..default
    };
    // Spans and classes measured from the stock plugin without renderMatch.
    for (doc, head, config, expected) in [
        (&go, 12, default, &[][..]),
        (&go, 26, default, &[(6, 7, true), (26, 27, true)]),
        (&go, 12, commas, &[(12, 13, false)]),
        (&go, 26, commas, &[(19, 20, false), (26, 27, false)]),
        (&rust, 4, default, &[][..]),
        (&rust, 4, angles, &[(4, 5, true), (6, 7, true)]),
        (&cap, 3, default, &[(2, 3, true), (13, 14, true)]),
        (&cap, 3, limited, &[(3, 5, true), (9, 11, true)]),
    ] {
        let mut marks = Vec::new();
        if let Some(hit) = doc.find(head, config) {
            marks.extend(
                [Some(hit.start), hit.end]
                    .into_iter()
                    .flatten()
                    .map(|span| (span.start, span.end, hit.matched)),
            );
            marks.sort_unstable();
        }
        assert_eq!(marks, expected, "{config:?} at byte {head}");
    }
}

enum Document {
    Go(GoDocument),
    Rust(RustDocument),
}

impl Document {
    fn find(&self, head: usize, config: BracketConfig<'_>) -> Option<BracketMatch> {
        match self {
            Self::Go(doc) => doc.bracket_near_with_config(head, config),
            Self::Rust(doc) => doc.bracket_near_with_config(head, config),
        }
    }
}
