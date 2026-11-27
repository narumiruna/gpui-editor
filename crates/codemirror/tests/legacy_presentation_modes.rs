//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs match
//! the SHA-512 digests in `docs/codemirror-upstream.json`. The oracle ran outside
//! the repository. These tests run only Rust.
//!
//! Each row records raw token styles and UTF-8 byte ranges for one logical line.
//! Adjacent slices of one surrogate pair form one scalar token. Each fixture
//! document starts with a fresh upstream parser and state.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_presentation_modes.json");

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_presentation_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in cases {
        let module = case["module"].as_str().unwrap();
        let export = case["id"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = from_export(module, export).unwrap();
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
        assert_eq!(language.name(), name);
        let doc = language.parse(Text::new(source), settings).unwrap();
        let positions =
            std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1));
        let actual_indents = positions
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        let expected_indents = case["indents"]
            .as_array()
            .unwrap()
            .iter()
            // The Rust API uses unsigned columns. Clamp negative VBScript indents.
            .map(|value| value.as_i64().map(|n| usize::try_from(n).unwrap_or(0)))
            .collect::<Vec<_>>();
        assert_eq!(
            actual_indents, expected_indents,
            "{export} ({label}) indents"
        );

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens");
        assert_eq!(
            json!(comments.map(|tokens| json!({
                "line": tokens.line,
                "block": tokens.block.map(|(open, close)| json!({ "open": open, "close": close })),
            }))),
            case["commentTokens"],
        );
    }
}

#[test]
fn resolves_only_the_published_module_and_export_pairs() {
    for (module, export, wrong, mode) in [
        ("mscgen", "mscgen", "Mscgen", LegacyMode::Mscgen),
        ("mscgen", "msgenny", "Msgenny", LegacyMode::Msgenny),
        ("mscgen", "xu", "Xu", LegacyMode::Xu),
        ("vbscript", "vbScript", "vbscript", LegacyMode::VbScript),
        (
            "vbscript",
            "vbScriptASP",
            "vbScriptAsp",
            LegacyMode::VbScriptAsp,
        ),
        ("velocity", "velocity", "Velocity", LegacyMode::Velocity),
    ] {
        assert_eq!(from_export(module, export), Some(mode));
        assert_eq!(from_export(module, wrong), None);
        assert_eq!(from_export(&format!("{module}.js"), export), None);
        assert_eq!(LegacyMode::from_export(module, export), None);
    }
    assert_eq!(from_export("msgenny", "msgenny"), None);
    assert_eq!(from_export("xu", "xu"), None);
    assert_eq!(from_export("vbscriptASP", "vbScriptASP"), None);
}

#[test]
fn isolates_msc_attribute_lists_between_scans_and_state_copies() {
    let mode = LegacyMode::Xu;
    let settings = StreamSettings::default();
    let mut state = mode.start_state(settings.indent_unit);
    let next = |state: &mut _, line| {
        let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
        mode.token(&mut stream, state)
    };
    assert_eq!(next(&mut state, "[").as_deref(), Some("bracket"));
    let mut copy = mode.copy_state(&state);
    assert_eq!(next(&mut copy, "title").as_deref(), Some("attribute"));
    assert_eq!(next(&mut copy, "]").as_deref(), Some("bracket"));
    assert_eq!(next(&mut state, "title").as_deref(), Some("attribute"));
    assert_eq!(next(&mut copy, "title"), None);
    let mut fresh = mode.start_state(settings.indent_unit);
    assert_eq!(next(&mut fresh, "title"), None);
}

#[test]
fn replays_edits_through_stateful_lines() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Xu,
            "msc {\n[label=\"one\",\n title=\"two\"]\n}\n",
            "[",
            "",
        ),
        (
            LegacyMode::VbScriptAsp,
            "If True Then\n  Response.Write \"hello\"\nEnd If\n",
            "If True Then",
            "While True\n  If True Then",
        ),
        (
            LegacyMode::Velocity,
            "#* note\n  continued\n  *# $name\n",
            "note",
            "note *#",
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
