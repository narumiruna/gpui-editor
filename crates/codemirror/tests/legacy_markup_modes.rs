//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs match
//! the SHA-512 pins in `docs/codemirror-upstream.json`. The oracle ran outside
//! this repository. These tests run only Rust.
//!
//! Expected tokens have raw styles and UTF-8 byte ranges. Split surrogate
//! pairs with matching styles form one token. Indents are from before each line.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyMode, LegacyState},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_markup_modes.json");

#[test]
fn matches_the_pinned_markup_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = LegacyMode::from_export(&format!("{PREFIX}{name}"), export).unwrap();
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
            std::iter::once(0).chain(source.match_indices('\n').map(|(position, _)| position + 1));
        let indents = positions
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{name} ({label}) indents");

        let data = doc.language_data_at(0).unwrap();
        assert_eq!(case["languageData"], json!({}));
        assert!(
            data.get::<CommentTokens<'static>>("commentTokens")
                .is_none()
        );
        assert!(data.get::<fn(&str) -> bool>("indentOnInput").is_none());
        assert!(data.get::<&[&str]>("autocomplete").is_none());
    }
}

#[test]
fn resolves_only_the_published_spelling() {
    for (name, export, wrong, mode) in [
        ("gherkin", "gherkin", "Gherkin", LegacyMode::Gherkin),
        ("troff", "troff", "Troff", LegacyMode::Troff),
        ("textile", "textile", "Textile", LegacyMode::Textile),
        (
            "tiddlywiki",
            "tiddlyWiki",
            "tiddlywiki",
            LegacyMode::TiddlyWiki,
        ),
        ("tiki", "tiki", "Tiki", LegacyMode::Tiki),
    ] {
        assert_eq!(
            LegacyMode::from_export(&format!("{PREFIX}{name}"), export),
            Some(mode)
        );
        assert_eq!(
            LegacyMode::from_export(&format!("{PREFIX}{name}"), wrong),
            None
        );
        assert_eq!(
            LegacyMode::from_export(&format!("{PREFIX}{name}.js"), export),
            None
        );
        assert_eq!(LegacyMode::from_export(name, export), None);
    }
    assert_eq!(
        LegacyMode::from_export(&format!("{PREFIX}tiddlyWiki"), "tiddlyWiki"),
        None
    );
}

#[test]
fn tiki_copy_preserves_a_quoted_plugin_attribute() {
    let mode = LegacyMode::Tiki;
    let settings = StreamSettings::default();
    let mut state = mode.start_state(settings.indent_unit);
    read_line(&mut state, "{P(value='open", settings);
    let mut copy = mode.copy_state(&state);
    read_line(&mut copy, "  close'", settings);
    assert_eq!(read_line(&mut state, ")}", settings), ["string"]);
    assert_eq!(read_line(&mut copy, ")}", settings), ["bracket", "tag"]);
    assert_eq!(mode.indent(&state, "{/P}", settings.indent_unit), Some(0));
    assert_eq!(mode.indent(&copy, "{/P}", settings.indent_unit), Some(0));
}

fn read_line(state: &mut LegacyState, line: &str, settings: StreamSettings) -> Vec<String> {
    let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
    let mut styles = Vec::new();
    while !stream.eol() {
        stream.start_token();
        if let Some(style) = LegacyMode::Tiki.token(&mut stream, state) {
            styles.push(style.into_owned());
        }
        assert!(stream.pos() > stream.start());
    }
    styles
}

#[test]
fn replays_edits_through_stateful_lines() {
    for (mode, source, before, after) in [
        (
            LegacyMode::Gherkin,
            "Feature: test\nScenario: case\nGiven input\n  \"\"\"\ntext\n  \"\"\"\nWhen more\n",
            "text",
            "still inside\n  \"\"\"\nWhen text",
        ),
        (
            LegacyMode::Textile,
            "h3.. Heading\n\ncontinued *bold*\n\np. Plain\n",
            "continued",
            "bq.. new quote\n\ncontinued",
        ),
        (
            LegacyMode::TiddlyWiki,
            "{{{\ncontent\n}}}\ntext [[link]]\n",
            "content",
            "changed\n}}}\n<<today>>",
        ),
        (
            LegacyMode::Tiki,
            "{P(value='open\n\tkeep\n  close'\n)}\n{/P}\ntext\n",
            "  close'",
            "    more\n close'",
        ),
    ] {
        let settings = StreamSettings::default();
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
