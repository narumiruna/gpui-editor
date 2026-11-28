use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{cpp::CppDocument, go::GoDocument},
};
use serde_json::Value;

fn byte_range(wire: &Value) -> Option<Range<usize>> {
    let at = |index: usize| usize::try_from(wire[index].as_u64()?).ok();
    Some(at(0)?..at(1)?)
}

fn check(
    source: &str,
    lines: &Text,
    comments: impl Iterator<Item = (Range<usize>, Option<Range<usize>>)>,
    foldable: impl Fn(usize, usize) -> Option<Range<usize>>,
    expected: &Value,
) {
    let expected_comments = expected["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (byte_range(&row[0]).unwrap(), byte_range(&row[1])));
    assert!(
        comments.eq(expected_comments),
        "comment fold for {source:?}: {expected}"
    );
    let rows = expected["lines"].as_array().unwrap();
    assert_eq!(lines.line_count(), rows.len(), "{source:?}");
    for (index, row) in rows.iter().enumerate() {
        let line = lines.line(index + 1).unwrap();
        assert_eq!(Some(line.range.clone()), byte_range(&row[0]), "{source:?}");
        assert_eq!(
            foldable(line.range.start, line.range.end),
            byte_range(&row[1]),
            "{source:?} line {}",
            index + 1
        );
    }
}

#[test]
fn comment_folds_match_pinned_editor_units_at_valid_byte_boundaries() {
    macro_rules! replay {
        ($kind:ty, $source:expr, $expected:expr) => {{
            let verify = |doc: &$kind, expected: &Value| {
                check(
                    doc.text().as_str(),
                    doc.syntax_lines(),
                    doc.nodes()
                        .filter(|node| node.kind().name() == "BlockComment")
                        .map(|node| (node.range(), node.fold_range())),
                    |from, to| doc.foldable(from, to),
                    expected,
                );
            };
            let source = $source;
            let doc = <$kind>::parse(Text::new(source));
            verify(&doc, &$expected);
            if let Some(expected) = $expected.get("deleted") {
                let end = source.len() - 2;
                let edited = doc
                    .reparse(&ChangeSet::single(Change::new(end..source.len(), "")).unwrap())
                    .unwrap();
                assert_eq!(edited.text().as_str(), &source[..end]);
                verify(&edited, expected);
                let repaired = edited
                    .reparse(&ChangeSet::single(Change::new(end..end, "*/")).unwrap())
                    .unwrap();
                assert_eq!(repaired.text().as_str(), source);
                verify(&repaired, &$expected["repaired"]);
            }
        }};
    }

    for row in include_str!("fixtures/c_go_comment_folds.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let source = case["source"].as_str().unwrap();
        match case["language"].as_str().unwrap() {
            "Cpp" => replay!(CppDocument, source, case),
            "Go" => replay!(GoDocument, source, case),
            name => panic!("unknown language {name}"),
        }
    }
}
