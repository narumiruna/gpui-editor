use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        brackets::{BracketConfig, BracketMatch, bracket_near},
        python::PythonDocument,
    },
};
use serde_json::{Value, json};

fn record(hit: Option<BracketMatch>) -> Value {
    hit.map_or(Value::Null, |hit| {
        json!([
            [hit.start.start, hit.start.end],
            hit.end.map(|end| [end.start, end.end]),
            hit.matched
        ])
    })
}

fn hit_at(probes: &Value, head: usize) -> &Value {
    probes
        .as_array()
        .unwrap()
        .iter()
        .find(|probe| probe[0] == head)
        .map_or(&Value::Null, |probe| &probe[1])
}

#[test]
fn python_bracket_feedback_matches_the_pinned_editor_across_edits() {
    let mut document = PythonDocument::parse(Text::new(""));
    for line in include_str!("fixtures/python_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        document = if let Some(source) = row["source"].as_str() {
            PythonDocument::parse(Text::new(source))
        } else {
            let (from, to, insert): (usize, usize, String) =
                serde_json::from_value(row["change"].clone()).unwrap();
            let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
            document.reparse(&changes).unwrap()
        };
        // The native parser retains only a valid prefix in these two rows.
        // Upstream still resolves brackets in parts of the damaged suffix.
        let unparsed_hits: &[usize] = match name {
            "replace call closer with wrong bracket" => &[37, 38, 46, 47, 48, 49, 50],
            "native prefix recovery cannot see later grouping" => &[22, 23, 33, 34, 35, 36],
            _ => &[],
        };
        let text = document.text();
        let fresh = PythonDocument::parse(text.clone());
        for doc in [&document, &fresh] {
            assert_eq!(doc.is_recovered(), row["recovered"], "{name}");
            // Also check every byte without a pinned hit, including the
            // interior of CRLF and multibyte scalars.
            for head in 0..=text.len() {
                let actual = record(doc.bracket_near(head));
                let upstream = hit_at(&row["hits"], head);
                let expected = if unparsed_hits.contains(&head) {
                    assert!(!upstream.is_null(), "{name} at byte {head}");
                    &Value::Null
                } else {
                    upstream
                };
                assert_eq!(&actual, expected, "{name} at byte {head}");
            }
            assert!(doc.bracket_near(text.len() + 1).is_none(), "{name}");
        }
        for probe in row["plain"].as_array().unwrap() {
            let head = probe[0].as_u64().unwrap() as usize;
            let raw = record(bracket_near(
                text,
                head,
                BracketConfig::default(),
                |_, _| true,
            ));
            assert_eq!(raw, probe[1], "{name} at byte {head}");
            assert_ne!(&raw, hit_at(&row["hits"], head), "{name} at byte {head}");
        }
    }
}

#[test]
fn only_unmarked_brackets_use_the_normalized_utf16_scan_limit() {
    for separator in ["\r\n", "\r"] {
        for count in [4999, 5000] {
            let source = format!("# ({}{separator}# )", "😀".repeat(count));
            let end = source.len();
            let doc = PythonDocument::parse(Text::new(source));
            assert!(!doc.is_recovered());
            assert_eq!(
                doc.bracket_near(2),
                (count == 4999).then_some(BracketMatch {
                    start: 2..3,
                    end: Some(end - 1..end),
                    matched: true,
                })
            );
        }
    }
    let structural = format!("({}\r\n1)", " ".repeat(10_001));
    let end = structural.len();
    let doc = PythonDocument::parse(Text::new(structural));
    assert!(!doc.is_recovered());
    assert_eq!(
        doc.bracket_near(0),
        Some(BracketMatch {
            start: 0..1,
            end: Some(end - 1..end),
            matched: true,
        })
    );
}
