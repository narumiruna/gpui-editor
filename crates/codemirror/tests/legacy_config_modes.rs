//! The fixture uses the unchanged npm exports from `@codemirror/legacy-modes`
//! 6.5.5 and `StringStream` from `@codemirror/streamparser` 6.0.0. Both tarballs
//! match the SHA-512 digests in `docs/codemirror-upstream.json`. The oracle ran
//! outside the repository. Only Rust runs these tests.
//!
//! Each row records raw token styles and UTF-8 byte ranges for one logical
//! line. Line endings have no tokens. Matching styles within one surrogate
//! pair form one scalar token. Indents were read before tokenizing each line.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_config_modes.json");

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_configuration_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in cases {
        let export = case["export"].as_str().unwrap();
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = from_export(case["module"].as_str().unwrap(), export).unwrap();
        assert_eq!(json!(mode.name()), case["name"]);
        assert_eq!(json!(mode.merge_tokens()), case["mergeTokens"]);

        let mut expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        for token in Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit) {
            let token = token.unwrap();
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                Some(&json!([token.range.start, token.range.end, token.style])),
                expected.next(),
                "{export} ({label}) at {:?}: {:?}",
                token.range,
                token.text,
            );
        }
        assert!(
            expected.next().is_none(),
            "{export} ({label}): missing tokens"
        );

        let language = mode.language();
        assert_eq!(language.name(), mode.name().unwrap());
        let doc = language.parse(Text::new(source), settings).unwrap();
        let actual_indents = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(pos, _)| pos + 1))
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        let expected_indents = case["indents"]
            .as_array()
            .unwrap()
            .iter()
            // The Rust API uses unsigned columns. Clamp a stray closing brace to zero.
            .map(|value| {
                value
                    .as_i64()
                    .map(|indent| usize::try_from(indent).unwrap_or(0))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual_indents, expected_indents,
            "{export} ({label}) indentation"
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
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            case["autocomplete"]
        );
        let indent_input = data.get::<fn(&str) -> bool>("indentOnInput");
        if let Some(probes) = case["indentOnInput"].as_array() {
            let indent_input = indent_input.unwrap();
            for probe in probes {
                let input = probe[0].as_str().unwrap();
                assert_eq!(
                    json!(indent_input(input)),
                    probe[1],
                    "indentOnInput: {input:?}"
                );
            }
        } else {
            assert!(indent_input.is_none());
        }
    }
}

#[test]
fn resolves_only_the_published_spelling() {
    for (module, export, mode) in [
        ("nginx", "nginx", LegacyMode::Nginx),
        ("mbox", "mbox", LegacyMode::Mbox),
        ("asterisk", "asterisk", LegacyMode::Asterisk),
        ("rpm", "rpmChanges", LegacyMode::RpmChanges),
        ("rpm", "rpmSpec", LegacyMode::RpmSpec),
    ] {
        assert_eq!(from_export(module, export), Some(mode));
    }
    for (module, export) in [
        ("apache", "apache"),
        ("nginx", "Nginx"),
        ("mbox", "Mbox"),
        ("asterisk", "Asterisk"),
        ("rpm", "rpmchanges"),
        ("rpm", "rpmspec"),
        ("rpm", "rpm"),
    ] {
        assert_eq!(from_export(module, export), None);
    }
}

#[test]
fn replays_edits_through_stateful_lines() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Nginx,
            "server {\n  /* open\n  */ root /x;\n}\n",
            "*/",
            "still open\n  */",
        ),
        (
            LegacyMode::Mbox,
            "To: name\n\nX-Local: <name>\nbody",
            "\nX-Local",
            " continuation\nX-Local",
        ),
        (
            LegacyMode::Asterisk,
            ";-- open\n--;\nsame => n,NoOp()\n",
            "--;",
            "still open\n--;",
        ),
        (
            LegacyMode::RpmSpec,
            "%ifarch x86_64\n42 >= 6\n",
            "x86_64",
            "x86_64 ",
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

#[test]
fn nginx_copies_its_indentation_stack() {
    let mode = LegacyMode::Nginx;
    let settings = StreamSettings {
        indent_unit: 4,
        ..StreamSettings::default()
    };
    let mut state = mode.start_state(settings.indent_unit);
    let mut stream = StringStream::new("{", settings.tab_size, settings.indent_unit);
    assert_eq!(mode.token(&mut stream, &mut state), None);
    let mut copy = mode.copy_state(&state);
    let mut stream = StringStream::new("}", settings.tab_size, settings.indent_unit);
    assert_eq!(mode.token(&mut stream, &mut copy), None);
    assert_eq!(mode.indent(&state, "next", settings.indent_unit), Some(4));
    assert_eq!(mode.indent(&copy, "next", settings.indent_unit), Some(0));
}
