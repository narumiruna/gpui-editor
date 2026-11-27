//! Byte-range fixtures from the pinned WAST parser and CodeMirror editor.

use gpui_editor_codemirror::{
    Change, ChangeSet, ConfigurationError, EditorState, Text, Transaction,
    language::{
        lezer::{ParseError, SyntaxTree},
        wast::{NodeProp, WastDocument, WastParseJob, parse_raw},
    },
    language_data::{NativeSyntax, attached_language_snapshot, wast_language},
};
use serde_json::{Value, json};

type Row<'a> = (usize, &'a str, usize, usize);

fn rows<'a>(tree: &'a SyntaxTree, source: &str) -> Vec<Row<'a>> {
    let mut pending = vec![(0, tree.root())];
    let mut rows = Vec::new();
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some(), "{range:?}");
        rows.push((depth, node.name(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    rows
}

fn editor_rows(doc: &WastDocument) -> Vec<Row<'static>> {
    let mut pending = vec![(0, doc.root())];
    let mut rows = Vec::new();
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        rows.push((depth, node.kind().name(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    rows
}

fn fixtures() -> impl Iterator<Item = Value> {
    include_str!("fixtures/wast_direct.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn hash_rows(rows: &[Row<'_>]) -> u32 {
    serde_json::to_vec(rows)
        .unwrap()
        .into_iter()
        .fold(2_166_136_261, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        })
}

#[test]
fn raw_and_editor_trees_match_pins_before_and_after_edits() {
    let mut previous: Option<WastDocument> = None;
    for fixture in fixtures().filter(|row| row["source"].is_string()) {
        let name = fixture["name"].as_str().unwrap();
        let source = fixture["source"].as_str().unwrap();
        let editor = if let Some(change) = fixture["change"].as_array() {
            let edit = Change::new(
                number(&change[0])..number(&change[1]),
                change[2].as_str().unwrap(),
            );
            let batch = ChangeSet::single(edit).unwrap();
            previous.unwrap().reparse(&batch).unwrap()
        } else {
            WastDocument::parse(Text::new(source))
        };
        assert_eq!(editor.text().as_str(), source, "{name}");
        assert_eq!(json!(editor_rows(&editor)), fixture["editor"], "{name}");

        let source = editor.text().as_str();
        let recovered = parse_raw(source, false).unwrap();
        assert_eq!(json!(rows(&recovered, source)), fixture["raw"], "{name}");
        match parse_raw(source, true) {
            Ok(tree) => {
                assert!(fixture["error"].is_null(), "{name}");
                assert_eq!(json!(rows(&tree, source)), fixture["raw"], "{name}");
            }
            Err(ParseError::UnexpectedToken { offset, .. }) => {
                assert_eq!(json!(offset), fixture["error"], "{name}");
            }
            Err(error) => panic!("{name}: {error}"),
        }
        previous = Some(editor);
    }
}

#[test]
fn both_direct_modes_parse_long_and_deep_valid_input() {
    for fixture in fixtures().filter(|row| row["bytes"].is_number()) {
        let name = fixture["name"].as_str().unwrap();
        let source = match name {
            "long" => "(i32.const 0) ;; 🦀\r\n".repeat(4096),
            "deep" => format!("{}\"🦀\r\n尾\"{}", "(".repeat(2048), ")".repeat(2048)),
            _ => panic!("unknown fixture {name}"),
        };
        assert_eq!(source.len(), number(&fixture["bytes"]));
        for strict in [false, true] {
            let tree = parse_raw(&source, strict).unwrap();
            let rows = rows(&tree, &source);
            assert_eq!(rows.len(), number(&fixture["nodes"]), "{name}");
            assert!(rows.iter().all(|row| row.1 != "⚠"), "{name}");
            assert_eq!(
                json!(hash_rows(&rows)),
                fixture["hash"],
                "{name}, strict={strict}"
            );
        }
    }
}

#[test]
fn all_permissive_wast_paths_match_the_pinned_deep_recovery() {
    let marker = NodeProp::new();
    let configured =
        wast_language().with_node_props(&[marker.add(|row| row.is_top().then_some(7))]);
    for fixture in fixtures().filter(|row| row["permissive"].is_object()) {
        let part = fixture["part"].as_str().unwrap();
        let depth = number(&fixture["depth"]);
        let source = part.repeat(depth) + &")".repeat(depth);
        let mut job = WastParseJob::new(Text::new(&source));
        let output = job
            .advance(source.len(), usize::MAX)
            .unwrap()
            .output
            .unwrap();
        assert!(output.complete);
        let expected = &fixture["permissive"];
        let raw = parse_raw(&source, false).unwrap();
        let direct = configured.parse(&source).unwrap();
        assert_eq!(
            direct.root().wast_node_type().unwrap().prop(&marker),
            Some(&7)
        );
        let recovered = rows(&raw, &source);
        assert_eq!(recovered, rows(&direct, &source), "{part:?} × {depth}");
        assert_eq!(recovered, rows(&output.tree, &source), "{part:?} × {depth}");
        let state = EditorState::with_extensions(source.as_str(), configured.extension()).unwrap();
        let attached = attached_language_snapshot(&state).unwrap();
        let NativeSyntax::Wast(doc) = attached.syntax() else {
            panic!("expected WAST");
        };
        assert_eq!(doc.root().prop(&marker), Some(&7));
        assert!(recovered == editor_rows(doc), "{part:?} × {depth}");
        assert_eq!(
            recovered.len(),
            number(&expected["nodes"]),
            "{part:?} × {depth}"
        );
        assert_eq!(
            recovered.iter().filter(|row| row.1 == "⚠").count(),
            number(&expected["errors"]),
            "{part:?} × {depth}"
        );
        assert_eq!(
            json!(hash_rows(&recovered)),
            expected["hash"],
            "{part:?} × {depth}"
        );
    }
}

#[test]
fn synchronous_edits_apply_the_depth_cut_and_reuse_a_strict_prefix_after_repair() {
    let row = fixtures()
        .find(|row| row["name"] == "edit-across-depth-cut")
        .unwrap();
    let prefix = row["prefixPart"]
        .as_str()
        .unwrap()
        .repeat(number(&row["prefixCount"]))
        + row["comment"].as_str().unwrap();
    let body = row["body"].as_str().unwrap();
    let depth = number(&row["depth"]);
    let extra = number(&row["extra"]);
    let source = format!("{prefix}{}{body}{}", "(".repeat(depth), ")".repeat(depth));
    let mut state =
        EditorState::with_extensions(source.as_str(), wast_language().extension()).unwrap();
    let retained = state.clone();
    let snapshot = |state: &EditorState, expected: &Value| {
        let document = attached_language_snapshot(state).unwrap();
        let NativeSyntax::Wast(doc) = document.syntax() else {
            panic!("expected WAST")
        };
        let rows = editor_rows(doc);
        assert_eq!(json!(hash_rows(&rows)), expected["hash"]);
        assert_eq!(
            json!(rows.iter().filter(|row| row.1 == "⚠").count()),
            expected["errors"]
        );
        doc.reuse_stats()
    };
    let insert = |at| {
        Transaction::new(
            ChangeSet::new(vec![
                Change::new(at..at, "(".repeat(extra)),
                Change::new(source.len()..source.len(), ")".repeat(extra)),
            ])
            .unwrap(),
        )
    };
    let at = prefix.len() + depth;
    state.update(insert(at)).unwrap();
    let end = state.text().len();
    let stats = snapshot(&state, &row["cut"]);
    assert_eq!(stats.nodes_reused, 0);
    assert_eq!(stats.resumed_at, 0);
    let repair = ChangeSet::new(vec![
        Change::new(at..at + extra, ""),
        Change::new(end - extra..end, ""),
    ])
    .unwrap();
    state.update(Transaction::new(repair)).unwrap();
    assert_eq!(state.text().as_str(), source);
    assert!(snapshot(&state, &row["before"]).nodes_reused > 100);
    snapshot(&retained, &row["before"]);

    // Upstream can reuse the entire old subtree when the same edits are
    // outside it. Native LR checkpoints cannot skip a nested subtree.
    let mut wrapped = retained;
    wrapped.update(insert(prefix.len())).unwrap();
    assert_ne!(row["wrapsReusedSubtree"], row["cut"]);
    snapshot(&wrapped, &row["cut"]);
}

#[test]
fn raw_recovery_keeps_the_tokenizer_work_limit() {
    let source = "(;".repeat(512);
    assert_eq!(
        parse_raw(&source, false).unwrap_err(),
        ParseError::LimitExceeded { offset: 845 }
    );
}

#[test]
fn in_budget_lookahead_matches_raw_and_projected_pinned_ranges() {
    for row in fixtures().filter(|row| row["lookahead"].is_object()) {
        let name = row["name"].as_str().unwrap();
        let source = row["part"].as_str().unwrap().repeat(number(&row["repeat"]));
        let raw = parse_raw(&source, false).unwrap();
        let text = Text::new(source.as_str());
        let mut job = WastParseJob::new(text.clone());
        let output = job.advance(text.len(), usize::MAX).unwrap().output.unwrap();
        assert!(output.complete);
        let raw = rows(&raw, &source);
        assert_eq!(raw, rows(&output.tree, &source), "{name}");
        assert!(raw == editor_rows(&WastDocument::parse(text)), "{name}");
        assert_eq!(json!(raw.len()), row["lookahead"]["nodes"], "{name}");
        assert_eq!(json!(hash_rows(&raw)), row["lookahead"]["hash"], "{name}");
    }
}

#[test]
fn strict_wast_stack_cut_matches_raw_and_editor_pins() {
    let strict = wast_language().configure(Some(true), None);
    for fixture in fixtures().filter(|row| row["direct"].is_object()) {
        let name = fixture["name"].as_str().unwrap();
        let depth = number(&fixture["depth"]);
        let source = format!(
            "{}{}{}{}",
            fixture["prefix"].as_str().unwrap(),
            fixture["part"].as_str().unwrap().repeat(depth),
            fixture["body"].as_str().unwrap(),
            ")".repeat(number(&fixture["closes"])),
        );
        let check = |mode: &str, result: Result<Vec<Row<'_>>, ParseError>| {
            let actual = match result {
                Ok(rows) => json!({
                    "hash": hash_rows(&rows),
                    "errors": rows.iter().filter(|row| row.1 == "⚠").count(),
                }),
                Err(ParseError::UnexpectedToken { offset, .. }) => json!({"offset": offset}),
                Err(error) => panic!("{name} ({mode}): {error:?}"),
            };
            assert_eq!(actual, fixture[mode], "{name} ({mode})");
        };
        let direct = parse_raw(&source, true);
        check(
            "direct",
            direct
                .as_ref()
                .map(|tree| rows(tree, &source))
                .map_err(Clone::clone),
        );
        let editor = EditorState::with_extensions(source.as_str(), strict.extension())
            .map(|state| {
                let document = attached_language_snapshot(&state).unwrap();
                let NativeSyntax::Wast(doc) = document.syntax() else {
                    panic!("{name}: expected WAST");
                };
                editor_rows(doc)
            })
            .map_err(|error| match error {
                ConfigurationError::StrictSyntax(error) => error,
                _ => panic!("{name}: {error:?}"),
            });
        check("editor", editor);
        if name == "cut-on-opening" {
            let raw = parse_raw(&source, false).unwrap();
            let recovered = rows(&raw, &source);
            assert!(recovered.iter().any(|row| row.1 == "⚠"));
            let doc = WastDocument::parse(Text::new(&source));
            assert_eq!(editor_rows(&doc), recovered);
        }
    }
}
