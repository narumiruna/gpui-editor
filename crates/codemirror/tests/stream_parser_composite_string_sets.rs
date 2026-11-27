//! Frozen `StringStream.match` results from `@codemirror/streamparser` 6.0.0
//! on Node v26.10.0. The npm archive matches the pinned SHA-512 inventory.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::stream_parser::utf16::{
    RegExp, RegExpError, RegExpMatch, StringStream,
};
use serde_json::{Value, json};

#[test]
fn composite_string_sets_match_the_pinned_streamparser() {
    let fixture = include_str!("fixtures/stream_parser_composite_string_sets.jsonl");
    for row in fixture.lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let flags = case["flags"].as_str().unwrap();
        let regex = match case["source"].as_str() {
            Some(source) => RegExp::new(source, flags),
            None => {
                let source: Vec<u16> = serde_json::from_value(case["source"].clone()).unwrap();
                RegExp::new_utf16(&source, flags)
            }
        };
        if case["invalid"] == true {
            assert!(matches!(regex, Err(RegExpError::Pattern(_))), "{row}");
            continue;
        }
        let mut regex = regex.unwrap_or_else(|error| panic!("{error}: {row}"));
        regex.last_index = case["lastIndex"].as_i64().unwrap() as isize;
        let line: Vec<u16> = serde_json::from_value(case["text"].clone()).unwrap();
        let mut stream = StringStream::new(&line, NonZeroUsize::new(4).unwrap(), 2);
        stream.pos = case["pos"].as_i64().unwrap() as isize;
        let result = match stream.match_regex(&mut regex, case["consume"].as_bool().unwrap()) {
            Some(RegExpMatch::Single(matched)) => json!({
                "captures": matched.captures().collect::<Vec<_>>(),
                "index": matched.index(),
                "input": matched.input(),
                "names": matched.named_captures().collect::<Vec<_>>(),
                "indices": matched.indices().unwrap().map(range_pair).collect::<Vec<_>>(),
                "namedIndices": matched.named_indices().into_iter().flatten().map(|(name, range)| (name, range_pair(range))).collect::<Vec<_>>(),
            }),
            Some(RegExpMatch::Global(matches)) => json!(matches),
            None => Value::Null,
        };
        assert_eq!(result, case["result"], "{row}");
        assert_eq!(json!(stream.pos), case["end"], "{row}");
        assert_eq!(json!(regex.last_index), case["lastAfter"], "{row}");
    }
}

fn range_pair(range: Option<std::ops::Range<usize>>) -> Option<[usize; 2]> {
    range.map(|range| [range.start, range.end])
}
