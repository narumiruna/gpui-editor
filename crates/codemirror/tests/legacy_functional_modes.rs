//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both tarballs matched
//! `docs/codemirror-upstream.json`. The scratch runner used tab size 4 and
//! indent unit 2. It recorded raw styles and UTF-8 byte ranges and joined
//! surrogate halves with the same style. End-state probes also use unit 3.
//! These tests run only Rust.

use gpui_editor_codemirror::{
    Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyMode, LegacyState},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_functional_modes.json");
const MODES: [(&str, &str, LegacyMode); 5] = [
    ("haskell", "haskell", LegacyMode::Haskell),
    ("mllike", "oCaml", LegacyMode::OCaml),
    ("mllike", "fSharp", LegacyMode::FSharp),
    ("mllike", "sml", LegacyMode::Sml),
    ("erlang", "erlang", LegacyMode::Erlang),
];

#[test]
fn recognizes_only_exact_exports() {
    for (module, export, mode) in MODES {
        let subpath = format!("{PREFIX}{module}");
        assert_eq!(LegacyMode::from_export(&subpath, export), Some(mode));
        assert_eq!(
            LegacyMode::from_export(&subpath, &export.to_uppercase()),
            None
        );
        assert_eq!(
            LegacyMode::from_export(&format!("{subpath}.js"), export),
            None
        );
    }
    for (module, export) in [
        ("ocaml", "oCaml"),
        ("elixir", "elixir"),
        ("mllike", "ocaml"),
        ("mllike", "mlLike"),
    ] {
        assert_eq!(
            LegacyMode::from_export(&format!("{PREFIX}{module}"), export),
            None
        );
    }
}

#[test]
fn matches_pinned_modes() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (_, export, mode) in MODES {
        let expected = &oracle["modes"][export];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert!(mode.merge_tokens());
        let language = mode.language();
        assert_eq!(language.name(), mode.name().unwrap_or_default());
        for (label, case) in expected["cases"].as_object().unwrap() {
            let source = case["source"].as_str().unwrap();
            let mut tokens = case["tokens"]
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
                    tokens.next().expect("extra token"),
                    "{export}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(tokens.next().is_none(), "{export}/{label}: missing tokens");
            for (i, probe) in oracle["afterInputs"].as_array().unwrap().iter().enumerate() {
                let after = probe.as_str().unwrap();
                let actual = [2, 3].map(|unit| mode.indent(tokenizer.state(), after, unit));
                let expected = case
                    .get("lastIndents")
                    .map_or(json!([null, null]), |indents| indents[i].clone());
                assert_eq!(
                    json!(actual),
                    expected,
                    "{export}/{label}: end indent for {after:?}"
                );
            }

            let doc = language.parse(Text::new(source), settings).unwrap();
            let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
            let indents = starts
                .map(|pos| doc.indentation(pos, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{export}/{label}: line indents"
            );

            let data = doc.language_data_at(0).unwrap();
            let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
            let expected_data = &expected["data"];
            assert_eq!(json!(comments.line), expected_data["commentTokens"]["line"]);
            assert_eq!(
                json!(
                    comments
                        .block
                        .map(|(open, close)| json!({"open": open, "close": close}))
                ),
                expected_data["commentTokens"]["block"],
            );
            assert_eq!(
                json!(data.get::<&[&str]>("autocomplete")),
                expected_data["autocomplete"]
            );
        }
    }
}

#[test]
fn copies_continued_tokens_and_erlang_indentation_state() {
    for (mode, opening, closing, body, style) in [
        (LegacyMode::Haskell, "{- {-", "-} -}", "body", "comment"),
        (LegacyMode::OCaml, "{|", "|}", "body", "string"),
        (LegacyMode::FSharp, "(* (*", "*) *)", "body", "comment"),
        (LegacyMode::Sml, "\"unfinished", "\"", "body", "string"),
        (
            LegacyMode::Erlang,
            "f() -> 'unfinished",
            "'",
            "body",
            "atom",
        ),
    ] {
        let mut state = mode.start_state(2);
        scan(mode, &mut state, opening);
        let mut fork = mode.copy_state(&state);
        let expected = [Some(style.to_owned())];
        assert_eq!(scan(mode, &mut state, closing), expected);
        assert_eq!(scan(mode, &mut fork, body), expected);
        assert_eq!(scan(mode, &mut fork, closing), expected);
    }

    let mode = LegacyMode::Erlang;
    let mut state = mode.start_state(2);
    scan(mode, &mut state, "f() ->");
    let mut fork = mode.copy_state(&state);
    scan(mode, &mut state, "  done.");
    assert_eq!(mode.indent(&state, "", 2), Some(0));
    assert_eq!(mode.indent(&fork, "", 2), Some(2));
    scan(mode, &mut fork, "  [one,");
    assert_eq!(mode.indent(&fork, "]", 2), Some(2));
}

#[test]
fn keeps_non_bmp_characters_on_scalar_boundaries() {
    let settings = StreamSettings::default();
    let mode = LegacyMode::Erlang;
    let tokens = Tokenizer::new(&mode, "$🙂 é🙂", settings.tab_size, settings.indent_unit)
        .map(|token| {
            let token = token.unwrap();
            (token.text, token.style)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tokens,
        [
            ("$🙂", Some("number".into())),
            (" ", None),
            ("é", Some("atom".into())),
            ("🙂", None)
        ]
    );
}

fn scan(mode: LegacyMode, state: &mut LegacyState, line: &str) -> Vec<Option<String>> {
    let settings = StreamSettings::default();
    let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
    let mut styles = Vec::new();
    while !stream.eol() {
        stream.start_token();
        styles.push(
            mode.token(&mut stream, state)
                .map(|style| style.into_owned()),
        );
        assert!(stream.pos() > stream.start());
    }
    styles
}
