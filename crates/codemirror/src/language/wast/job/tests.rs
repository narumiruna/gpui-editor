//! Replay @codemirror/lang-wast 6.0.2 with @lezer/lr 1.4.13.

use serde_json::{Value, json};

use super::*;
use crate::language::{
    ParseJob, ParseRequest, ParseService, ParseStep,
    line_projection::splits_crlf,
    wast::{WastDocument, parse_raw},
};

type Row<'a> = (usize, &'a str, usize, usize);

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn records<'a>(tree: &'a SyntaxTree, raw: &str) -> Vec<Row<'a>> {
    let mut pending = vec![(0, tree.root())];
    let mut rows = Vec::new();
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(raw.get(range.clone()).is_some(), "{range:?}");
        let parent = range.clone();
        rows.push((depth, node.name(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| {
            let child_range = child.range();
            assert!(parent.start <= child_range.start && child_range.end <= parent.end);
            (depth + 1, child)
        }));
    }
    rows
}

fn pinned_rows(rows: &[Row<'_>], expected: &Value, name: &str) {
    assert_eq!(rows[0].3, number(&expected["end"]), "{name}");
    assert_eq!(rows.len(), number(&expected["nodes"]), "{name}");
    let hash = serde_json::to_vec(rows)
        .unwrap()
        .into_iter()
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    assert_eq!(json!(hash), expected["hash"], "{name}");
    if !expected["rows"].is_null() {
        assert_eq!(json!(rows), expected["rows"], "{name}");
    }
}

// Floor a UTF-16 position inside a surrogate pair to the scalar start. The
// pinned parser can stop on either unit; the job never exposes half a scalar.
fn cutoff_byte(text: &str, stop: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > stop {
            return byte;
        }
    }
    assert_eq!(stop, units);
    text.len()
}

fn drive(
    job: &mut WastParseJob,
    cutoff: usize,
    rounds: usize,
) -> Result<WastParseOutput, ParseError> {
    loop {
        let step = job.advance(cutoff, rounds)?;
        assert!(step.rounds <= rounds);
        if let Some(output) = step.output {
            assert!(step.output_work > 0);
            assert!(!output.tree.has_incremental_state());
            return Ok(output);
        }
        assert_eq!(step.rounds, rounds);
        assert_eq!(step.output_work, 0);
    }
}

fn stage(job: &mut WastParseJob, cutoff: usize, rounds: usize) -> WastParseOutput {
    let output = drive(job, cutoff, rounds).unwrap();
    assert!(!output.complete);
    assert_eq!(output.parsed_to, cutoff);
    assert!(output.tree.root().range().end > cutoff);
    output
}

#[test]
fn a_requested_cutoff_inside_crlf_rounds_to_the_end_of_the_break() {
    let raw = ";; 😀\r\n(module)";
    let midpoint = raw.find("\r\n").unwrap() + 1;
    let mut rounded = WastParseJob::new(Text::new(raw));
    let output = drive(&mut rounded, midpoint, 1).unwrap();
    assert!(!output.complete);
    assert_eq!(output.parsed_to, midpoint + 1);
    let mut exact = WastParseJob::new(Text::new(raw));
    let expected = stage(&mut exact, midpoint + 1, 1);
    assert_eq!(records(&output.tree, raw), records(&expected.tree, raw));

    let raw = "(func)\r\n";
    let mut job = WastParseJob::new(Text::new(raw));
    let output = drive(&mut job, raw.len() - 1, 1).unwrap();
    assert!(output.complete);
    assert_eq!(output.parsed_to, raw.len());
}

fn utf16_rows<'a>(
    rows: &[Row<'a>],
    raw: &str,
    input: Option<&NormalizedInput<'_>>,
) -> Vec<Row<'a>> {
    let source = input.map_or(raw, |input| input.text.as_ref());
    let ascii = source.is_ascii();
    let utf16 = |at| {
        let at = input.map_or(at, |input| {
            assert!(!splits_crlf(raw, at));
            input.normalized_byte(at)
        });
        if ascii {
            at
        } else {
            source[..at].encode_utf16().count()
        }
    };
    rows.iter()
        .map(|&(depth, node, from, to)| (depth, node, utf16(from), utf16(to)))
        .collect()
}

