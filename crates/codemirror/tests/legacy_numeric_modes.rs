//! The fixture records unmodified `@codemirror/legacy-modes` 6.5.5 with
//! `StringStream` from `@codemirror/streamparser` 6.0.0. The SHA512 digests of
//! both npm tarballs matched `docs/codemirror-upstream.json` before extraction.
//! The external oracle used tab size 4 and indent unit 2. It recorded raw token
//! styles and UTF-8 byte ranges, joining surrogate halves of the same style.
//! End-state indentation probes use units 2 and 3. Only Rust runs these tests.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_numeric_modes.json");

#[test]
fn matches_pinned_token_streams_and_indentation() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (name, expected_mode) in oracle["modes"].as_object().unwrap() {
        let mode = LegacyMode::from_export(&format!("{PREFIX}{name}"), name).unwrap();
        assert_eq!(json!(mode.name()), expected_mode["name"]);
        assert_eq!(json!(mode.merge_tokens()), expected_mode["mergeTokens"]);
        let language = mode.language();
        assert_eq!(language.name(), name);
        for case in expected_mode["cases"].as_array().unwrap() {
            let source = case["source"].as_str().unwrap();
            let label = case["label"].as_str().unwrap();
            let mut expected = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|line| line.as_array().unwrap());
            let mut tokenizer =
                Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
            for token in tokenizer.by_ref() {
                let token = token.unwrap_or_else(|err| panic!("{name}/{label}: {err}"));
                assert_eq!(
                    &json!([token.range.start, token.range.end, token.style]),
                    expected.next().expect("extra token"),
                    "{name}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(expected.next().is_none(), "{name}/{label}: missing tokens");

            let after = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    let input = input.as_str().unwrap();
                    [
                        mode.indent(tokenizer.state(), input, 2),
                        mode.indent(tokenizer.state(), input, 3),
                    ]
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(after),
                case["lastIndents"],
                "{name}/{label}: end state"
            );

            let doc = language.parse(Text::new(source), settings).unwrap();
            let indents = line_starts(source)
                .map(|pos| doc.indentation(pos, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{name}/{label}: indentation"
            );
        }
    }
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}

#[test]
fn matches_pinned_language_data_and_exact_exports() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (name, mode) in [
        ("mathematica", LegacyMode::Mathematica),
        ("octave", LegacyMode::Octave),
        ("sas", LegacyMode::Sas),
        ("q", LegacyMode::Q),
        ("modelica", LegacyMode::Modelica),
    ] {
        let module = format!("{PREFIX}{name}");
        assert_eq!(LegacyMode::from_export(&module, name), Some(mode));
        assert_eq!(LegacyMode::from_export(&module, &name.to_uppercase()), None);
        assert_eq!(LegacyMode::from_export(&format!("{module}.js"), name), None);
        let language = mode.language();
        let doc = language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let expected = &oracle["modes"][name]["data"];
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        assert_eq!(json!(comments.line), expected["commentTokens"]["line"]);
        assert_eq!(
            comments
                .block
                .map(|(open, close)| json!({"open": open, "close": close})),
            expected["commentTokens"].get("block").cloned(),
        );
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            expected["autocomplete"]
        );
    }
}

#[test]
fn copies_active_states_without_sharing_mutations() {
    let settings = StreamSettings::default();
    for (mode, opener, closer, style) in [
        (LegacyMode::Mathematica, "(* (* nested", "*) *)", "comment"),
        (LegacyMode::Octave, "%{", "%}", "comment"),
        (LegacyMode::Sas, "/*", "*/", "comment"),
        (LegacyMode::Q, "/", "\\", "comment"),
        (LegacyMode::Modelica, "\"open", "\"", "string"),
    ] {
        let mut first = mode.start_state(settings.indent_unit);
        let mut stream = StringStream::new(opener, settings.tab_size, settings.indent_unit);
        mode.token(&mut stream, &mut first);
        assert!(stream.eol());
        let mut fork = mode.copy_state(&first);
        let mut stream = StringStream::new(closer, settings.tab_size, settings.indent_unit);
        assert_eq!(mode.token(&mut stream, &mut first).as_deref(), Some(style));
        assert!(stream.eol());
        let mut stream = StringStream::new("body", settings.tab_size, settings.indent_unit);
        assert_eq!(mode.token(&mut stream, &mut fork).as_deref(), Some(style));
        assert!(stream.eol());
        let mut stream = StringStream::new("body", settings.tab_size, settings.indent_unit);
        assert_ne!(mode.token(&mut stream, &mut first).as_deref(), Some(style));
    }
}

#[test]
fn replays_edits_across_state_boundaries() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Mathematica,
            "(* outer (* nested\nbody\n*) *) x\ny\n",
            "body",
            "*) body",
        ),
        (
            LegacyMode::Octave,
            "%{\nbody\n%} 10'\nreturn\n",
            "body",
            "%} x",
        ),
        (
            LegacyMode::Sas,
            "data sample;\n/* body\nrun; */\nput 1;\n",
            "body",
            "*/",
        ),
        (LegacyMode::Q, "/\nbody\n\\\nshow 42\n", "body", "\\"),
        (
            LegacyMode::Modelica,
            "model M\n  /* body\n  Real p; */\n  equation\nend M;\n",
            "body",
            "*/ connector C",
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
            "{mode:?}"
        );
        for pos in line_starts(&updated) {
            assert_eq!(
                replay.indentation(pos, Default::default()).unwrap(),
                fresh.indentation(pos, Default::default()).unwrap(),
                "{mode:?} at {pos}",
            );
        }
    }
}
