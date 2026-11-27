use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::CssDocument,
        less::LessDocument,
        lezer::ReuseStats,
        sass::{SassDialect, SassDocument},
    },
};
use serde_json::{Value, json};

trait StyleDocument: Sized {
    fn text(&self) -> &Text;
    fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError>;
    fn reuse_stats(&self) -> ReuseStats;
    fn syntax(&self) -> Value;
    fn line_services(&self) -> Value;
}

macro_rules! style_document {
    ($doc:ty, $node:ident => $name:expr) => {
        impl StyleDocument for $doc {
            fn text(&self) -> &Text {
                self.text()
            }

            fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
                self.reparse(changes)
            }

            fn reuse_stats(&self) -> ReuseStats {
                self.reuse_stats()
            }

            fn syntax(&self) -> Value {
                let source = self.text().as_str();
                let nodes: Vec<_> = self.nodes().map(|$node| {
                    let range = $node.range();
                    assert!(source.get(range.clone()).is_some());
                    let depth = std::iter::successors($node.parent(), |node| node.parent()).count();
                    json!([$name, range.start, range.end, depth])
                }).collect();
                let highlights: Vec<_> = self.highlights(0..source.len()).unwrap().map(|span| {
                    assert!(source.get(span.range.clone()).is_some());
                    let tags: Vec<_> = span.tags.iter().map(|tag| {
                        let mut name = tag.name().to_owned();
                        name.push_str(match tag.modifiers() {
                            [] => "",
                            [TagModifier::Constant] => ".constant",
                            [TagModifier::Special] => ".special",
                            other => panic!("unexpected tag modifiers: {other:?}"),
                        });
                        name
                    }).collect();
                    json!([span.range.start, span.range.end, tags.join(" ")])
                }).collect();
                json!({"nodes": nodes, "highlights": highlights})
            }

            fn line_services(&self) -> Value {
                (1..=self.text().line_count()).map(|number| {
                    let line = self.text().line(number).unwrap().range;
                    let indent = self.indentation(line.start, StreamSettings::default(), IndentOptions::default());
                    let fold = self.foldable(line.start, line.end).map(|range| [range.start, range.end]);
                    json!([indent, fold])
                }).collect()
            }
        }
    };
}

style_document!(CssDocument, node => node.kind().name());
style_document!(LessDocument, node => node.kind().name());
style_document!(SassDocument, node => node.name());

fn assert_fresh<D: StyleDocument>(doc: &D, parse: impl Fn(Text) -> D, case: &str) {
    let fresh = parse(doc.text().clone());
    assert_eq!(doc.syntax(), fresh.syntax(), "syntax: {case}");
    assert_eq!(
        doc.line_services(),
        fresh.line_services(),
        "editor services: {case}"
    );
    assert_eq!(fresh.reuse_stats(), ReuseStats::default(), "fresh: {case}");
}

fn reuse_before<D: StyleDocument>(doc: &D, old_prefix: &str, case: &str) {
    let stats = doc.reuse_stats();
    let limit = old_prefix.len() - old_prefix.matches("\r\n").count();
    assert!(
        stats.nodes_reused > 0 && (1..=limit).contains(&stats.resumed_at),
        "{case}: {stats:?}, limit {limit}"
    );
}

