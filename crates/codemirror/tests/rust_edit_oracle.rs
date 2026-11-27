use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::{IndentOptions, StreamSettings, TagModifier, lezer::ReuseStats, rust::RustDocument},
};
use serde_json::{Value, json};

fn snapshot(doc: &RustDocument) -> [Value; 4] {
    let nodes: Vec<_> = doc
        .nodes()
        .map(|node| {
            let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
            let range = node.range();
            json!([depth, node.kind().name(), range.start, range.end])
        })
        .collect();
    let spans: Vec<_> = doc
        .highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let names = span
                .tags
                .iter()
                .map(|tag| {
                    let mut name = tag.name().to_owned();
                    for modifier in tag.modifiers() {
                        name.push_str(match modifier {
                            TagModifier::Definition => ".definition",
                            TagModifier::Function => ".function",
                            TagModifier::Special => ".special",
                            tag => panic!("unexpected tag modifier: {tag:?}"),
                        });
                    }
                    name
                })
                .collect::<Vec<_>>()
                .join(" ");
            json!([span.range.start, span.range.end, names])
        })
        .collect();
    let settings = StreamSettings {
        indent_unit: 4,
        tab_size: 4.try_into().unwrap(),
    };
    let mut folds = Vec::new();
    let mut indents = Vec::new();
    for n in 1..=doc.syntax_lines().line_count() {
        let line = doc.syntax_lines().line(n).unwrap().range;
        let fold = doc
            .foldable(line.start, line.end)
            .map(|range| [range.start, range.end]);
        folds.push(json!([line.start, line.end, fold]));
        for pos in [line.start, line.end]
            .into_iter()
            .take(if line.is_empty() { 1 } else { 2 })
        {
            indents.push(json!([
                pos,
                doc.indentation(pos, settings, IndentOptions::default()),
                doc.indentation(
                    pos,
                    settings,
                    IndentOptions {
                        simulate_break: Some(pos),
                        simulate_double_break: true,
                        ..IndentOptions::default()
                    }
                )
            ]));
        }
    }
    [json!(nodes), json!(spans), json!(folds), json!(indents)]
}

fn edit(from: usize, to: usize, insert: &str) -> ChangeSet {
    ChangeSet::new(vec![Change::new(from..to, insert)]).unwrap()
}

