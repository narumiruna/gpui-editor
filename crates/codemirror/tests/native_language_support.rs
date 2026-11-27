use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditorState, Text, Transaction,
    commands::CommentTokens,
    language::{StreamSettings, javascript::JavascriptOptions},
    language_data::{CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage},
};
use serde_json::{Map, Value, json};

const LABELS: &[&str] = &[
    "class", "private", "return", "prior", "color", "if", "lower", "endif", "item", "SELECT",
    "local", "arg", "div", "true", "red",
];

fn byte_offset(source: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (at, ch) in source.char_indices() {
        if units == utf16 {
            return at;
        }
        units += ch.len_utf16();
    }
    assert_eq!(units, utf16);
    source.len()
}

#[test]
fn default_sources_and_metadata_follow_selected_pinned_catalog_contexts() {
    let fixtures = include_str!("fixtures/native_language_support.jsonl");
    let mut mismatches = Vec::new();
    for line in fixtures.lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["lang"].as_str().unwrap();
        let source = case["doc"].as_str().unwrap();
        let catalog_name = if name == "angular" {
            "Angular Template"
        } else {
            name
        };
        let support = *LanguageDescription::match_name(LANGUAGES, catalog_name, false)
            .unwrap()
            .native_support_load()
            .load()
            .unwrap();
        let doc = support.parse(Text::new(source)).unwrap();
        let offset =
            |at: &Value| byte_offset(source, usize::try_from(at.as_u64().unwrap()).unwrap());
        let pos = offset(&case["pos"]);
        let data = doc.data_at(pos, Assoc::Before);
        let comments: Vec<_> = data
            .iter()
            .filter_map(|entry| {
                let tokens = entry.get::<CommentTokens>("commentTokens")?;
                let mut data = Map::new();
                if let Some(line) = tokens.line {
                    data.insert("line".into(), json!(line));
                }
                if let Some((open, close)) = tokens.block {
                    data.insert("block".into(), json!({"open": open, "close": close}));
                }
                Some(data)
            })
            .collect();
        let brackets: Vec<_> = data
            .iter()
            .filter_map(|entry| {
                let config = entry.get::<CloseBracketData>("closeBrackets")?;
                let mut data = Map::new();
                if let Some(brackets) = config.brackets {
                    data.insert("brackets".into(), json!(brackets));
                }
                if !config.string_prefixes.is_empty() {
                    data.insert("stringPrefixes".into(), json!(config.string_prefixes));
                }
                Some(data)
            })
            .collect();
        let words: Vec<_> = data
            .iter()
            .filter_map(|entry| entry.get::<&str>("wordChars").copied())
            .collect();
        let indents = data
            .iter()
            .filter(|entry| entry.get::<fn(&str) -> bool>("indentOnInput").is_some())
            .count();
        let context = format!("{name} {pos} {source:?}");
        for (key, actual) in [
            ("commentTokens", json!(comments)),
            ("closeBrackets", json!(brackets)),
            ("wordChars", json!(words)),
            ("indentOnInput", json!(indents)),
        ] {
            let expected = &case["data"][key];
            let expected = if key == "indentOnInput" {
                json!(expected.as_array().unwrap().len())
            } else {
                expected.clone()
            };
            if actual != expected {
                mismatches.push(format!("{context} {key}: {actual} != {expected}"));
            }
        }
        let actual: Vec<_> = doc
            .indexed_completions(pos, false)
            .map(|(slot, result)| {
                let labels: Vec<_> = result.options.iter().map(|item| item.label.as_str())
                    .filter(|label| LABELS.contains(label)).collect();
                (slot, json!({"from": result.from, "to": result.to, "count": result.options.len(), "labels": labels}))
            })
            .collect();
        let expected: Vec<_> = case["results"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, result)| !result.is_null())
            .map(|(slot, result)| {
                let mut result = result.clone();
                result["from"] = json!(offset(&result["from"]));
                if !result["to"].is_null() {
                    result["to"] = json!(offset(&result["to"]));
                }
                (slot, result)
            })
            .collect();
        if actual != expected {
            mismatches.push(format!("{context} completions: {actual:?} != {expected:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn markdown_projects_mounts_and_snapshot_data_without_crossing_gaps() {
    let source = "> ```js\r\n> const prior = 1;\r\n> pri\r\n> ```";
    let position = source.rfind("pri").unwrap() + "pri".len();
    let support = NativeLanguage::Markdown.support();
    let doc = support
        .parse_with_code_languages(Text::new(source), StreamSettings::default(), LANGUAGES)
        .unwrap();
    let doc = Arc::new(doc);
    let js = NativeLanguage::Javascript(JavascriptOptions::default());
    let results = doc.completions(position, false);
    assert_eq!(doc.language_at(position, Assoc::Before), Some(js));
    assert_eq!(
        doc.language_at(position, Assoc::After),
        Some(NativeLanguage::Markdown)
    );
    assert_eq!(results.len(), 2);
    assert_eq!(results[1].from, position - 3);
    assert!(results[1].options.iter().any(|item| item.label == "prior"));
    assert!(!doc.highlights(position - 3..position).unwrap().is_empty());

    let mut state = EditorState::with_extensions(source, doc.language_data_extension()).unwrap();
    assert_eq!(
        state.language_data_at::<&str>("wordChars", position, Assoc::Before),
        ["$"]
    );
    let changes = ChangeSet::new(vec![Change::new(0..0, "x")]).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert!(
        state
            .language_data_at::<&str>("wordChars", position + 1, Assoc::Before)
            .is_empty()
    );
}

#[test]
fn projected_completion_cannot_replace_a_template_gap_or_split_a_character() {
    let source = "😀<script>pre{{ x }}fix</script>";
    let doc = NativeLanguage::Jinja
        .support()
        .parse(Text::new(source))
        .unwrap();
    assert_eq!(doc.language_at(1, Assoc::Before), None);
    let prefix_end = source.find("fix").unwrap() + "fix".len();
    assert!(doc.completions(prefix_end, false).is_empty());
}

#[test]
fn every_catalog_entry_loads_composed_native_services() {
    for entry in LANGUAGES {
        let load = entry.native_support_load();
        let support = load.load().unwrap_or_else(|_| panic!("{}", entry.name));
        let same = load.load().unwrap();
        assert!(Arc::ptr_eq(&support, &same), "{}", entry.name);
        assert_eq!(Some(support.language()), entry.built_in());
        let document = support.parse(Text::new("")).unwrap();
        let root = match support.language() {
            NativeLanguage::Vue | NativeLanguage::Angular => NativeLanguage::Html,
            language => language,
        };
        assert_eq!(
            document.language_at(0, Assoc::Before),
            Some(root),
            "{}",
            entry.name
        );
        assert!(document.highlights(Range::default()).unwrap().is_empty());
    }
}
