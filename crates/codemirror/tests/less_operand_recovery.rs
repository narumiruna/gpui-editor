use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings,
        less::{self, LessDocument, less_completion_source},
    },
};
use serde_json::{Value, json};

fn fixture() -> impl Iterator<Item = Value> {
    include_str!("fixtures/less_operand_recovery.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

#[test]
fn less_language_metadata_matches_pinned_package() {
    let record = fixture().next().unwrap();
    let (open, close) = less::BLOCK_COMMENT;
    assert_eq!(json!(less::NAME), record["language"]);
    assert_eq!(json!(less::WORD_CHARS), record["wordChars"]);
    assert_eq!(
        json!({"line": less::LINE_COMMENT, "block": {"open": open, "close": close}}),
        record["comments"]
    );
    for pair in record["indentOnInput"].as_array().unwrap() {
        let prefix = pair[0].as_str().unwrap();
        assert_eq!(json!(less::indent_on_input(prefix)), pair[1], "{prefix:?}");
    }
}

#[test]
fn missing_right_operands_match_pinned_less_services() {
    let mut previous: Option<LessDocument> = None;
    let mut mismatches = Vec::new();
    for record in fixture().skip(1) {
        let name = record["name"].as_str().unwrap();
        let source = record["source"].as_str().unwrap();
        let fresh = LessDocument::parse(Text::new(source));
        assert_eq!(fresh.text().as_str(), source, "{name}: source after parse");
        let actual = observe(&fresh, &record);
        previous = Some(
            if let Some([from, to, insert]) = record["change"].as_array().map(Vec::as_slice) {
                let change = Change::new(offset(from)..offset(to), insert.as_str().unwrap());
                let changes = ChangeSet::single(change).unwrap();
                let edited = previous.as_ref().unwrap().reparse(&changes).unwrap();
                assert_eq!(edited.text().as_str(), source, "{name}: source after edit");
                assert_eq!(observe(&edited, &record), actual, "{name}: fresh vs edit");
                edited
            } else {
                fresh
            },
        );
        for (key, value) in actual.as_object().unwrap() {
            if *value != record[key] {
                mismatches.push(format!(
                    "{name}: {key}\n  native: {value}\n  pinned: {}",
                    record[key]
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "\n{}", mismatches.join("\n"));
}

fn observe(doc: &LessDocument, record: &Value) -> Value {
    let nodes: Vec<_> = doc
        .nodes()
        .map(|node| {
            let range = node.range();
            let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
            json!([node.kind().name(), range.start, range.end, depth])
        })
        .collect();
    let folds: Vec<_> = record["folds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            // Query the logical line bounds from CodeMirror, including lone CR.
            let (start, end) = (offset(&row[0]), offset(&row[1]));
            let fold = doc.foldable(start, end).map(|r| [r.start, r.end]);
            json!([start, end, fold])
        })
        .collect();
    let indents: Vec<_> = record["indents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|probe| {
            let pos = offset(&probe[0]);
            let indent = |simulate_break, simulate_double_break| {
                doc.indentation(
                    pos,
                    StreamSettings::default(),
                    IndentOptions {
                        simulate_break,
                        simulate_double_break,
                        ..IndentOptions::default()
                    },
                )
            };
            json!([
                pos,
                indent(None, false),
                indent(Some(pos), false),
                indent(Some(pos), true)
            ])
        })
        .collect();
    let completions: Vec<_> = record["completions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|probe| {
            let pos = offset(&probe[0]);
            let complete = |explicit| less_completion_source(doc, pos, explicit).map(signature);
            json!([pos, complete(false), complete(true)])
        })
        .collect();
    json!({"nodes": nodes, "folds": folds, "indents": indents, "completions": completions})
}

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn signature(result: SourceResult) -> Value {
    let mut hash = 2_166_136_261u32;
    for item in result.options.iter() {
        for byte in item
            .label
            .bytes()
            .chain([0])
            .chain(item.kind.as_deref().unwrap().bytes())
            .chain([0])
            .chain(item.insert.as_deref().unwrap_or_default().bytes())
            .chain([b'\n'])
        {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    json!([result.from, result.to, result.options.len(), hash])
}
