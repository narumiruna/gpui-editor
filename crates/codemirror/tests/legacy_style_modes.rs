//! Oracle: unchanged `@codemirror/legacy-modes` 6.5.5 ESM exports and
//! `@codemirror/streamparser` 6.0.0 `StringStream`. Both tarballs match
//! their SHA-512 SRI values in `docs/codemirror-upstream.json`. The scratch
//! harness called `indent` before each line and `token` until it advanced.
//! Raw token positions are UTF-8 bytes; LF and CRLF produce no tokens.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{CSS_KEYWORDS, LegacyMode},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const MODULE: &str = "@codemirror/legacy-modes/mode/css";
const MODES: [(&str, LegacyMode); 4] = [
    ("css", LegacyMode::Css),
    ("sCSS", LegacyMode::Scss),
    ("less", LegacyMode::Less),
    ("gss", LegacyMode::Gss),
];

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/legacy_style_modes.json")).unwrap()
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}

#[test]
fn registers_concrete_exports_by_exact_name() {
    let oracle = oracle();
    for export in oracle["exports"].as_array().unwrap() {
        let export = export.as_str().unwrap();
        let mode = MODES.iter().find(|(name, _)| *name == export).map(|m| m.1);
        assert_eq!(LegacyMode::from_export(MODULE, export), mode, "{export}");
    }
    for wrong in ["CSS", "Scss", "scss", "SASS", "sass", "GSS"] {
        assert_eq!(LegacyMode::from_export(MODULE, wrong), None);
    }
}

#[test]
fn matches_pinned_style_stream_tokens_and_indentation() {
    for case in oracle()["cases"].as_array().unwrap() {
        let export = case["export"].as_str().unwrap();
        let title = case["title"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = LegacyMode::from_export(MODULE, export).unwrap();
        let settings = StreamSettings {
            indent_unit: case["unit"].as_u64().unwrap().try_into().unwrap(),
            tab_size: NonZeroUsize::new(case["tabSize"].as_u64().unwrap().try_into().unwrap())
                .unwrap(),
        };

        let mut expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        for token in Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit) {
            let token = token.unwrap();
            let actual = json!([token.range.start, token.range.end, token.style]);
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                Some(&actual),
                expected.next(),
                "{export}/{title}: {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert_eq!(expected.next(), None, "{export}/{title}: missing tokens");

        let doc = mode.language().parse(Text::new(source), settings).unwrap();
        let indents = line_starts(source)
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{export}/{title}: indent");
    }
}

#[test]
fn matches_pinned_metadata_and_all_five_keyword_lists() {
    let words: Value =
        serde_json::from_str(include_str!("fixtures/legacy_style_keywords.json")).unwrap();
    let metadata = &oracle()["metadata"];
    for (key, actual) in [
        ("properties", CSS_KEYWORDS.properties),
        ("colors", CSS_KEYWORDS.colors),
        ("fonts", CSS_KEYWORDS.fonts),
        ("values", CSS_KEYWORDS.values),
        ("all", CSS_KEYWORDS.all),
    ] {
        assert_eq!(json!(actual), words[key], "keywords.{key}");
    }
    for (export, mode) in MODES {
        let expected = &metadata[export];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert_eq!(json!(mode.merge_tokens()), expected["mergeTokens"]);
        let language = mode.language();
        assert_eq!(json!(language.name()), expected["name"]);
        let doc = language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        assert_eq!(
            data.get::<&[&str]>("autocomplete").copied(),
            Some(CSS_KEYWORDS.all),
            "{export}: autocomplete"
        );
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        let (open, close) = comments.block.unwrap();
        assert_eq!(json!(comments.line), expected["commentTokens"]["line"]);
        assert_eq!(
            json!({"open": open, "close": close}),
            expected["commentTokens"]["block"]
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        for probe in expected["indentOnInput"].as_array().unwrap() {
            let input = probe[0].as_str().unwrap();
            assert_eq!(
                json!(trigger(input)),
                probe[1],
                "{export} trigger: {input:?}"
            );
        }
    }
}

#[test]
fn replays_edits_from_open_style_mode_state() {
    let settings = StreamSettings::default();
    for (mode, source, old, new) in [
        (
            LegacyMode::Css,
            "a {\n  background: url(\nold/path\n);\n}",
            "old/path",
            "new\n/path",
        ),
        (
            LegacyMode::Scss,
            ".a {\n  color: #{\n$old\n};\n}",
            "$old",
            "call($new)\n",
        ),
        (
            LegacyMode::Less,
            "@media print {\n  a { /* open\nold */\n    color: @red;\n  }\n}",
            "old */",
            "new\n text */",
        ),
        (
            LegacyMode::Gss,
            "@component {\n.a {\n  color: old;\n}\n}",
            "  color: old;",
            "  color: new;\n  display: block;",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let start = source.find(old).unwrap();
        let change = ChangeSet::single(Change::new(start..start + old.len(), new)).unwrap();
        replay.apply_changes(&change).unwrap();
        let updated = source.replacen(old, new, 1);
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

#[test]
fn copied_url_state_retries_without_consuming_the_closing_paren() {
    let mode = LegacyMode::Css;
    let settings = StreamSettings::default();
    let mut prefix = Tokenizer::new(
        &mode,
        "a {\n  background: url(https://",
        settings.tab_size,
        settings.indent_unit,
    );
    for token in prefix.by_ref() {
        token.unwrap();
    }
    let mut copy = mode.copy_state(prefix.state());
    let mut stream = StringStream::new("); }", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut stream, &mut copy).as_deref(),
        Some("string")
    );
    assert_eq!(stream.pos(), 0);
    assert_eq!(mode.token(&mut stream, &mut copy), None);
    assert_eq!(stream.current(), ")");

    let mut original = mode.copy_state(prefix.state());
    let mut stream = StringStream::new("host", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut stream, &mut original).as_deref(),
        Some("string")
    );
    assert_eq!(stream.current(), "host");
}
