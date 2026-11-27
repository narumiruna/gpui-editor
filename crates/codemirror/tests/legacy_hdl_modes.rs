//! Oracle: unmodified `mode/verilog.js` and `mode/vhdl.js` from
//! `@codemirror/legacy-modes` 6.5.5, with `StringStream` from
//! `@codemirror/streamparser` 6.0.0. Both tarballs matched the SHA-512 SRI
//! in `docs/codemirror-upstream.json`. The scratch runner recorded raw styles
//! and UTF-8 byte ranges, joining surrogate halves with the same style.
//! It used tab size 4 and indent unit 2, clamped negative indents to zero,
//! and probed the final state with indent units 2 and 3. Only Rust runs here.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyMode, LegacyState},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_hdl_modes.json");

#[test]
fn matches_pinned_tokens_and_indents() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (export, expected_mode) in oracle["modes"].as_object().unwrap() {
        let mode = mode(export);
        assert_eq!(json!(mode.name()), expected_mode["name"]);
        assert!(mode.merge_tokens());
        assert_eq!(
            mode.language().name(),
            expected_mode["name"].as_str().unwrap()
        );
        for case in expected_mode["cases"].as_array().unwrap() {
            let label = case["label"].as_str().unwrap();
            let source = case["source"].as_str().unwrap();
            let mut expected = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|line| line.as_array().unwrap());
            let mut tokenizer =
                Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
            for token in tokenizer.by_ref() {
                let token = token.unwrap_or_else(|err| panic!("{export}/{label}: {err}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                assert_eq!(
                    &json!([token.range.start, token.range.end, token.style]),
                    expected.next().expect("extra token"),
                    "{export}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(
                expected.next().is_none(),
                "{export}/{label}: missing tokens"
            );
            let last_indents = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|text| {
                    let text = text.as_str().unwrap();
                    [
                        mode.indent(tokenizer.state(), text, 2),
                        mode.indent(tokenizer.state(), text, 3),
                    ]
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(last_indents),
                case["lastIndents"],
                "{export}/{label}: final state"
            );

            let doc = mode.language().parse(Text::new(source), settings).unwrap();
            let indents = line_starts(source)
                .map(|pos| doc.indentation(pos, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{export}/{label}: line indents"
            );
        }
    }
}

#[test]
fn recognizes_exact_exports_and_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (module, export, expected) in [
        ("verilog", "verilog", LegacyMode::Verilog),
        ("verilog", "tlv", LegacyMode::Tlv),
        ("vhdl", "vhdl", LegacyMode::Vhdl),
    ] {
        let path = format!("{PREFIX}{module}");
        assert_eq!(mode(export), expected);
        assert_eq!(LegacyMode::from_export(&path, &export.to_uppercase()), None);
        assert_eq!(LegacyMode::from_export(&format!("{path}.js"), export), None);
        let expected = &oracle["modes"][export]["data"];
        let doc = mode(export)
            .language()
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        assert_eq!(json!(comments.line), expected["commentTokens"]["line"]);
        assert_eq!(
            comments
                .block
                .map(|(open, close)| json!({"open": open, "close": close})),
            expected["commentTokens"].get("block").cloned(),
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        let results = oracle["triggerInputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| trigger(input.as_str().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(json!(results), expected["triggers"], "{export}");
    }
    assert_eq!(
        LegacyMode::from_export(&format!("{PREFIX}tlv"), "tlv"),
        None
    );
}

#[test]
fn copies_active_state_without_sharing_mutations() {
    for (mode, open, close, next, original, closed) in [
        (
            LegacyMode::Verilog,
            "  /* open",
            "*/ fork",
            "join",
            None,
            Some(0),
        ),
        (
            LegacyMode::Tlv,
            "|scope\n/* open",
            "*/\n\\SV\n  module",
            "wire",
            Some(0),
            Some(4),
        ),
        (
            LegacyMode::Vhdl,
            "  call(first",
            ")",
            "next",
            Some(7),
            Some(4),
        ),
    ] {
        let settings = StreamSettings::default();
        let mut tokenizer = Tokenizer::new(&mode, open, settings.tab_size, settings.indent_unit);
        for token in tokenizer.by_ref() {
            token.unwrap();
        }
        let mut fork = mode.copy_state(tokenizer.state());
        let mut original_state = mode.copy_state(&fork);
        scan_lines(mode, &mut fork, close);
        assert_eq!(mode.indent(&fork, next, 2), closed, "{mode:?}");
        assert_eq!(mode.indent(&original_state, next, 2), original, "{mode:?}");
        scan_lines(mode, &mut original_state, close);
        assert_eq!(mode.indent(&original_state, next, 2), closed, "{mode:?}");
    }
}

#[test]
fn replays_edits_across_state_boundaries() {
    for (mode, source, before, after) in [
        (
            LegacyMode::Verilog,
            "module test;\n  /* open\n  body */\n  func(12,\n    3);\nendmodule\n",
            "body",
            "*/ fork\n  body",
        ),
        (
            LegacyMode::Tlv,
            "|pipe\n   @1\n      /* open\n      */ $a\n\\SV\nmodule test;\nendmodule\n",
            "      */ $a",
            "      body\n\\TLV",
        ),
        (
            LegacyMode::Vhdl,
            "entity x is\n  PORT(a : in bit);\n  call(first,\n    second)\nEND x;\n",
            "first,",
            "-- first,",
        ),
    ] {
        let language = mode.language();
        let settings = StreamSettings::default();
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
            "{mode:?}"
        );
        for pos in line_starts(&updated) {
            assert_eq!(
                replay.indentation(pos, Default::default()).unwrap(),
                fresh.indentation(pos, Default::default()).unwrap(),
                "{mode:?} at {pos}"
            );
        }
    }
}

fn scan_lines(mode: LegacyMode, state: &mut LegacyState, source: &str) {
    let settings = StreamSettings::default();
    for line in source.lines() {
        let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
        while !stream.eol() {
            stream.start_token();
            mode.token(&mut stream, state);
            assert!(stream.pos() > stream.start());
        }
    }
}

fn mode(export: &str) -> LegacyMode {
    let module = if export == "tlv" { "verilog" } else { export };
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export).unwrap()
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> {
    std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1))
}
