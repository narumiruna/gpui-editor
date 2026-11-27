use gpui_editor_codemirror::{
    Text,
    language::{
        brackets::{BracketConfig, BracketMatch},
        java::JavaDocument,
        python::PythonDocument,
    },
};
use serde_json::{Value, json};

// Selected valid and bounded malformed cases, not a parser parity fixture.
// The stock plugin's marks were checked against its renderMatch callback with
// SHA-512 verified @codemirror/language 6.13.1, @codemirror/lang-java 6.0.2 +
// @lezer/java 1.2.1, and @codemirror/lang-python 6.2.1 + @lezer/python 1.1.20.
const CASES: &str = include_str!("fixtures/java_python_bracket_options.jsonl");

#[test]
fn configured_hits_match_the_pinned_plugins() {
    let defaults = BracketConfig::default();
    for line in CASES.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let name = row["name"].as_str().unwrap();
        let options = &row["options"];
        let config = BracketConfig {
            brackets: options["brackets"].as_str().unwrap_or(defaults.brackets),
            max_scan_distance: options["maxScanDistance"]
                .as_u64()
                .map_or(defaults.max_scan_distance, |limit| limit as usize),
            after_cursor: options["afterCursor"]
                .as_bool()
                .unwrap_or(defaults.after_cursor),
        };
        let doc = match row["mode"].as_str().unwrap() {
            "java" => Document::Java(JavaDocument::parse(Text::new(source))),
            "python" => Document::Python(PythonDocument::parse(Text::new(source))),
            mode => panic!("unknown mode {mode}"),
        };
        assert_eq!(doc.is_recovered(), name.starts_with("missing "), "{name}");
        for check in row["checks"].as_array().unwrap() {
            let head = check[0].as_u64().unwrap() as usize;
            let actual = doc.find(head, config).map_or(Value::Null, |hit| {
                json!([
                    [hit.start.start, hit.start.end],
                    hit.end.map(|end| [end.start, end.end]),
                    hit.matched
                ])
            });
            assert_eq!(actual, check[1], "{name} at byte {head}");
        }
        for head in 1..source.len() {
            if !source.is_char_boundary(head) || source.as_bytes()[head - 1..=head] == *b"\r\n" {
                assert_eq!(doc.find(head, config), None, "{name} at byte {head}");
            }
        }
        assert_eq!(doc.find(source.len() + 1, config), None, "{name}");
        if name == "implicit defaults" {
            for head in 0..=source.len() {
                assert_eq!(
                    doc.find(head, defaults),
                    doc.find_legacy(head),
                    "{source:?} at byte {head}"
                );
            }
        }
    }
}

#[test]
fn python_config_does_not_match_in_the_unparsed_suffix() {
    let source = "first = (1)\r\nbroken = )\r\nlater = [2] # «😀»\r\n";
    let doc = PythonDocument::parse(Text::new(source));
    assert!(doc.is_recovered());
    let config = BracketConfig {
        brackets: "«»",
        ..BracketConfig::default()
    };
    let open = source.find('(').unwrap();
    let close = source.find(')').unwrap();
    assert_eq!(
        doc.bracket_near_with_config(open, config),
        Some(BracketMatch {
            start: open..open + 1,
            end: Some(close..close + 1),
            matched: true,
        })
    );
    for head in source.find("broken").unwrap()..=source.len() {
        assert_eq!(
            doc.bracket_near_with_config(head, config),
            None,
            "byte {head}"
        );
    }
}

enum Document {
    Java(JavaDocument),
    Python(PythonDocument),
}

impl Document {
    fn find(&self, head: usize, config: BracketConfig<'_>) -> Option<BracketMatch> {
        match self {
            Self::Java(doc) => doc.bracket_near_with_config(head, config),
            Self::Python(doc) => doc.bracket_near_with_config(head, config),
        }
    }

    fn find_legacy(&self, head: usize) -> Option<BracketMatch> {
        match self {
            Self::Java(doc) => doc.bracket_near(head),
            Self::Python(doc) => doc.bracket_near(head),
        }
    }

    fn is_recovered(&self) -> bool {
        match self {
            Self::Java(doc) => doc.is_recovered(),
            Self::Python(doc) => doc.is_recovered(),
        }
    }
}
