use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{brackets::BracketMatch, xml::XmlDocument},
};
use serde_json::{Value, json};

#[test]
fn xml_brackets_match_pinned_editor_probes() {
    let mut failures = Vec::new();
    for row in include_str!("fixtures/xml_brackets.jsonl").lines() {
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
        let doc = XmlDocument::parse(Text::new(source.as_str()));
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
            if json!(hit) != probe[1] {
                failures.push(format!("{input} at byte {head}: {hit:?} != {}", probe[1]));
            }
        }
        for head in 0..source.len() {
            if !source.is_char_boundary(head)
                || (head > 0 && source.as_bytes().get(head - 1..=head) == Some(b"\r\n"))
            {
                assert!(doc.bracket_near(head).is_none(), "{input} at byte {head}");
            }
        }
        assert!(doc.bracket_near(source.len() + 1).is_none());
        assert_eq!(doc.text().as_str(), source);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn xml_reparse_keeps_tag_handles_in_original_bytes() {
    let source = "é😀\r\n<漢😀>\r[\n]\r</漢😀>";
    let start = source.find('漢').unwrap();
    let end = source.rfind('漢').unwrap();
    let name_len = "漢😀".len();
    let doc = XmlDocument::parse(Text::new(source));
    let edits = ChangeSet::single(Change::new(start..start + name_len, "節")).unwrap();
    let next = doc.reparse(&edits).unwrap();
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(next.text().as_str(), "é😀\r\n<節>\r[\n]\r</漢😀>");
    assert_eq!(
        doc.bracket_near(start + '漢'.len_utf8()),
        Some(BracketMatch {
            start: start..start + name_len,
            end: Some(end..end + name_len),
            matched: true,
        })
    );
    assert_eq!(
        next.bracket_near(start + '節'.len_utf8()),
        Some(BracketMatch {
            start: start..start + '節'.len_utf8(),
            end: None,
            matched: false,
        })
    );
}