fn edit(from: usize, to: usize, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(from..to, insert)).unwrap()
}

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn exercise<D: StyleDocument>(case: &Value, parse: impl Fn(Text) -> D) {
    let mode = case["mode"].as_str().unwrap();
    let before = case["doc"].as_str().unwrap();
    let old = parse(Text::new(before));
    assert_eq!(old.syntax(), case["before"], "upstream before: {mode}");
    assert_eq!(old.reuse_stats(), ReuseStats::default());

    let edits = case["edits"].as_array().unwrap();
    let changes = ChangeSet::new(
        edits
            .iter()
            .map(|entry| {
                Change::new(
                    offset(&entry[0])..offset(&entry[1]),
                    entry[2].as_str().unwrap(),
                )
            })
            .collect(),
    )
    .unwrap();
    let new = old.reparse(&changes).unwrap();
    assert_eq!(old.text().as_str(), before);
    assert_eq!(new.text().as_str(), case["newDoc"].as_str().unwrap());
    assert_eq!(new.syntax(), case["after"], "upstream after: {mode}");
    assert_fresh(&new, &parse, mode);
    reuse_before(&new, &before[..offset(&edits[0][0])], mode);
    if mode == "indented" {
        // The resumed stack is already inside the outer indentation block.
        let logical = before.replace("\r\n", "\n");
        assert!(new.reuse_stats().resumed_at > logical.find("  $width").unwrap());
    }

    // Edits are atomic and checked in original UTF-8 bytes.
    let inside_scalar = before
        .char_indices()
        .find(|&(_, ch)| !ch.is_ascii())
        .unwrap()
        .0
        + 1;
    assert!(matches!(
        old.reparse(&edit(inside_scalar, inside_scalar, "x")),
        Err(EditError::InvalidRange)
    ));
    assert!(matches!(
        old.reparse(&edit(before.len(), before.len() + 1, "")),
        Err(EditError::InvalidRange)
    ));
    assert_eq!(old.text().as_str(), before);

    // A start-of-file insertion leaves no prefix, but still records new checkpoints.
    let shifted = new.reparse(&edit(0, 0, "/* 😀 */\r\n")).unwrap();
    assert_eq!(
        shifted.reuse_stats(),
        ReuseStats::default(),
        "early edit: {mode}"
    );
    assert_fresh(&shifted, &parse, mode);
    let unchanged = shifted.reparse(&ChangeSet::default()).unwrap();
    reuse_before(&unchanged, shifted.text().as_str(), mode);
    assert_fresh(&unchanged, &parse, mode);

    // An edit at a CRLF boundary may change how the input is normalized.
    let source = new.text().as_str();
    let split_at = source.rfind("\r\n").unwrap() + 1;
    let split = new.reparse(&edit(split_at, split_at, " ")).unwrap();
    assert_fresh(&split, &parse, mode);
    let joined = split.reparse(&edit(split_at, split_at + 1, "")).unwrap();
    assert_eq!(joined.text().as_str(), source);
    assert_eq!(joined.syntax(), new.syntax(), "CRLF join: {mode}");
    let prefix = &source[..split_at - 1];
    reuse_before(&split, prefix, mode);
    reuse_before(&joined, prefix, mode);

    // A failed strict parse must not pass checkpoints on to recovery or repair.
    let end = source.len();
    let break_it = edit(end, end, "\"unterminated");
    let broken = new.reparse(&break_it).unwrap();
    assert_eq!(
        broken.reuse_stats(),
        ReuseStats::default(),
        "recovery: {mode}"
    );
    assert!(
        broken.syntax()["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node[0] == "⚠"),
        "recovery: {mode}"
    );
    assert_fresh(&broken, &parse, mode);
    let repaired = broken
        .reparse(&break_it.inverted(new.text()).unwrap())
        .unwrap();
    assert_eq!(
        repaired.reuse_stats(),
        ReuseStats::default(),
        "repaired from recovery: {mode}"
    );
    assert_eq!(repaired.syntax(), new.syntax(), "repaired: {mode}");
    let replay = repaired.reparse(&ChangeSet::default()).unwrap();
    reuse_before(&replay, source, mode);
    assert_fresh(&replay, &parse, mode);
}

#[test]
fn edited_styles_match_fresh_parses_and_pinned_oracles() {
    // Captured in external scratch from the npm tarballs verified against
    // docs/codemirror-upstream.json. Fresh and TreeFragment reparse matched in
    // JS. The fixture converts logical UTF-16 offsets to original UTF-8 bytes.
    for line in include_str!("fixtures/style_incremental.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        match case["mode"].as_str().unwrap() {
            "css" => exercise(&case, CssDocument::parse),
            "styles" => exercise(&case, CssDocument::parse_styles),
            "less" => exercise(&case, LessDocument::parse),
            "scss" => exercise(&case, |text| SassDocument::parse(text, SassDialect::Scss)),
            "indented" => exercise(&case, |text| {
                SassDocument::parse(text, SassDialect::Indented)
            }),
            mode => panic!("unknown fixture mode: {mode}"),
        }
    }
}

#[test]
fn css_glr_fallback_does_not_keep_checkpoints() {
    use gpui_editor_codemirror::language::css::CssNodeKind;

    let source = "/* 😀 */\r\n.a { --color: red; --padding: 12px; --margin: 23px; --display: block; }\r\n.b { --width: 3px; }";
    let doc = CssDocument::parse(Text::new(source));
    let at = source.rfind("--width").unwrap();
    let changes = edit(at, at + 2, "");
    let glr = doc.reparse(&changes).unwrap();
    assert_fresh(&glr, CssDocument::parse, "late GLR fallback");
    assert!(glr.nodes().all(|node| node.kind() != CssNodeKind::Error));
    assert_eq!(glr.reuse_stats(), ReuseStats::default());
    let restored = glr.reparse(&changes.inverted(doc.text()).unwrap()).unwrap();
    assert_eq!(restored.syntax(), doc.syntax());
    assert_eq!(restored.reuse_stats(), ReuseStats::default());
    let reused = restored.reparse(&edit(at, at + 7, "--height")).unwrap();
    reuse_before(&reused, &source[..at], "fresh checkpoints after GLR");
    assert_fresh(&reused, CssDocument::parse, "strict parse after GLR");
}

#[test]
fn new_css_color_spellings_start_a_new_checkpoint_history() {
    let source = "/* prefix with 😀 */\r\n.a { --color: #fff; --padding: 12px; --margin: 23px; --display: block; }\r\n.b { --color: #fff; }";
    let doc = CssDocument::parse(Text::new(source));
    let at = source.rfind("#fff").unwrap();
    let changed = doc.reparse(&edit(at, at + 4, "#B123")).unwrap();
    assert_eq!(changed.reuse_stats(), ReuseStats::default());
    assert_fresh(&changed, CssDocument::parse, "new hex color");
    assert!(
        changed
            .highlights(0..changed.text().len())
            .unwrap()
            .any(|span| {
                changed.text().slice(span.range) == Some("#B123") && span.tags[0].name() == "color"
            })
    );
    let restored = changed.reparse(&edit(at, at + 5, "#fff")).unwrap();
    assert_eq!(restored.text().as_str(), source);
    reuse_before(&restored, &source[..at], "existing hex color");
    assert_eq!(restored.syntax(), doc.syntax());
}
