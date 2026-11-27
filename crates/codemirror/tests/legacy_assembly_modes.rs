//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs match
//! the SHA-512 digests in `docs/codemirror-upstream.json`. The oracle ran outside
//! the repository. These tests run only Rust.
//!
//! Token rows contain raw styles and UTF-8 byte ranges for each logical line.
//! Two halves of a surrogate pair with the same style form one scalar token.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_assembly_modes.json");

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_assembly_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in cases {
        let module = case["module"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = from_export(module, export).unwrap();
        assert_eq!(mode.name(), Some(module));
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
                .unwrap_or_else(|| panic!("{export} ({label}): missing token {expected}"))
                .unwrap();
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                expected,
                "{export} ({label}) at {:?}: {:?}",
                token.range,
                token.text,
            );
        }
        assert!(actual.next().is_none(), "{export} ({label}): extra tokens");

        let language = mode.language();
        assert_eq!(language.name(), module);
        let doc = language.parse(Text::new(source), settings).unwrap();
        let positions =
            std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1));
        let indents = positions
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{export} ({label})");

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens");
        assert_eq!(
            json!(comments.map(|tokens| {
                json!({
                    "line": tokens.line,
                    "block": tokens.block.map(|(open, close)| json!({"open": open, "close": close})),
                })
            })),
            case["commentTokens"],
            "{export} ({label})",
        );
    }
}

#[test]
fn resolves_only_the_published_spellings() {
    for (module, export, wrong, mode) in [
        ("gas", "gas", "Gas", LegacyMode::Gas),
        ("gas", "gasArm", "gasarm", LegacyMode::GasArm),
        ("z80", "z80", "Z80", LegacyMode::Z80),
        ("z80", "ez80", "eZ80", LegacyMode::Ez80),
    ] {
        assert_eq!(from_export(module, export), Some(mode));
        assert_eq!(from_export(module, wrong), None);
        assert_eq!(from_export(&module.to_ascii_uppercase(), export), None);
        assert_eq!(from_export(&format!("{module}.js"), export), None);
        assert_eq!(LegacyMode::from_export(module, export), None);
    }
    assert_eq!(from_export("gas", "mkGas"), None);
    assert_eq!(from_export("z80", "mkZ80"), None);
}

#[test]
fn replays_edits_that_close_a_multiline_comment() {
    let settings = StreamSettings::default();
    let source = "/* note\r\n\r\n  another note\r\n.text\r\nAX R0\r\n";
    let start = source.find(".text").unwrap();
    let replacement = "*/\r\n";
    let change = ChangeSet::single(Change::new(start..start, replacement)).unwrap();
    let updated = format!("{}{replacement}{}", &source[..start], &source[start..]);
    for mode in [LegacyMode::Gas, LegacyMode::GasArm] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        replay.apply_changes(&change).unwrap();
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            replay
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?} highlights",
        );
    }
}