#[test]
fn owned_recovery_matches_pinned_stages_and_cold_full_trees() {
    fn assert_send<T: Send>() {}
    assert_send::<WastParseJob>();
    for line in include_str!("../../../../tests/fixtures/wast_resume.jsonl").lines() {
        let fixture: Value = serde_json::from_str(line).unwrap();
        let name = fixture["name"].as_str().unwrap();
        let raw = fixture["raw"]
            .as_str()
            .unwrap()
            .repeat(number(&fixture["repeat"]));
        if name.starts_with("raw-") {
            let tree = parse_raw(&raw, false).unwrap();
            pinned_rows(
                &utf16_rows(&records(&tree, &raw), &raw, None),
                &fixture["full"],
                name,
            );
            continue;
        }
        let text = Text::new(&raw);
        let input = NormalizedInput::new(&raw);
        let mut job = WastParseJob::new(text.clone());
        let mut held = Vec::new();
        for expected in fixture["stages"].as_array().unwrap() {
            let stop = number(&expected["stop"]);
            let cutoff = input.original_byte(cutoff_byte(&input.text, stop));
            let staged = stage(&mut job, cutoff, 7);
            let mut fresh = WastParseJob::new(text.clone());
            let fresh = stage(&mut fresh, cutoff, 19);
            let rows = records(&staged.tree, &raw);
            assert_eq!(rows, records(&fresh.tree, &raw), "{name}");
            if name == "astral-half-stop" && stop <= 1 {
                assert_eq!(rows, [(0, "Module", 0, 4), (1, "⚠", 0, 4)]);
            } else {
                pinned_rows(&utf16_rows(&rows, &raw, Some(&input)), expected, name);
            }
            if stop == 9 && name == "editor-crlf" {
                assert_eq!(staged.tree.root().range(), 0..12);
            }
            held.push((json!(rows), staged));
        }

        let full = drive(&mut job, raw.len(), 13).unwrap();
        assert!(full.complete);
        assert_eq!(full.parsed_to, raw.len());
        let ended = job.advance(raw.len(), 1).unwrap();
        assert!(ended.output.is_none());
        assert_eq!(ended.rounds, 0);
        for (rows, staged) in held {
            assert_eq!(rows, json!(records(&staged.tree, &raw)), "{name}");
        }
        let full_rows = records(&full.tree, &raw);
        pinned_rows(
            &utf16_rows(&full_rows, &raw, Some(&input)),
            &fixture["full"],
            name,
        );
        let recovering = PARSER.parse_recovering(&input.text).unwrap();
        let expected: Vec<_> = records(&recovering, &input.text)
            .into_iter()
            .map(|(depth, name, from, to)| {
                (
                    depth,
                    name,
                    input.original_byte(from),
                    input.original_byte(to),
                )
            })
            .collect();
        assert_eq!(full_rows, expected, "{name}");
        let cold = WastDocument::parse(text);
        let mut pending = vec![(0, cold.root())];
        let mut cold_rows = Vec::new();
        while let Some((depth, node)) = pending.pop() {
            cold_rows.push((
                depth,
                node.kind().name(),
                node.range().start,
                node.range().end,
            ));
            pending.extend(node.children().rev().map(|child| (depth + 1, child)));
        }
        assert_eq!(full_rows, cold_rows, "{name}");
    }
}

#[test]
fn one_round_quota_keeps_the_virtual_astral_deletion_step() {
    let mut job = WastParseJob::new(Text::new("😀 (func $f)\n"));
    assert!(job.advance(0, 1).unwrap().output.is_none());
    let half = job.session.frontier().unwrap();
    assert_eq!((half.0, half.1), (4, false));
    assert!(job.advance(0, 0).unwrap().output.is_none());
    assert_eq!(job.session.frontier(), Some(half));
    let output = stage(&mut job, 0, 1);
    assert_eq!(output.tree.root().range(), 0..4);
}

#[test]
fn crossing_token_or_crlf_at_eof_does_not_finish_the_job() {
    for (raw, cutoff, end) in [("\"😀\"", 2, 6), ("(func)\r\n", 6, 8), ("", 0, 0)] {
        let mut job = WastParseJob::new(Text::new(raw));
        if !raw.is_empty() {
            let output = drive(&mut job, cutoff, 1).unwrap();
            assert!(!output.complete);
            assert_eq!(output.parsed_to, if cutoff == 2 { 5 } else { cutoff });
            assert_eq!(output.tree.root().range(), 0..end);
            let pending = job.advance(end, 0).unwrap();
            assert!(pending.output.is_none());
        }
        let complete = job.advance(end, 1).unwrap();
        assert_eq!(complete.rounds, 1);
        assert!(complete.output.unwrap().complete);
    }
}

