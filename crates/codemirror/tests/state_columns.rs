use std::num::NonZeroUsize;

use gpui_editor_codemirror::{Assoc, count_column, find_cluster_break, find_column};
use serde_json::Value;

#[test]
fn text_columns_match_the_pinned_state_in_byte_coordinates() {
    for line in include_str!("fixtures/state_columns.jsonl").lines().skip(1) {
        let mut case: Value = serde_json::from_str(line).unwrap();
        let tab_size: NonZeroUsize = serde_json::from_value(case["tabSize"].take()).unwrap();
        let count: Vec<(usize, usize)> = serde_json::from_value(case["count"].take()).unwrap();
        let find: Vec<(usize, Option<usize>, Option<usize>)> =
            serde_json::from_value(case["find"].take()).unwrap();
        let breaks: Vec<(usize, bool, usize, usize)> =
            serde_json::from_value(case["breaks"].take()).unwrap();
        let text = case["text"].as_str().unwrap();
        for (offset, include_extending, before, after) in breaks {
            assert_eq!(
                find_cluster_break(text, offset, Assoc::Before, include_extending),
                Some(before),
                "{text:?} before {offset}, include extenders {include_extending}"
            );
            assert_eq!(
                find_cluster_break(text, offset, Assoc::After, include_extending),
                Some(after),
                "{text:?} after {offset}, include extenders {include_extending}"
            );
        }
        for (offset, expected) in count {
            assert_eq!(
                count_column(text, tab_size, offset),
                Some(expected),
                "{text:?} at {offset}, tab {tab_size}"
            );
        }
        for (column, relaxed, strict) in find {
            assert_eq!(
                find_column(text, column, tab_size, false),
                relaxed,
                "{text:?}, column {column}, tab {tab_size}"
            );
            assert_eq!(
                find_column(text, column, tab_size, true),
                strict,
                "{text:?}, column {column}, tab {tab_size}"
            );
        }
    }
}

#[test]
fn column_scan_pairs_long_regional_indicator_runs() {
    let tab_size = NonZeroUsize::new(4).unwrap();
    let mut text = "🇨".repeat(12_001);
    text.push(' ');
    text.push_str("🇩🇩🇩");
    assert_eq!(count_column(&text, tab_size, text.len()), Some(6_004));
    assert_eq!(find_column(&text, 4_000, tab_size, true), Some(32_000));
    assert_eq!(find_column(&text, 6_003, tab_size, true), Some(48_013));
}

#[test]
fn column_count_clamps_past_eof_and_rejects_positions_inside_a_scalar() {
    let tab_size = NonZeroUsize::new(4).unwrap();
    assert_eq!(count_column("😀\t", tab_size, usize::MAX), Some(4));
    assert_eq!(count_column("😀\t", tab_size, 1), None);
    assert_eq!(find_cluster_break("😀\t", 1, Assoc::Before, true), None);
}
