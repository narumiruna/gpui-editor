use std::ptr;

use gpui_editor_codemirror::{
    Text,
    language::{
        json::{JsonParseJob, JsonParseOutput},
        lezer::{NodeProp, SyntaxTree},
    },
    language_data::json_language,
};
use serde_json::{Value, json};

fn records<'a>(tree: &'a SyntaxTree, raw: &str) -> Vec<(usize, u16, &'a str, usize, usize)> {
    let mut rows = Vec::new();
    let mut pending = vec![(0, tree.root())];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(raw.get(range.clone()).is_some(), "{range:?} in {raw:?}");
        rows.push((depth, node.term(), node.name(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    rows
}

fn drive(job: &mut JsonParseJob, cutoff: usize, rounds: usize) -> JsonParseOutput {
    for _ in 0..10_000 {
        let step = job.advance(cutoff, rounds).unwrap();
        assert!(step.rounds <= rounds);
        if let Some(output) = step.output {
            assert!(step.output_work > 0);
            assert_eq!(output.parsed_to, cutoff);
            return output;
        }
        assert_eq!(step.rounds, rounds);
        assert_eq!(step.output_work, 0);
    }
    panic!("no result at {cutoff}");
}

#[test]
fn cold_and_continued_jobs_match_pinned_direct_and_editor_prefixes() {
    fn assert_send<T: Send>() {}
    assert_send::<JsonParseJob>();
    for line in include_str!("fixtures/json_resume.jsonl").lines() {
        let fixture: Value = serde_json::from_str(line).unwrap();
        let name = fixture["name"].as_str().unwrap();
        let raw = fixture["raw"].as_str().unwrap();
        let new_job = if fixture["editor"] == true {
            JsonParseJob::new
        } else {
            JsonParseJob::new_raw
        };
        let mut job = new_job(Text::new(raw));
        let mut held = Vec::new();
        for expected in fixture["stages"].as_array().unwrap() {
            let cutoff = expected["stop"].as_u64().unwrap() as usize;
            let cold = drive(&mut new_job(Text::new(raw)), cutoff, 3);
            let output = drive(&mut job, cutoff, 1);
            assert_eq!(output.complete, cutoff == raw.len(), "{name} at {cutoff}");
            assert_eq!(output.complete, cold.complete);
            let rows = json!(records(&output.tree, raw));
            let cold = json!(records(&cold.tree, raw));
            assert_eq!(cold, expected["rows"], "cold {name} at {cutoff}");
            assert_eq!(rows, cold, "resumed {name} at {cutoff}");
            if !output.complete {
                let zero = job.advance(cutoff, 0).unwrap();
                assert_eq!(zero.rounds, 0);
                assert!(zero.output_work > 0);
                assert_eq!(json!(records(&zero.output.unwrap().tree, raw)), rows);
            }
            held.push((output.tree, rows));
        }
        let ended = job.advance(raw.len(), 1).unwrap();
        assert_eq!((ended.rounds, ended.output_work), (0, 0));
        assert!(ended.output.is_none());
        for (tree, before) in held {
            assert_eq!(json!(records(&tree, raw)), before);
        }
    }
}

#[test]
fn zero_rounds_do_not_scan_and_continuation_does_not_replay_the_prefix() {
    let text = Text::new(format!("[{}0]", "0,".repeat(1000)));
    let mut job = JsonParseJob::new(text);
    let pending = job.advance(200, 0).unwrap();
    assert_eq!((pending.rounds, pending.output_work), (0, 0));
    assert!(pending.output.is_none());
    let pending = job.advance(200, 200).unwrap();
    assert_eq!((pending.rounds, pending.output_work), (200, 0));
    assert!(pending.output.is_none());
    let output = job.advance(200, 1).unwrap().output.unwrap();
    assert_eq!(output.parsed_to, 200);
    assert_eq!(output.tree.root().range(), 0..201);
    let pending = job.advance(400, 199).unwrap();
    assert_eq!((pending.rounds, pending.output_work), (199, 0));
    assert!(pending.output.is_none());
    let resumed = job.advance(400, 1).unwrap().output.unwrap();
    assert_eq!(resumed.parsed_to, 400);
    assert_eq!(resumed.tree.root().range(), 0..401);
    assert_eq!(output.tree.root().range(), 0..201);
}

#[test]
fn a_virtual_deletion_prefix_is_not_eof() {
    let raw = "😀";
    let mut job = JsonParseJob::new(Text::new(raw));
    let prefix = job.advance(0, 1).unwrap().output.unwrap();
    assert_eq!(prefix.parsed_to, 0);
    assert!(!prefix.complete);
    assert_eq!(
        records(&prefix.tree, raw),
        [(0, 1, "JsonText", 0, 4), (1, 0, "⚠", 0, 4)]
    );
    let pending = job.advance(usize::MAX, 1).unwrap();
    assert_eq!((pending.rounds, pending.output_work), (1, 0));
    assert!(pending.output.is_none());
    assert!(job.advance(usize::MAX, 0).unwrap().output.is_none());
    let full = job.advance(usize::MAX, 1).unwrap().output.unwrap();
    assert_eq!(full.parsed_to, raw.len());
    assert!(full.complete);
    assert_eq!(records(&prefix.tree, raw), records(&full.tree, raw));
}

#[test]
fn configured_jobs_keep_node_row_identity_and_round_editor_offsets() {
    let text = Text::new("[\r\n\"😀\"\r\n]");
    let key = NodeProp::new();
    let language = json_language().with_node_props(&[key.add(|row| row.is("Array").then_some(42))]);
    let types: Vec<_> = language.node_types().collect();
    let mut job = language.parse_job(text.clone()).unwrap();
    let start = job.advance(2, 3).unwrap().output.unwrap();
    assert!(!start.complete);
    assert_eq!(start.parsed_to, 3);
    assert_eq!(start.tree.root().range(), 0..9);
    let overrun = job.advance(5, 0).unwrap().output.unwrap();
    assert_eq!(overrun.parsed_to, 8);
    assert_eq!(
        records(&overrun.tree, text.as_str()),
        records(&start.tree, text.as_str())
    );
    let full = drive(&mut job, text.len(), 1);
    for output in [start, overrun, full] {
        let mut pending = vec![output.tree.root()];
        while let Some(node) = pending.pop() {
            assert!(ptr::eq(
                node.node_type().unwrap(),
                types[usize::from(node.term())]
            ));
            if node.name() == "Array" {
                assert_eq!(node.prop(&key), Some(&42));
            }
            pending.extend(node.children());
        }
    }
    assert!(
        language
            .configure(Some(true), None)
            .parse_job(text)
            .is_none()
    );
}