#[test]
fn pinned_rust_edit_oracle() {
    let mut previous = None::<RustDocument>;
    for (i, line) in include_str!("fixtures/rust_edit_oracle.jsonl")
        .lines()
        .enumerate()
    {
        let row: Value = serde_json::from_str(line).unwrap();
        let id = row["id"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let fresh = RustDocument::parse(Text::new(source));
        let phase = i % 3;
        let doc = if let Some(change) = row["edit"].as_array() {
            let from = change[0].as_u64().unwrap() as usize;
            let to = change[1].as_u64().unwrap() as usize;
            previous
                .as_ref()
                .unwrap()
                .reparse(&edit(from, to, change[2].as_str().unwrap()))
                .unwrap()
        } else {
            RustDocument::parse(Text::new(source))
        };
        assert_eq!(doc.text().as_str(), source, "{id}");
        let split_crlf = phase == 1 && id == "split-crlf";
        assert_eq!(doc.is_recovered(), phase == 1 && !split_crlf, "{id}");
        assert_eq!(doc.is_recovered(), fresh.is_recovered(), "{id}");
        if doc.is_recovered() || id == "ambiguous-if" {
            assert_eq!(doc.reuse_stats(), ReuseStats::default(), "{id}");
        }
        let actual = snapshot(&doc);
        assert_eq!(actual, snapshot(&fresh), "{id} phase {phase}");
        let [nodes, spans, folds, indents] = actual;
        assert_eq!(nodes, row["nodes"], "{id} phase {phase}: tree");
        assert_eq!(spans, row["spans"], "{id} phase {phase}: styles");
        assert_eq!(folds, row["folds"], "{id} phase {phase}: folds");
        assert_eq!(indents, row["indents"], "{id} phase {phase}: indentation");
        previous = Some(doc);
    }
}

#[test]
fn unfinished_comment_does_not_fold_into_the_middle_of_a_scalar() {
    // The pinned fold for this malformed input ends at the low surrogate.
    let source = "/* open\n  😀x";
    let document = RustDocument::parse(Text::new(source));
    assert!(document.is_recovered());
    assert_eq!(document.foldable(0, "/* open".len()), None);
}

#[test]
fn repaired_document_uses_only_strict_prefix_checkpoints() {
    let prefix = "// 🦀\r\nconst VALUE: i32 = 2;\r\nconst OTHER: i32 = 3;\r\n".repeat(3);
    let source = prefix + "const LAST: i32 = 1\r\n";
    let at = source.rfind("\r\n").unwrap();
    let fresh_broken = RustDocument::parse(Text::new(&source));
    assert!(fresh_broken.is_recovered());
    assert_eq!(fresh_broken.reuse_stats(), ReuseStats::default());

    let fixed = fresh_broken.reparse(&edit(at, at, ";")).unwrap();
    assert!(!fixed.is_recovered());
    assert!(fixed.reuse_stats().nodes_reused > 0);
    let resumed = fixed.reuse_stats().resumed_at;
    assert!(resumed > 64 && resumed <= at && source.is_char_boundary(resumed));
    let broken_again = fixed.reparse(&edit(at, at + 1, "")).unwrap();
    assert_eq!(broken_again.reuse_stats(), ReuseStats::default());
    assert_eq!(snapshot(&broken_again), snapshot(&fresh_broken));
    let fixed_again = broken_again.reparse(&edit(at, at, ";")).unwrap();
    assert!(fixed_again.reuse_stats().nodes_reused > 0);
    assert_eq!(snapshot(&fixed_again), snapshot(&fixed));

    let fresh = RustDocument::parse(fixed.text().clone());
    assert_eq!(snapshot(&fixed), snapshot(&fresh));
    let astral = source.find('🦀').unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(astral..astral + '🦀'.len_utf8(), "λ"),
        Change::new(at..at, ";"),
    ])
    .unwrap();
    let changed_prefix = fresh_broken.reparse(&changes).unwrap();
    assert_eq!(changed_prefix.reuse_stats(), ReuseStats::default());
    assert_eq!(
        snapshot(&changed_prefix),
        snapshot(&RustDocument::parse(Text::new(
            fixed.text().as_str().replacen('🦀', "λ", 1)
        )))
    );
    for invalid in [astral + 1, source.len() + 1] {
        assert_eq!(
            fresh_broken
                .reparse(&edit(invalid, invalid, "x"))
                .unwrap_err(),
            EditError::InvalidRange
        );
    }
    assert_eq!(fresh_broken.text().as_str(), source);
}

#[test]
fn over_recovery_cap_uses_local_tree_without_a_seed() {
    let prefix = "const ONE: i32 = 1;\r\n".repeat(5);
    let tail = "\r\nconst LAST: i32 = 1\r\n";
    let padding = 16 * 1024 - prefix.len() - "//".len() - tail.len();
    for extra in [0, 1] {
        let source = format!("{prefix}//{}{tail}", " ".repeat(padding + extra));
        let broken = RustDocument::parse(Text::new(&source));
        assert!(broken.is_recovered());
        assert_eq!(broken.reuse_stats(), ReuseStats::default());
        assert_eq!(
            broken.nodes().any(|node| node.kind().name() == "ConstItem"),
            extra == 0
        );
        let at = source.rfind("\r\n").unwrap();
        let fixed = broken.reparse(&edit(at, at, ";")).unwrap();
        assert!(!fixed.is_recovered());
        assert_eq!(fixed.reuse_stats().nodes_reused > 0, extra == 0);
        assert_eq!(
            snapshot(&fixed),
            snapshot(&RustDocument::parse(fixed.text().clone()))
        );
    }
}
