//! Oracle: unchanged `@codemirror/legacy-modes` 6.5.5 exports and `StringStream`
//! from `@codemirror/streamparser` 6.0.0. Both npm tarballs match the SHA-512
//! digests in `docs/codemirror-upstream.json`. The oracle ran outside the repository.
//!
//! Rows record raw styles and UTF-8 byte ranges. Matching slices of one
//! surrogate pair are joined. Indent values precede tokenization of each line.

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
const ORACLE: &str = include_str!("fixtures/legacy_template_streams.json");

#[test]
fn matches_the_pinned_template_stream_oracle() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let module = case["module"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let settings = StreamSettings {
            indent_unit: case["unit"].as_u64().unwrap().try_into().unwrap(),
            tab_size: NonZeroUsize::new(case["tab"].as_u64().unwrap().try_into().unwrap()).unwrap(),
        };
        let mode = LegacyMode::from_export(&format!("{PREFIX}{module}"), export).unwrap();
        let mut tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        let expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        for expected in expected {
            let token = tokens
                .next()
                .unwrap_or_else(|| panic!("{export} ({label}): missing token {expected}"))
                .unwrap_or_else(|err| panic!("{export} ({label}): {err}"));
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                expected,
                "{export} ({label}) at {:?}: {:?}",
                token.range,
                token.text,
            );
        }
        assert!(tokens.next().is_none(), "{export} ({label}): extra tokens");

        let doc = mode.language().parse(Text::new(source), settings).unwrap();
        let positions =
            std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1));
        let indents: Vec<_> = positions
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect();
        assert_eq!(
            json!(indents),
            case["indents"],
            "{export} ({label}) indents"
        );
    }
}

#[test]
fn matches_export_spelling_and_metadata() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (expected, metadata) in [
        LegacyMode::CoffeeScript,
        LegacyMode::LiveScript,
        LegacyMode::PegJs,
        LegacyMode::Pug,
    ]
    .into_iter()
    .zip(oracle["metadata"].as_array().unwrap())
    {
        let module = metadata["module"].as_str().unwrap();
        let export = metadata["export"].as_str().unwrap();
        let path = format!("{PREFIX}{module}");
        assert_eq!(LegacyMode::from_export(&path, export), Some(expected));
        assert_eq!(LegacyMode::from_export(&path, &export.to_uppercase()), None);
        if module != export {
            assert_eq!(LegacyMode::from_export(&path, module), None);
        }
        assert_eq!(LegacyMode::from_export(&format!("{path}.js"), export), None);
        assert_eq!(LegacyMode::from_export(module, export), None);
        assert_eq!(json!(expected.name()), metadata["name"]);
        assert_eq!(json!(expected.merge_tokens()), metadata["merge"]);

        let language = expected.language();
        assert_eq!(language.name(), expected.name().unwrap_or_default());
        let comments = language
            .language_data()
            .get::<CommentTokens>("commentTokens");
        assert_eq!(
            json!(comments.and_then(|tokens| tokens.line)),
            metadata["commentTokens"]["line"]
        );
        assert!(comments.and_then(|tokens| tokens.block).is_none());
    }
}

#[test]
fn matches_pug_state_copies() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let mode = LegacyMode::Pug;
    let settings = StreamSettings::default();
    let line_tokens = |line: &str, state: &mut LegacyState| {
        let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
        let mut tokens = Vec::new();
        while !stream.eol() {
            stream.start_token();
            let token = (0..10)
                .find_map(|_| {
                    let style = mode.token(&mut stream, state);
                    (stream.pos() > stream.start()).then(|| json!([stream.current(), style]))
                })
                .expect("Pug advances after its state transition");
            tokens.push(token);
        }
        json!(tokens)
    };
    for case in oracle["pugCopies"].as_array().unwrap() {
        let mut state = mode.start_state(settings.indent_unit);
        line_tokens(case["first"].as_str().unwrap(), &mut state);
        let mut copy = mode.copy_state(&state);
        let next = case["next"].as_str().unwrap();
        assert_eq!(line_tokens(next, &mut state), case["original"]);
        assert_eq!(line_tokens(next, &mut copy), case["copy"]);
    }
}

#[test]
fn replays_edits_with_multiline_state() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::CoffeeScript,
            "if yes\n  r = \"\"\"one\n  two\"\"\"\n  then x\n",
            "two",
            "more\n  two",
        ),
        (
            LegacyMode::LiveScript,
            "r = // first\n  second //g\na = 2\n",
            "second",
            "# skip\n  second",
        ),
        (
            LegacyMode::PegJs,
            "start = ([a-z\n0-9] / _)+\n",
            "0-9",
            "_\\]\n0-9",
        ),
        (
            LegacyMode::Pug,
            "p(a=[\n  1, 2\n  ] disabled)\n",
            "1, 2",
            "1,\n  f(2)",
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
