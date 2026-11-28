use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Text, Transaction,
    language::{
        IndentOptions, get_indentation,
        json::{JsonParseJob, JsonParseOutput, parse_raw},
        lezer::{ParseError, SyntaxTree},
    },
    language_data::{NativeSyntax, attached_language_snapshot, json_extension},
};
use serde_json::{Value, json};

#[test]
fn large_reductions_match_pinned_json_at_eof_and_after_edits() {
    let cases: Vec<Value> = include_str!("fixtures/json_large_reduction.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for case in &cases {
        let source = source(case);
        let state = EditorState::with_extensions(&source, json_extension()).unwrap();
        check_editor(&state, case);
        let strict_offset = if case["container"] == "Array" {
            2800
        } else {
            4666
        };
        assert!(
            matches!(parse_raw(&source, true),
                Err(ParseError::UnexpectedToken { offset, .. }) if offset == strict_offset),
            "strict: {case}"
        );
        assert_eq!(
            tree_shape(&parse_raw(&source, false).unwrap(), &source),
            case["shape"],
            "raw: {case}"
        );
        let mut job = JsonParseJob::new_raw(Text::new(&source));
        if let Some(prefix) = case.get("prefix") {
            let at = usize::try_from(prefix["at"].as_u64().unwrap()).unwrap();
            let output = drive(&mut job, at);
            assert!(!output.complete, "prefix: {case}");
            assert_eq!(output.parsed_to, at, "prefix: {case}");
            assert_eq!(
                tree_shape(&output.tree, &source),
                prefix["shape"],
                "prefix: {case}"
            );
        }
        let output = drive(&mut job, source.len());
        assert!(output.complete, "{case}");
        assert_eq!(output.parsed_to, source.len(), "{case}");
        assert_eq!(
            tree_shape(&output.tree, &source),
            case["shape"],
            "job: {case}"
        );
    }

    let [.., filled, empty] = cases.as_slice() else {
        panic!("missing value-line cases");
    };
    let mut state = EditorState::with_extensions(source(filled), json_extension()).unwrap();
    for (range, insert, case) in [(5811..5812, "", empty), (5811..5811, "0", filled)] {
        let changes = ChangeSet::single(Change::new(range, insert)).unwrap();
        state.update(Transaction::new(changes)).unwrap();
        assert_eq!(state.text().as_str(), source(case));
        check_editor(&state, case);
    }
}

fn drive(job: &mut JsonParseJob, cutoff: usize) -> JsonParseOutput {
    (0..=cutoff)
        .find_map(|_| job.advance(cutoff, 17).unwrap().output)
        .expect("JSON output at requested cutoff")
}

fn source(case: &Value) -> String {
    let depth = usize::try_from(case["depth"].as_u64().unwrap()).unwrap();
    let (open, close) = if case["container"] == "Array" {
        ("[", ']')
    } else {
        ("{\"x\":", '}')
    };
    let mut source = open.repeat(depth);
    source.push_str(case["tail"].as_str().unwrap());
    if case["closed"] == true {
        source.extend(std::iter::repeat_n(close, depth));
    }
    source
}

fn check_editor(state: &EditorState, case: &Value) {
    let snapshot = attached_language_snapshot(state).unwrap();
    let NativeSyntax::Json(doc) = snapshot.syntax() else {
        panic!("expected attached JSON syntax");
    };
    let mut shape = Shape::new();
    let mut pending = vec![(0, doc.root())];
    while let Some((depth, node)) = pending.pop() {
        shape.add(depth, node.kind().name(), node.range());
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    assert_eq!(shape.json(), case["shape"], "editor: {case}");
    for row in case["indents"].as_array().unwrap() {
        let at = usize::try_from(row[0].as_u64().unwrap()).unwrap();
        assert_eq!(
            json!(get_indentation(state, at, IndentOptions::default(), None).unwrap()),
            row[1],
            "indent at {at}: {case}"
        );
    }
}

fn tree_shape(tree: &SyntaxTree, source: &str) -> Value {
    let mut shape = Shape::new();
    let mut pending = vec![(0, tree.root())];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some(), "{range:?}");
        shape.add(depth, node.name(), range);
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    shape.json()
}

struct Shape {
    count: usize,
    hash: u32,
    outer_end: Option<usize>,
    number: Option<[usize; 3]>,
}

impl Shape {
    fn new() -> Self {
        Self {
            count: 0,
            hash: 2_166_136_261,
            outer_end: None,
            number: None,
        }
    }

    fn add(&mut self, depth: usize, name: &str, range: Range<usize>) {
        self.count += 1;
        self.hash = format!("{depth}:{name}:{}:{};", range.start, range.end)
            .bytes()
            .fold(self.hash, |hash, byte| {
                (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
            });
        if depth == 1 && matches!(name, "Object" | "Array") {
            self.outer_end.get_or_insert(range.end);
        }
        if name == "Number" {
            self.number.get_or_insert([depth, range.start, range.end]);
        }
    }

    fn json(&self) -> Value {
        json!({"count": self.count, "hash": self.hash, "outerEnd": self.outer_end, "number": self.number})
    }
}
