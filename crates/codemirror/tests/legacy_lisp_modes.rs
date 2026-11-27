//! Oracle: the unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both tarballs matched
//! `docs/codemirror-upstream.json`. The scratch runner converted ranges to
//! UTF-8 and joined surrogate halves with the same style. These tests run Rust only.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_lisp_modes.json");
const MODES: [(&str, &str, LegacyMode); 3] = [
    ("clojure", "clojure", LegacyMode::Clojure),
    ("commonlisp", "commonLisp", LegacyMode::CommonLisp),
    ("scheme", "scheme", LegacyMode::Scheme),
];

#[test]
fn recognizes_exact_exports() {
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
        ("racket", "racket"),
        ("commonlisp", "commonlisp"),
        ("commonLisp", "commonLisp"),
    ] {
        assert_eq!(
            LegacyMode::from_export(&format!("{PREFIX}{module}"), export),
            None
        );
    }
}

#[test]
fn matches_pinned_metadata() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (_, export, mode) in MODES {
        let expected = &oracle["metadata"][export];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert!(mode.merge_tokens());
        let language = mode.language();
        assert_eq!(json!(language.name()), expected["name"]);
        let doc = language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        assert_eq!(json!(comments.line), expected["commentTokens"]["line"]);
        assert_eq!(
            json!(
                comments
                    .block
                    .map(|(open, close)| json!({"open": open, "close": close}))
            ),
            expected["commentTokens"]["block"],
        );
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            expected["autocomplete"]
        );
        assert_eq!(
            json!(data.get::<&[&str]>("closeBrackets")),
            expected["closeBrackets"]["brackets"]
        );
    }
}

#[test]
fn matches_pinned_tokens_and_indentation() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let export = case["mode"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let (_, _, mode) = MODES
            .into_iter()
            .find(|&(_, name, _)| name == export)
            .unwrap();
        let source = case["source"].as_str().unwrap();
        let settings = StreamSettings {
            tab_size: NonZeroUsize::new(case["tabSize"].as_u64().unwrap().try_into().unwrap())
                .unwrap(),
            indent_unit: case["unit"].as_u64().unwrap().try_into().unwrap(),
        };
        let mut tokens = case["tokens"].as_array().unwrap().iter();
        let mut tokenizer = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        for token in tokenizer.by_ref() {
            let token = token.unwrap_or_else(|error| panic!("{export}/{label}: {error}"));
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                tokens
                    .next()
                    .unwrap_or_else(|| panic!("{export}/{label}: extra token")),
                "{export}/{label}: {:?}",
                token.text,
            );
        }
        assert!(tokens.next().is_none(), "{export}/{label}: missing tokens");
        assert_eq!(
            json!(mode.indent(tokenizer.state(), ")", settings.indent_unit + 1)),
            case["endIndent"],
            "{export}/{label}: end indent",
        );
        let doc = mode.language().parse(Text::new(source), settings).unwrap();
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1));
        let indents = starts
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            json!(indents),
            case["indents"],
            "{export}/{label}: line indents"
        );
    }
}

#[test]
fn copied_states_keep_nested_contexts_and_multiline_modes() {
    for (mode, source, closing, body_style, closing_style, indent) in [
        (
            LegacyMode::Clojure,
            "(let [\"value",
            "\"])",
            "string",
            "bracket",
            6,
        ),
        (
            LegacyMode::CommonLisp,
            "(defun x (#| note",
            "|#))",
            "comment",
            "bracket",
            10,
        ),
        (
            LegacyMode::Scheme,
            "'(list [\"value",
            "\"])",
            "atom",
            "atom",
            8,
        ),
    ] {
        let settings = StreamSettings::default();
        let mut left = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        left.by_ref().for_each(|token| {
            token.unwrap();
        });
        let mut first = mode.copy_state(left.state());
        let mut second = mode.copy_state(&first);
        let mut stream = StringStream::new(closing, settings.tab_size, settings.indent_unit);
        let mut last = None;
        while !stream.eol() {
            stream.start_token();
            last = mode.token(&mut stream, &mut first);
            assert!(stream.pos() > stream.start());
        }
        assert_eq!(last.as_deref(), Some(closing_style));
        assert_eq!(mode.indent(&first, "", settings.indent_unit), Some(0));
        assert_eq!(mode.indent(&second, "", settings.indent_unit), Some(indent));
        mode.blank_line(&mut second, settings.indent_unit);
        let mut stream = StringStream::new("body", settings.tab_size, settings.indent_unit);
        assert_eq!(
            mode.token(&mut stream, &mut second).as_deref(),
            Some(body_style)
        );
        assert!(stream.eol());
    }
}
