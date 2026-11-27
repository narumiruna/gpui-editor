use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        brackets::{BracketConfig, BracketMatch, bracket_near_in},
        go::GoDocument,
    },
};
use serde_json::{Value, json};

#[test]
fn plain_scan_confuses_go_code_with_a_comment_and_literal() {
    let source = "package p\r\nfunc f() { println(/* 🦀 ) */ \"界)\"); }\r";
    let text = Text::new(source);
    let plain = |head| {
        bracket_near_in(
            &text,
            0..text.len(),
            head,
            BracketConfig::default(),
            |_, _| true,
        )
    };
    let doc = GoDocument::parse(text.clone());
    for (head, plain_end, syntax_end) in [
        (29, Some(38..39), Some(49..50)),
        (38, Some(29..30), None),
        (47, Some(20..21), None),
    ] {
        let fallback = plain(head).unwrap();
        assert_eq!(fallback.end, plain_end);
        assert_eq!(
            doc.bracket_near(head),
            Some(BracketMatch {
                start: head..head + 1,
                matched: syntax_end.is_some(),
                end: syntax_end,
            })
        );
    }
}

#[test]
fn go_feedback_matches_the_pinned_editor_across_edits() {
    let mut document = GoDocument::parse(Text::new(""));
    for line in include_str!("fixtures/go_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        document = if let Some(source) = row["source"].as_str() {
            GoDocument::parse(Text::new(source))
        } else {
            let (from, to, insert): (usize, usize, String) =
                serde_json::from_value(row["change"].clone()).unwrap();
            let changes = ChangeSet::new(vec![Change::new(from..to, insert)]).unwrap();
            document.reparse(&changes).unwrap()
        };
        let fresh = GoDocument::parse(document.text().clone());
        for doc in [&document, &fresh] {
            assert_eq!(
                doc.is_recovered(),
                row["recovered"].as_bool().unwrap(),
                "{name}"
            );
            for probe in row["hits"].as_array().unwrap() {
                let head = probe[0].as_u64().unwrap() as usize;
                let actual = doc.bracket_near(head).map_or(Value::Null, |hit| {
                    json!([
                        [hit.start.start, hit.start.end],
                        hit.end.map(|end| [end.start, end.end]),
                        hit.matched
                    ])
                });
                assert_eq!(actual, probe[1], "{name} at byte {head}");
            }
        }
        let text = document.text().as_str();
        for head in (0..text.len()).filter(|&head| !text.is_char_boundary(head)) {
            assert!(
                document.bracket_near(head).is_none(),
                "{name} at byte {head}"
            );
        }
        assert!(document.bracket_near(text.len() + 1).is_none(), "{name}");
    }
}
