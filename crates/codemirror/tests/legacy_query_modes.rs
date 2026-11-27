//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. The npm tarballs match
//! the SHA-512 digests in `docs/codemirror-upstream.json`. The oracle ran outside
//! the repository. These tests run only Rust.
//!
//! Token ranges use UTF-8 bytes. Tokens that split a surrogate pair in the
//! oracle form one scalar token. Indents were read before scanning each line.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_query_modes.json");

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> {
    std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1))
}

#[test]
fn matches_the_pinned_query_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    for case in cases {
        let module = case["module"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let settings = StreamSettings {
            tab_size: usize::try_from(case["tab"].as_u64().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
            indent_unit: usize::try_from(case["unit"].as_u64().unwrap()).unwrap(),
        };
        let mode = from_export(module, export).unwrap();
        assert_eq!(mode.name(), Some(name));
        assert_eq!(json!(mode.merge_tokens()), case["mergeTokens"]);

        let mut actual = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        let expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        for expected in expected {
            let token = actual
                .next()
                .unwrap_or_else(|| panic!("{name} ({label}): missing token {expected}"))
                .unwrap();
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                expected,
                "{name} ({label}) at {:?}: {:?}",
                token.range,
                token.text,
            );
        }
        assert!(actual.next().is_none(), "{name} ({label}): extra tokens");

        let language = mode.language();
        assert_eq!(language.name(), name);
        let doc = language.parse(Text::new(source), settings).unwrap();
        let indents: Vec<_> = line_starts(source)
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect();
        assert_eq!(json!(indents), case["indents"], "{name} ({label}) indents");

        let data = doc.language_data_at(0).unwrap();
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            case["autocomplete"],
            "{name} autocomplete"
        );
        let comments = data.get::<CommentTokens<'static>>("commentTokens");
        assert_eq!(
            json!(comments.map(|tokens| {
                assert!(tokens.line.is_none());
                let (open, close) = tokens.block.unwrap();
                json!({ "block": { "open": open, "close": close } })
            })),
            case["commentTokens"],
            "{name} comments"
        );
        let indent_input = data.get::<fn(&str) -> bool>("indentOnInput");
        if let Some(probes) = case["indentOnInput"].as_array() {
            let indent_input = indent_input.unwrap();
            for probe in probes {
                let input = probe[0].as_str().unwrap();
                assert_eq!(json!(indent_input(input)), probe[1], "{name}: {input:?}");
            }
        } else {
            assert!(indent_input.is_none());
        }
    }
}

#[test]
fn resolves_only_the_published_spelling() {
    for (module, export, wrong, mode) in [
        ("cypher", "cypher", "Cypher", LegacyMode::Cypher),
        ("solr", "solr", "Solr", LegacyMode::Solr),
        ("sieve", "sieve", "Sieve", LegacyMode::Sieve),
        ("webidl", "webIDL", "webidl", LegacyMode::WebIdl),
        ("xquery", "xQuery", "xquery", LegacyMode::XQuery),
    ] {
        assert_eq!(from_export(module, export), Some(mode));
        assert_eq!(from_export(module, wrong), None);
        assert_eq!(from_export(&module.to_ascii_uppercase(), export), None);
        assert_eq!(from_export(&format!("{module}.js"), export), None);
        assert_eq!(LegacyMode::from_export(module, export), None);
    }
}

#[test]
fn replays_edits_from_query_state_checkpoints() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Cypher,
            "MATCH (n:Node {\n a: [\n  1\n ]\n})\nRETURN n",
            "  1",
            "  {a: 1,\n   b: 2}",
        ),
        (
            LegacyMode::Solr,
            "field:\"first\\\n  second\" AND 1\nnext:value",
            "  second",
            "  changed",
        ),
        (
            LegacyMode::Sieve,
            "if true {\n text:\n first\n.\n stop;\n}",
            " first",
            " new\n text",
        ),
        (
            LegacyMode::WebIdl,
            "/* open\nclose */\ntypedef long Item;",
            "close */",
            "new\nclose */",
        ),
        (
            LegacyMode::XQuery,
            "<x attr=\"first\nsecond\">{fn:doc($x)}</x>",
            "second",
            "more\nsecond",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let start = source.find(before).unwrap();
        let change = ChangeSet::single(Change::new(start..start + before.len(), after)).unwrap();
        replay.apply_changes(&change).unwrap();
        let updated = source.replacen(before, after, 1);
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            replay
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?} highlights"
        );
        for pos in line_starts(&updated) {
            assert_eq!(
                replay.indentation(pos, Default::default()).unwrap(),
                fresh.indentation(pos, Default::default()).unwrap(),
                "{mode:?} indentation at {pos}"
            );
        }
    }
}

#[test]
fn malformed_xquery_quoted_variable_reaches_end_of_line() {
    let settings = StreamSettings::default();
    let source = "$\"unclosed\nreturn $other";
    let mode = LegacyMode::XQuery;
    let mut tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
    let token = tokens.next().unwrap().unwrap();
    assert_eq!(token.text, "$\"unclosed");
    assert_eq!(token.style.as_deref(), Some("variable"));
    assert_eq!(
        tokens.next().unwrap().unwrap().style.as_deref(),
        Some("keyword")
    );
}
