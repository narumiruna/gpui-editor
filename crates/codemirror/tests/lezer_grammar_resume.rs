use gpui_editor_codemirror::{
    Text,
    language::{
        lezer::{ParseError, SyntaxTree},
        lezer_grammar::{LezerDocument, LezerParseJob, LezerParseOutput, parse_raw},
    },
};
use serde_json::{Value, json};

fn rows<'a>(tree: &'a SyntaxTree, source: &str) -> Vec<(usize, u16, &'a str, usize, usize)> {
    let mut rows = Vec::new();
    let mut pending = vec![(0, tree.root())];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(
            source.get(range.clone()).is_some(),
            "{range:?} in {source:?}"
        );
        assert_eq!(node.node_type().unwrap().name(), node.name());
        rows.push((depth, node.term(), node.name(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    rows
}

fn drive(job: &mut LezerParseJob, stop: usize, budget: usize) -> LezerParseOutput {
    for _ in 0..10_000 {
        let step = job.advance(stop, budget).unwrap();
        assert!(step.rounds <= budget);
        if let Some(output) = step.output {
            assert!(step.output_work > 0);
            return output;
        }
        assert_eq!((step.rounds, step.output_work), (budget, 0));
    }
    panic!("no result at {stop}");
}

#[test]
fn cold_and_continued_jobs_match_pinned_prefixes_and_eof() {
    fn assert_send<T: Send>() {}
    assert_send::<LezerParseJob>();
    let fixture = include_str!("fixtures/lezer_grammar_resume.jsonl");
    for line in fixture.lines().skip(1) {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["raw"].as_str().unwrap();
        let editor = case["editor"].as_bool().unwrap();
        let start = if editor {
            LezerParseJob::new
        } else {
            LezerParseJob::new_raw
        };
        let text = Text::new(source);
        let mut job = start(text.clone());
        let mut held = Vec::new();
        for stage in case["stages"].as_array().unwrap() {
            let stop = stage["stop"].as_u64().unwrap() as usize;
            let output = drive(&mut job, stop, 1);
            let cold = drive(&mut start(text.clone()), stop, 3);
            for output in [&output, &cold] {
                assert_eq!(output.complete, stop == text.len(), "{name} at {stop}");
                assert_eq!(output.parsed_to, stop);
                assert_eq!(
                    json!(rows(&output.tree, source)),
                    stage["rows"],
                    "{name} (editor={editor}) at {stop} in {source:?}"
                );
            }
            if !output.complete {
                let restaged = job.advance(stop, 0).unwrap();
                assert_eq!(restaged.rounds, 0);
                assert!(restaged.output_work > 0);
                assert_eq!(
                    json!(rows(&restaged.output.unwrap().tree, source)),
                    stage["rows"]
                );
            }
            held.push((output.tree, stage));
        }
        let ended = job.advance(usize::MAX, 1).unwrap();
        assert_eq!((ended.rounds, ended.output_work), (0, 0));
        assert!(ended.output.is_none());
        for (tree, stage) in held {
            assert_eq!(json!(rows(&tree, source)), stage["rows"]);
        }
    }
}

#[test]
fn token_overrun_does_not_consume_eof_or_replay_the_token() {
    let source = format!("// {}😀", "é".repeat(4096));
    let text = Text::new(&source);
    for mut job in [
        LezerParseJob::new_raw(text.clone()),
        LezerParseJob::new(text.clone()),
    ] {
        let pending = job.advance(0, 0).unwrap();
        assert_eq!((pending.rounds, pending.output_work), (0, 0));
        assert!(pending.output.is_none());
        let step = job.advance(0, 1).unwrap();
        assert_eq!(step.rounds, 1);
        let prefix = step.output.unwrap();
        assert!(!prefix.complete);
        assert_eq!(prefix.parsed_to, 0);
        assert_eq!(prefix.tree.root().range(), 0..text.len());
        let restaged = job.advance(10, 0).unwrap().output.unwrap();
        assert!(!restaged.complete);
        assert_eq!(restaged.parsed_to, 11);
        assert_eq!(rows(&restaged.tree, &source), rows(&prefix.tree, &source));
        assert!(job.advance(usize::MAX, 0).unwrap().output.is_none());
        let step = job.advance(usize::MAX, 1).unwrap();
        assert_eq!(step.rounds, 1);
        let full = step.output.unwrap();
        assert!(full.complete);
        assert_eq!(full.parsed_to, text.len());
        assert_eq!(rows(&prefix.tree, &source), rows(&full.tree, &source));
    }
}

#[test]
fn rounds_resume_the_frontier_after_a_long_prefix() {
    let text = Text::new("R {}\n".repeat(512));
    let mut job = LezerParseJob::new(text);
    let first = job.advance(499, 499).unwrap();
    assert_eq!((first.rounds, first.output_work), (499, 0));
    assert!(first.output.is_none());
    let first = job.advance(499, 1).unwrap().output.unwrap();
    assert_eq!(first.tree.root().range(), 0..500);
    let second = job.advance(999, 499).unwrap();
    assert_eq!((second.rounds, second.output_work), (499, 0));
    assert!(second.output.is_none());
    let second = job.advance(999, 1).unwrap().output.unwrap();
    assert_eq!(second.tree.root().range(), 0..1000);
    assert_eq!(first.tree.root().range(), 0..500);
}

#[test]
fn editor_rounds_crlf_and_both_modes_round_utf8_scalars() {
    let source = "// 😀\r\nR {}";
    let text = Text::new(source);
    let mut editor = LezerParseJob::new(text.clone());
    let mut raw = LezerParseJob::new_raw(text);
    let before_cr = 7;
    for job in [&mut editor, &mut raw] {
        assert_eq!(drive(job, 4, 1).parsed_to, before_cr);
    }
    assert_eq!(
        drive(&mut editor, before_cr + 1, 1).parsed_to,
        before_cr + 2
    );
    assert_eq!(drive(&mut raw, before_cr + 1, 1).parsed_to, before_cr + 1);
    assert_eq!(
        drive(&mut editor, before_cr + 2, 1).parsed_to,
        before_cr + 2
    );
}

#[test]
fn live_work_limit_ends_the_job_and_projects_the_error() {
    // Each unterminated comment can scan the rest of the input.
    let tail = "/* ".repeat(1024);
    let mut raw = LezerParseJob::new_raw(Text::new(format!("\n{tail}")));
    let mut editor = LezerParseJob::new(Text::new(format!("\r\n{tail}")));
    let exhaust = |job: &mut LezerParseJob| {
        let prefix = drive(job, 0, 1);
        let error = job.advance(usize::MAX, usize::MAX).unwrap_err();
        let ended = job.advance(usize::MAX, 1).unwrap();
        assert_eq!((ended.rounds, ended.output_work), (0, 0));
        assert!(ended.output.is_none());
        assert!(!prefix.complete);
        let ParseError::LimitExceeded { offset } = error else {
            panic!("unexpected parse error: {error:?}");
        };
        offset
    };
    assert_eq!(exhaust(&mut editor), exhaust(&mut raw) + 1);
}

#[test]
#[should_panic(expected = "cannot lower the cutoff")]
fn rejects_a_decreasing_cutoff_even_when_it_is_not_reached() {
    let mut job = LezerParseJob::new_raw(Text::new("R {}"));
    assert!(job.advance(3, 0).unwrap().output.is_none());
    let _ = job.advance(2, 0);
}

#[test]
fn synchronous_parses_keep_their_whole_document_paths() {
    let source = "// 😀\r@top R { @digit+ }\r\n";
    let raw = parse_raw(source, true).unwrap();
    assert_eq!(raw.root().range(), 0..source.len());
    assert_eq!(
        raw.root().children().next().unwrap().range(),
        0..source.len() - 1
    );
    let doc = LezerDocument::parse(Text::new(source));
    assert_eq!(doc.root().range(), 0..source.len());
    assert!(doc.nodes().any(|node| node.kind().name() == "CharClass"));
    let recovered = parse_raw("R { A |", false).unwrap();
    assert_eq!(recovered.root().range(), 0..7);
    assert!(parse_raw("R { A |", true).is_err());
}
