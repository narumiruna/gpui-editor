//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs match
//! the SHA-512 digests in `docs/codemirror-upstream.json`. The oracle ran outside
//! the repository. These tests run only Rust.
//!
//! Rows record raw token styles and UTF-8 byte ranges. Matching slices of one
//! surrogate pair are joined. Indents are recorded before tokenizing each line.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyMode, LegacyState},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_vendor_modes.json");

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_vendor_mode_oracle() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let settings = StreamSettings {
            indent_unit: usize::try_from(case["unit"].as_u64().unwrap()).unwrap(),
            tab_size: NonZeroUsize::new(
                usize::try_from(case["tabSize"].as_u64().unwrap()).unwrap(),
            )
            .unwrap(),
        };
        let mode = from_export(name, name).unwrap();
        let mut tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        let expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        for expected in expected {
            let token = tokens
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
        assert!(tokens.next().is_none(), "{name} ({label}): extra tokens");

        let doc = mode.language().parse(Text::new(source), settings).unwrap();
        let positions =
            std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1));
        let indents: Vec<_> = positions
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect();
        // The Rust API clamps negative columns in the upstream mode to zero.
        let expected: Vec<_> = case["indents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_i64().map(|n| usize::try_from(n).unwrap_or(0)))
            .collect();
        assert_eq!(indents, expected, "{name} ({label}) indents");
    }
}

#[test]
fn matches_the_pinned_language_metadata() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for entry in oracle["metadata"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let mode = from_export(name, name).unwrap();
        assert_eq!(mode.name(), Some(name));
        assert_eq!(json!(mode.merge_tokens()), entry["mergeTokens"]);
        let language = mode.language();
        assert_eq!(language.name(), name);
        let doc = language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").map(|tokens| {
            json!({"line": tokens.line, "block": tokens.block.map(|(open, close)| json!({"open": open, "close": close}))})
        });
        assert_eq!(json!(comments), entry["commentTokens"], "{name}");
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            entry["autocomplete"],
            "{name}"
        );
        let indent_input = data.get::<fn(&str) -> bool>("indentOnInput");
        if let Some(probes) = entry["indentOnInput"].as_array() {
            let indent_input = indent_input.unwrap();
            for probe in probes {
                let input = probe[0].as_str().unwrap();
                assert_eq!(json!(indent_input(input)), probe[1], "{name}: {input:?}");
            }
        } else {
            assert!(indent_input.is_none(), "{name}");
        }
    }
}

#[test]
fn resolves_only_the_published_spelling() {
    for (name, mode) in [
        ("fcl", LegacyMode::Fcl),
        ("forth", LegacyMode::Forth),
        ("ecl", LegacyMode::Ecl),
        ("idl", LegacyMode::Idl),
        ("pig", LegacyMode::Pig),
        ("puppet", LegacyMode::Puppet),
    ] {
        assert_eq!(from_export(name, name), Some(mode));
        assert_eq!(from_export(name, &name.to_uppercase()), None);
        assert_eq!(from_export(&name.to_uppercase(), name), None);
        assert_eq!(from_export(&format!("{name}.js"), name), None);
        assert_eq!(LegacyMode::from_export(name, name), None);
    }
}

#[test]
fn separates_forth_copies_of_the_dictionary_and_compilation_state() {
    let mode = LegacyMode::Forth;
    let settings = StreamSettings::default();
    let mut original = mode.start_state(settings.indent_unit);
    let mut copy = mode.copy_state(&original);
    let token = |line, state: &mut LegacyState| {
        let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
        mode.token(&mut stream, state)
    };
    assert_eq!(token(": twice", &mut copy).as_deref(), Some("def"));
    assert_eq!(token("twice", &mut copy).as_deref(), Some("variable"));
    assert_eq!(token("twice", &mut original).as_deref(), Some("atom"));
    assert_eq!(token("POSTPONE DUP", &mut copy).as_deref(), Some("builtin"));
    assert_eq!(token("POSTPONE DUP", &mut original).as_deref(), Some("def"));
}

#[test]
fn replays_edits_through_stateful_lines() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Fcl,
            "VAR_INPUT\n  (* comment\n  *)\nend_var\n",
            "*)",
            "more\n  */",
        ),
        (
            LegacyMode::Forth,
            ": twice\n   DUP + ;\nTWICE .\n",
            "DUP + ;",
            "DUP +\n; : four twice twice ; four",
        ),
        (
            LegacyMode::Ecl,
            "x := if (\n  y,\n  1, 0\n);\n",
            "y,",
            "f(y,\n    2),",
        ),
        (
            LegacyMode::Pig,
            "LOAD 'left\\\n right' AS int;\nDUMP a;\n",
            "right'",
            "continued\\\n right'",
        ),
        (
            LegacyMode::Puppet,
            "class app {\n  group => \"a\n  $user b\";\n}\n",
            "$user b",
            "${::user}\n  $other b",
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
            "{mode:?} highlights"
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
