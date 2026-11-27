use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        brackets::BracketMatch,
        html::{HtmlDocument, HtmlOptions},
    },
};
use serde_json::{Value, json};

#[test]
fn html_host_brackets_match_pinned_editor_probes() {
    for row in include_str!("fixtures/html_brackets.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let input = &case["source"];
        let source = if let Some(text) = input.as_str() {
            text.to_owned()
        } else {
            input
                .as_array()
                .unwrap()
                .iter()
                .map(|part| {
                    part.as_str().map_or_else(
                        || {
                            part[0]
                                .as_str()
                                .unwrap()
                                .repeat(part[1].as_u64().unwrap() as usize)
                        },
                        str::to_owned,
                    )
                })
                .collect::<String>()
        };
        let options = HtmlOptions {
            match_closing_tags: case["options"]["matchClosingTags"]
                .as_bool()
                .unwrap_or(true),
            self_closing_tags: case["options"]["selfClosingTags"]
                .as_bool()
                .unwrap_or(false),
        };
        let doc = HtmlDocument::parse_with_options(Text::new(source.as_str()), options);
        for probe in case["hits"].as_array().unwrap() {
            let head = probe[0].as_u64().unwrap() as usize;
            let coordinates = |range: Range<usize>| [range.start, range.end];
            let hit = doc.bracket_near(head).map(|hit| {
                json!([
                    coordinates(hit.start),
                    hit.end.map(coordinates),
                    hit.matched
                ])
            });
            assert_eq!(json!(hit), probe[1], "{input} {options:?} at byte {head}");
        }
        for head in 0..source.len() {
            if !source.is_char_boundary(head) {
                assert!(doc.bracket_near(head).is_none());
            }
        }
        assert!(doc.bracket_near(source.len() + 1).is_none());
        assert_eq!(doc.text().as_str(), source);
    }
}

#[test]
fn html_host_brackets_do_not_use_mounted_syntax_as_html_text() {
    let source = "<a onclick=\"f(x)\">(</a><script>f(x)</script>)";
    let doc = HtmlDocument::parse(Text::new(source));
    for (at, _) in source.match_indices("f(x)") {
        assert!(doc.bracket_near(at + 2).is_none());
        assert!(doc.bracket_near(at + 3).is_none());
    }
    let script = source.find("<script>").unwrap();
    let body_start = script + "<script>".len();
    let body_end = source.find("</script>").unwrap();
    assert_eq!(
        doc.bracket_near(body_start).unwrap().start,
        body_start - 1..body_start
    );
    assert_eq!(
        doc.bracket_near(body_end).unwrap().start,
        body_end..body_end + 2
    );
    let open = source.find(">(").unwrap() + 1;
    let close = source.len() - 1;
    assert_eq!(
        doc.bracket_near(open + 1),
        Some(BracketMatch {
            start: open..open + 1,
            end: Some(close..close + 1),
            matched: true,
        })
    );
}

#[test]
fn html_reparse_keeps_bracket_ranges_in_the_original_bytes() {
    let source = "😀\r\n<div>\r[\n]\r</div>";
    let doc = HtmlDocument::parse(Text::new(source));
    assert!(doc.bracket_near(5).is_none()); // Between CR and LF.
    let start = source.find("div").unwrap();
    let end = source.rfind("div").unwrap();
    let edits = ChangeSet::single(Change::new(start..start + 3, "section")).unwrap();
    let next = doc.reparse(&edits).unwrap();
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(next.text().as_str(), "😀\r\n<section>\r[\n]\r</div>");
    assert_eq!(
        next.bracket_near(start + 1),
        Some(BracketMatch {
            start: start..start + 7,
            end: None,
            matched: false,
        })
    );
    assert_eq!(
        doc.bracket_near(start + 1),
        Some(BracketMatch {
            start: start..start + 3,
            end: Some(end..end + 3),
            matched: true,
        })
    );
}