#[test]
fn reaching_5000_does_not_publish_or_replay_the_3000_prefix() {
    // A caller can hand off a ready tree to ParseService without changing
    // how the service schedules work or manages the published snapshot.
    struct Ready<'a>(&'a std::cell::RefCell<Option<WastParseOutput>>);
    impl ParseJob for Ready<'_> {
        type Tree = SyntaxTree;
        type Error = ParseError;

        fn advance(
            &mut self,
            _: ParseRequest<'_, SyntaxTree>,
        ) -> Result<ParseStep<SyntaxTree>, ParseError> {
            Ok(self
                .0
                .borrow_mut()
                .take()
                .map_or(ParseStep::Pending, |output| {
                    ParseStep::publish(output.tree, output.parsed_to)
                }))
        }
        fn restart(&mut self) {}
    }

    let text = Text::new("(func $f)\n".repeat(12_000));
    let mut job = WastParseJob::new(text.clone());
    let ready = std::cell::RefCell::new(None);
    let mut publisher = ParseService::new(text.clone(), Ready(&ready));
    let pending = job.advance(3_000, 1_800).unwrap();
    assert_eq!(pending.rounds, 1_800);
    assert!(pending.output.is_none());
    let first = job.advance(3_000, 1).unwrap();
    assert_eq!(first.rounds, 1);
    let output = first.output.unwrap();
    assert_eq!(output.tree.root().range(), 0..3_001);
    ready.replace(Some(output));
    publisher.ensure(3_000, 1).unwrap().unwrap();
    let old = publisher.syntax_tree().unwrap();

    let pending = job.advance(5_000, 1_199).unwrap();
    assert_eq!(pending.rounds, 1_199);
    assert!(pending.output.is_none());
    let second = job.advance(5_000, 1).unwrap();
    assert_eq!(second.rounds, 1);
    let output = second.output.unwrap();
    assert_eq!(output.tree.root().range(), 0..5_001);
    assert_eq!(output.parsed_to, 5_000);
    assert!(std::ptr::eq(old, publisher.syntax_tree().unwrap()));
    assert_eq!(old.tree().root().range(), 0..3_001);
    assert!(!publisher.available(5_000));

    let mut control = WastParseJob::new(text.clone());
    assert!(control.advance(text.len(), 3_001).unwrap().output.is_none());
    assert_eq!(job.session.frontier(), control.session.frontier());
    let again = job.advance(5_000, 0).unwrap();
    assert_eq!(again.rounds, 0);
    assert_eq!(again.output_work, second.output_work);
    assert!(again.output_work > first.output_work);
    assert_eq!(
        records(&again.output.unwrap().tree, text.as_str()),
        records(&output.tree, text.as_str())
    );
    assert_eq!(job.session.frontier(), control.session.frontier());

    ready.replace(Some(output));
    let published = publisher.ensure(5_000, 1).unwrap().unwrap();
    assert_eq!(published.tree().root().range(), 0..5_001);
    assert!(publisher.available(5_000));
    assert!(!publisher.available(5_001));
}

#[test]
fn continuation_keeps_recovery_depth_and_does_not_commit_staged_output() {
    let open = "(".repeat(4_000);
    let mut job = WastParseJob::new(Text::new(&open));
    assert_eq!(stage(&mut job, 100, 17).tree.root().range(), 0..101);
    let resumed = drive(&mut job, open.len(), 17).unwrap();
    assert!(resumed.complete);
    let recovering = PARSER.parse_recovering(&open).unwrap();
    let rows = records(&resumed.tree, &open);
    assert_eq!(rows, records(&recovering, &open));
    assert_eq!(
        rows.iter().find(|row| row.1 == "⚠").map(|row| row.2),
        Some(2_800)
    );

    let balanced = open + &")".repeat(4_000);
    let mut job = WastParseJob::new(Text::new(&balanced));
    assert_eq!(stage(&mut job, 100, 17).tree.root().range(), 0..101);
    let staged = stage(&mut job, 7_950, 17);
    let frontier = job.session.frontier();
    let repeated = stage(&mut job, 7_950, 0);
    assert_eq!(
        records(&staged.tree, &balanced),
        records(&repeated.tree, &balanced)
    );
    assert_eq!(job.session.frontier(), frontier);
    let full = drive(&mut job, balanced.len(), 17).unwrap();
    assert!(full.complete);
    let mut fresh = WastParseJob::new(Text::new(&balanced));
    let cold = drive(&mut fresh, balanced.len(), usize::MAX).unwrap();
    assert_eq!(
        records(&full.tree, &balanced),
        records(&cold.tree, &balanced)
    );
    assert!(job.advance(balanced.len(), 1).unwrap().output.is_none());
}

#[test]
fn recovery_keeps_tokenizer_work_limits_and_projects_the_error_offset() {
    // An unterminated block comment can make the tokenizer scan far ahead
    // at many positions. Deferring repeat lists does not change this work.
    let input = "(;".repeat(512);
    let limit = |offset| ParseError::LimitExceeded { offset };
    assert_eq!(PARSER.parse_recovering(&input).unwrap_err(), limit(845));
    let mut job = WastParseJob::new(Text::new(&input));
    assert_eq!(drive(&mut job, input.len(), 17).unwrap_err(), limit(845));
    assert!(job.advance(input.len(), 1).unwrap().output.is_none());

    let normalized = format!("\n{input}");
    let ParseError::LimitExceeded { offset } = PARSER.parse_recovering(&normalized).unwrap_err()
    else {
        panic!("expected the native recovery limit");
    };
    let raw = format!("\r\n{input}");
    let mut projected = WastParseJob::new(Text::new(&raw));
    assert_eq!(
        drive(&mut projected, raw.len(), 17).unwrap_err(),
        limit(offset + 1)
    );
}
