//! The scratch oracle ran the unchanged `@codemirror/legacy-modes` 6.5.5 ESM
//! exports with `@codemirror/streamparser` 6.0.0. Both tarballs matched the SRI
//! digests in `docs/codemirror-upstream.json`. It recorded raw styles, retries,
//! and indents with tab size 4 and indent unit 2. The end-state probes also use
//! unit 3. Matching surrogate tokens are joined into UTF-8 scalar ranges.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_compiled_modes.json");
const MODES: [(&str, LegacyMode); 4] = [
    ("elm", LegacyMode::Elm),
    ("swift", LegacyMode::Swift),
    ("d", LegacyMode::D),
    ("crystal", LegacyMode::Crystal),
];

#[test]
fn registers_only_exact_exports() {
    for (name, mode) in MODES {
        let path = format!("{PREFIX}{name}");
        assert_eq!(LegacyMode::from_export(&path, name), Some(mode));
        assert_eq!(LegacyMode::from_export(&path, &name.to_uppercase()), None);
        assert_eq!(LegacyMode::from_export(&format!("{path}.js"), name), None);
    }
    assert_eq!(
        LegacyMode::from_export(&format!("{PREFIX}elixir"), "elixir"),
        None
    );
}

#[test]
fn matches_pinned_token_and_indent_oracle() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (name, mode) in MODES {
        let expected = &oracle["modes"][name];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert_eq!(json!(mode.merge_tokens()), expected["mergeTokens"]);
        let language = mode.language();
        assert_eq!(language.name(), name);
        for (label, case) in expected["cases"].as_object().unwrap() {
            let source = case["source"].as_str().unwrap();
            let mut raw = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|row| row.as_array().unwrap());
            let mut tokenizer =
                Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
            for token in tokenizer.by_ref() {
                let token = token.unwrap_or_else(|err| panic!("{name}/{label}: {err}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                assert_eq!(
                    &json!([token.range.start, token.range.end, token.style]),
                    raw.next().expect("extra token"),
                    "{name}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(raw.next().is_none(), "{name}/{label}: missing tokens");

            for (i, input) in oracle["indentInputs"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let text = input.as_str().unwrap();
                for (j, unit) in [2, 3].into_iter().enumerate() {
                    assert_eq!(
                        mode.indent(tokenizer.state(), text, unit),
                        nonnegative_indent(&case["lastIndents"][i][j]),
                        "{name}/{label}: end indent for {text:?} with unit {unit}",
                    );
                }
            }

            let doc = language.parse(Text::new(source), settings).unwrap();
            let indents =
                line_starts(source).map(|pos| doc.indentation(pos, Default::default()).unwrap());
            let expected_indents = case["indents"]
                .as_array()
                .unwrap()
                .iter()
                .map(nonnegative_indent);
            assert!(indents.eq(expected_indents), "{name}/{label}: line indents");
        }
    }
}

#[test]
fn matches_pinned_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (name, mode) in MODES {
        let expected = &oracle["modes"][name];
        let doc = mode.language().parse(Text::new(""), settings).unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        assert_eq!(
            json!(comments.line),
            expected["data"]["commentTokens"]["line"]
        );
        assert_eq!(
            json!(
                comments
                    .block
                    .map(|(open, close)| json!({"open": open, "close": close}))
            ),
            expected["data"]["commentTokens"]["block"],
        );
        assert_eq!(
            json!(
                data.get::<&[&str]>("closeBrackets")
                    .map(|brackets| json!({"brackets": brackets}))
            ),
            expected["data"]["closeBrackets"],
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput");
        for (i, probe) in oracle["triggers"].as_array().unwrap().iter().enumerate() {
            let text = probe.as_str().unwrap();
            assert_eq!(
                json!(trigger.map(|test| test(text))),
                expected["triggers"][i],
                "{name}: {text:?}"
            );
        }
    }
}

#[test]
fn matches_crystal_zero_width_state_changes() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    let mode = LegacyMode::Crystal;
    for (label, case) in oracle["modes"]["crystal"]["cases"].as_object().unwrap() {
        let mut state = mode.start_state(settings.indent_unit);
        let mut retries = Vec::new();
        let mut offset = 0;
        for line in case["source"].as_str().unwrap().split('\n') {
            let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
            while !stream.eol() {
                stream.start_token();
                for _ in 0..10 {
                    let style = mode.token(&mut stream, &mut state);
                    if stream.pos() > stream.start() {
                        break;
                    }
                    retries.push(json!([offset + stream.start(), style]));
                }
                assert!(
                    stream.pos() > stream.start(),
                    "{label}: scanner did not advance"
                );
            }
            offset += line.len() + 1;
        }
        assert_eq!(json!(retries), case["retries"], "{label}");
    }
}

#[test]
fn replays_edits_from_open_mode_state() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Elm,
            "a = {- {-\ninner\n-}\n-}\nvalue",
            "inner",
            "new -}\n{- inner",
        ),
        (
            LegacyMode::Swift,
            "func f() {\n  let a = \"\"\"\\(call(\n   one)\n  ) end\"\"\"\n}",
            "one",
            "one,\n   two",
        ),
        (
            LegacyMode::D,
            "void f() {\n  /+ comment\n  content +/\n  foo(a,\n    b);\n}",
            "content",
            "new content\n  more",
        ),
        (
            LegacyMode::Crystal,
            "macro m\n  \"open\n  {{ y }} end\"\nend",
            "{{ y }}",
            "#{z}\n  {{ y }}",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let pos = source.find(before).unwrap();
        let change = ChangeSet::single(Change::new(pos..pos + before.len(), after)).unwrap();
        replay.apply_changes(&change).unwrap();
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
        for start in line_starts(&updated) {
            assert_eq!(
                replay.indentation(start, Default::default()),
                fresh.indentation(start, Default::default()),
                "{mode:?} at {start}"
            );
        }
    }
}

fn nonnegative_indent(value: &Value) -> Option<usize> {
    value
        .as_i64()
        .map(|width| usize::try_from(width.max(0)).unwrap())
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}
