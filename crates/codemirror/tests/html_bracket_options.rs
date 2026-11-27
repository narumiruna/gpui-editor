use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        brackets::{BracketConfig, BracketMatch},
        html::{
            HtmlCustomDocument, HtmlDocument, HtmlNestedAttribute, HtmlNestedLanguage,
            HtmlNestedParser, HtmlNesting, HtmlOptions,
        },
    },
};
use serde_json::{Value, json};

#[derive(Debug)]
struct Plain;
impl HtmlCustomDocument for Plain {}

fn parse_custom(source: &str) -> HtmlDocument {
    let parser = HtmlNestedParser {
        name: "plain",
        parse: &|_| Box::new(Plain),
    };
    HtmlDocument::parse_with_nesting(
        Text::new(source),
        HtmlOptions::default(),
        HtmlNesting {
            nested_languages: &[
                HtmlNestedLanguage {
                    tag: "x-body",
                    attrs: None,
                    parser,
                },
                HtmlNestedLanguage {
                    tag: "SCRIPT",
                    attrs: None,
                    parser,
                },
            ],
            nested_attributes: &[HtmlNestedAttribute {
                name: "data-code",
                tag_name: None,
                parser,
            }],
        },
    )
}

const CUSTOM: BracketConfig<'static> = BracketConfig {
    brackets: "«»",
    max_scan_distance: 1,
    after_cursor: true,
};

#[test]
fn configured_html_matches_pinned_plugin() {
    for line in include_str!("fixtures/html_bracket_options.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let head = row["head"].as_u64().unwrap() as usize;
        let doc = if name.starts_with("custom-") {
            parse_custom(source)
        } else {
            HtmlDocument::parse(Text::new(source))
        };
        let config = BracketConfig {
            brackets: row["options"]["brackets"].as_str().unwrap_or_default(),
            max_scan_distance: row["options"]["maxScanDistance"]
                .as_u64()
                .unwrap_or_default() as usize,
            after_cursor: row["options"]["afterCursor"].as_bool().unwrap_or(true),
        };
        let hit = doc.bracket_near_with_config(head, config);
        let mut marks: Vec<_> = hit
            .iter()
            .flat_map(|hit| {
                std::iter::once(&hit.start)
                    .chain(hit.end.iter())
                    .map(|range| (range.start, range.end, if hit.matched { "M" } else { "X" }))
            })
            .collect();
        marks.sort_unstable();
        assert_eq!(json!(marks), row["marks"], "{name} at byte {head}");
        for pos in 0..=source.len() + 1 {
            let hit = doc.bracket_near_with_config(pos, config);
            if !source.is_char_boundary(pos)
                || pos > 0 && source.as_bytes().get(pos - 1..pos + 1) == Some(b"\r\n")
            {
                assert!(hit.is_none(), "{name} at byte {pos}");
            } else if let Some(hit) = hit {
                assert!(source.get(hit.start).is_some(), "{name} at byte {pos}");
                assert!(
                    hit.end.is_none_or(|range| source.get(range).is_some()),
                    "{name} at byte {pos}"
                );
            }
        }
        assert_eq!(doc.text().as_str(), source);
    }
}

fn unmatched(start: Range<usize>) -> Option<BracketMatch> {
    Some(BracketMatch {
        start,
        end: None,
        matched: false,
    })
}

#[test]
fn native_plain_scans_do_not_pair_across_mounts() {
    for (source, custom) in [
        ("<script>/*«*/</script><script>/*»*/</script>", false),
        (r#"<p onclick="/*«*/" onblur="/*»*/"></p>"#, false),
        ("<x-body>«</x-body>»", true),
        (r#"<p data-code="«">»</p>"#, true),
    ] {
        let doc = if custom {
            parse_custom(source)
        } else {
            HtmlDocument::parse(Text::new(source))
        };
        let start = source.find('«').unwrap();
        let end = source.find('»').unwrap();
        assert_eq!(
            doc.bracket_near_with_config(start + 2, CUSTOM),
            unmatched(start..start + 2),
            "{source}"
        );
        assert_eq!(
            doc.bracket_near_with_config(end + 2, CUSTOM),
            unmatched(end..end + 2),
            "{source}"
        );
    }
}

#[test]
fn native_attribute_start_is_owned_by_the_forward_probe() {
    let doc = HtmlDocument::parse(Text::new(r#"<p onclick="(x)"></p>"#));
    assert_eq!(
        doc.bracket_near_with_config(12, CUSTOM),
        Some(BracketMatch {
            start: 12..13,
            end: Some(14..15),
            matched: true
        })
    );
    assert_eq!(
        doc.bracket_near_with_config(
            12,
            BracketConfig {
                after_cursor: false,
                ..CUSTOM
            }
        ),
        None
    );
}

#[test]
fn no_partner_at_scope_end_is_unmatched_not_cutoff() {
    for (source, custom) in [
        ("<script>/*(😀\r\n</script>", false),
        ("<x-body>(😀\r\n</x-body>", true),
    ] {
        let doc = if custom {
            parse_custom(source)
        } else {
            HtmlDocument::parse(Text::new(source))
        };
        let at = source.find('(').unwrap();
        assert_eq!(
            doc.bracket_near_with_config(
                at + 1,
                BracketConfig {
                    max_scan_distance: 3,
                    ..BracketConfig::default()
                }
            ),
            unmatched(at..at + 1)
        );
        assert_eq!(
            doc.bracket_near_with_config(
                at + 1,
                BracketConfig {
                    max_scan_distance: 2,
                    ..BracketConfig::default()
                }
            ),
            None
        );
    }
}

#[test]
fn native_supplementary_pairs_use_full_utf8_ranges() {
    let source = "<script>/*🙂x🙃*/</script>";
    let doc = HtmlDocument::parse(Text::new(source));
    let config = BracketConfig {
        brackets: "🙂🙃",
        ..BracketConfig::default()
    };
    assert_eq!(
        doc.bracket_near_with_config(10, config),
        Some(BracketMatch {
            start: 10..14,
            end: Some(15..19),
            matched: true
        })
    );
    for at in [11, 12, 13, 16, 17, 18] {
        assert_eq!(doc.bracket_near_with_config(at, config), None);
    }
}

#[test]
fn native_host_tag_does_not_pair_to_a_handle_inside_an_overlay() {
    let doc = parse_custom("<SCRIPT>content</script>");
    assert_eq!(doc.bracket_near_with_config(3, CUSTOM), unmatched(1..7));
    assert_eq!(
        doc.bracket_near(3),
        Some(BracketMatch {
            start: 1..7,
            end: Some(17..23),
            matched: true
        })
    );
}
