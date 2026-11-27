//! Matcher and ranking cases from the verified @codemirror/autocomplete 6.20.4
//! npm distribution. Regenerate with `fixtures/autocomplete_ranking.mjs`.

use std::{ops::Range, ptr};

use gpui_editor_codemirror::{
    Text,
    autocomplete::{
        Completion, CompletionMatcher, CompletionResult, CompletionSection, MatchMode, SectionRank,
        rank_completions, rank_completions_by,
    },
};
use serde_json::{Value, json};

fn fixture() -> impl Iterator<Item = Value> {
    include_str!("fixtures/autocomplete_ranking.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn spans(ranges: &[Range<usize>]) -> Vec<[usize; 2]> {
    ranges
        .iter()
        .map(|range| [range.start, range.end])
        .collect()
}

#[test]
fn scores_and_utf8_highlights_match_the_distribution() {
    for case in fixture().filter(|case| case.get("query").is_some()) {
        let query = case["query"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        for (mode, key) in [(MatchMode::Fuzzy, "fuzzy"), (MatchMode::Strict, "strict")] {
            let actual = CompletionMatcher::new(query, mode)
                .match_label(label)
                .map(|found| json!([found.score, spans(&found.ranges)]));
            assert_eq!(json!(actual), case[key], "{mode:?}: {query:?} in {label:?}");
        }
    }
}

fn completion(value: &Value) -> Completion<'_> {
    let section = match &value["section"] {
        Value::String(name) => Some(CompletionSection {
            name,
            rank: SectionRank::ByName,
        }),
        Value::Object(_) => Some(CompletionSection {
            name: value["section"]["name"].as_str().unwrap(),
            rank: match &value["section"]["rank"] {
                Value::String(rank) if rank == "dynamic" => SectionRank::Dynamic,
                Value::Number(number) => {
                    SectionRank::Fixed(i32::try_from(number.as_i64().unwrap()).unwrap())
                }
                _ => SectionRank::ByName,
            },
        }),
        Value::Null => None,
        other => panic!("invalid section: {other}"),
    };
    Completion {
        label: value["label"].as_str().unwrap(),
        display_label: value["displayLabel"].as_str(),
        sort_text: value["sortText"].as_str(),
        detail: value["detail"].as_str(),
        kind: value["type"].as_str(),
        info: value["info"].as_str(),
        insert: value["apply"].as_str(),
        boost: i32::try_from(value["boost"].as_i64().unwrap_or_default()).unwrap(),
        section,
    }
}

#[test]
fn ranked_option_identity_scores_and_highlights_match_the_distribution() {
    for case in fixture().filter(|case| case.get("sources").is_some()) {
        let name = case["name"].as_str().unwrap();
        let text = Text::new(case["doc"].as_str().unwrap());
        let mode = if case["strict"].as_bool().unwrap() {
            MatchMode::Strict
        } else {
            MatchMode::Fuzzy
        };
        let sources = case["sources"].as_array().unwrap();
        let options: Vec<Vec<_>> = sources
            .iter()
            .map(|source| {
                source["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(completion)
                    .collect()
            })
            .collect();
        let results: Vec<_> = sources
            .iter()
            .zip(&options)
            .map(|(source, options)| {
                let [start, end]: [usize; 2] =
                    serde_json::from_value(source["range"].clone()).unwrap();
                CompletionResult {
                    range: start..end,
                    options,
                    filter: source["filter"].as_bool().unwrap(),
                }
            })
            .collect();
        let ranked = if case["stable"].as_bool().unwrap() {
            rank_completions_by(&text, &results, mode, |_, _| std::cmp::Ordering::Equal)
        } else {
            rank_completions(&text, &results, mode)
        }
        .unwrap();
        let actual: Vec<_> = ranked
            .iter()
            .map(|row| {
                let (source, index) = options
                    .iter()
                    .enumerate()
                    .find_map(|(source, options)| {
                        options
                            .iter()
                            .position(|option| ptr::eq(option, row.completion))
                            .map(|index| (source, index))
                    })
                    .unwrap();
                assert_eq!(row.range, results[source].range, "{name}");
                json!([source, index, row.score, spans(&row.match_ranges)])
            })
            .collect();
        assert_eq!(json!(actual), case["rows"], "{name}");
    }
}
