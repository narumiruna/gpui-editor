//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs match
//! the SHA-512 digests in `docs/codemirror-upstream.json`. The oracle ran outside
//! the repository. These tests run only Rust.
//!
//! Each row records raw token styles and UTF-8 byte ranges for one logical
//! line. Matching slices of one surrogate pair form one scalar token. Indents
//! were read before tokenizing each line in an independent scan.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_grammar_modes.json");

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_grammar_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = from_export(name, name).unwrap();
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
        let positions =
            std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1));
        let actual_indents = positions
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        let expected_indents = case["indents"]
            .as_array()
            .unwrap()
            .iter()
            // The Rust API uses unsigned columns. Clamp negative DTD indents to zero.
            .map(|value| value.as_i64().map(|n| usize::try_from(n).unwrap_or(0)))
            .collect::<Vec<_>>();
        assert_eq!(actual_indents, expected_indents, "{name} ({label}) indents");

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens");
        assert_eq!(
            json!(comments.map(|tokens| {
                assert!(tokens.block.is_none());
                json!({ "line": tokens.line })
            })),
            case["commentTokens"],
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
    for (name, wrong, mode) in [
        ("dtd", "DTD", LegacyMode::Dtd),
        ("ntriples", "nTriples", LegacyMode::NTriples),
        ("sparql", "SPARQL", LegacyMode::Sparql),
    ] {
        assert_eq!(from_export(name, name), Some(mode));
        assert_eq!(from_export(name, wrong), None);
        assert_eq!(from_export(wrong, name), None);
        assert_eq!(from_export(&format!("{name}.js"), name), None);
        assert_eq!(LegacyMode::from_export(name, name), None);
    }
}

#[test]
fn preserves_the_dtd_last_token_kind_in_a_copy() {
    let mode = LegacyMode::Dtd;
    let settings = StreamSettings {
        indent_unit: 4,
        ..StreamSettings::default()
    };
    let mut state = mode.start_state(settings.indent_unit);
    let mut stream = StringStream::new("<!ELEMENT node", settings.tab_size, settings.indent_unit);
    while !stream.eol() {
        stream.start_token();
        mode.token(&mut stream, &mut state);
    }
    let mut copy = mode.copy_state(&state);
    let mut stream = StringStream::new("|", settings.tab_size, settings.indent_unit);
    mode.token(&mut stream, &mut copy);
    assert_eq!(mode.indent(&state, ">", settings.indent_unit), Some(0));
    assert_eq!(mode.indent(&copy, ">", settings.indent_unit), Some(4));
}

#[test]
fn replays_edits_through_stateful_lines() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Dtd,
            "<!DOCTYPE root [\n  <!-- note\n  -->\n  <!ELEMENT root ANY>\n]>\n",
            "-->",
            "more\n  -->",
        ),
        (
            LegacyMode::Sparql,
            "SELECT * WHERE {\n  ?s ex:p [ex:q ?o]\n  ex:r ?v.\n}\n",
            "ex:q ?o]",
            "ex:q ?o;\n      ex:next \"v\"]",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let start = source.find(before).unwrap();
        replay
            .apply_changes(
                &ChangeSet::single(Change::new(start..start + before.len(), after)).unwrap(),
            )
            .unwrap();
        let updated = source.replacen(before, after, 1);
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            replay
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?} highlights",
        );
        for (pos, _) in updated.match_indices('\n') {
            assert_eq!(
                replay.indentation(pos + 1, Default::default()).unwrap(),
                fresh.indentation(pos + 1, Default::default()).unwrap(),
                "{mode:?} indentation at {}",
                pos + 1,
            );
        }
    }
}
