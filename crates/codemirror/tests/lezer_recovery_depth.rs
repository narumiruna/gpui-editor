//! Deep stack cuts from the pinned WAST grammar and Lezer runtime.

use std::fmt::Write as _;

use gpui_editor_codemirror::{
    Text,
    language::wast::{WastDocument, WastNodeKind},
};
use serde_json::Value;

#[test]
fn cuts_deep_wast_stacks_like_pinned_lezer() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/lezer_recovery_depth.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let count = |key| usize::try_from(case[key].as_u64().unwrap()).unwrap();
        let source = "(".repeat(count("opens"))
            + &")".repeat(count("closes"))
            + case["tail"].as_str().unwrap();
        let doc = WastDocument::parse(Text::new(&source));
        assert_eq!(doc.root().range(), 0..source.len());
        let mut pending = vec![(0, doc.root())];
        let mut records = 0;
        let mut errors = 0;
        let mut app_end = None;
        let mut first_error = None;
        let mut signature = String::new();
        while let Some((depth, node)) = pending.pop() {
            let range = node.range();
            let kind = node.kind();
            records += 1;
            if kind == WastNodeKind::App && range.start == 1999 {
                app_end = Some(range.end);
            }
            if kind == WastNodeKind::Error {
                errors += 1;
                first_error.get_or_insert([range.start, range.end]);
            }
            write!(
                signature,
                "{depth}:{}:{}:{};",
                kind.name(),
                range.start,
                range.end,
            )
            .unwrap();
            pending.extend(node.children().rev().map(|child| (depth + 1, child)));
        }
        let hash = signature.bytes().fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
        assert_eq!(serde_json::json!(app_end), case["appEnd"], "{case}");
        assert_eq!(serde_json::json!(first_error), case["firstError"], "{case}");
        assert_eq!(records, count("records"), "{case}");
        assert_eq!(errors, count("errors"), "{case}");
        assert_eq!(u64::from(hash), case["hash"].as_u64().unwrap(), "{case}");
    }
}
