//! Oracle: unmodified `@codemirror/legacy-modes` 6.5.5 ESM exports and
//! `@codemirror/streamparser` 6.0.0 `StringStream`, both verified against
//! `docs/codemirror-upstream.json`. The external harness retries stalled tokens
//! and records UTF-8 byte offsets without splitting Unicode scalars.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{CSS_KEYWORDS, LegacyMode},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const MODES: [(&str, &str, LegacyMode); 4] = [
    ("sass", "sass", LegacyMode::Sass),
    ("stex", "stex", LegacyMode::Stex),
    ("stex", "stexMath", LegacyMode::StexMath),
    ("stylus", "stylus", LegacyMode::Stylus),
];

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/legacy_typeset_modes.json")).unwrap()
}

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("@codemirror/legacy-modes/mode/{module}"), export)
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}

#[test]
fn registers_only_exact_export_names() {
    for (module, exports) in oracle()["exports"].as_object().unwrap() {
        for export in exports.as_array().unwrap() {
            let export = export.as_str().unwrap();
            let mode = MODES
                .iter()
                .find(|(m, e, _)| *m == module && *e == export)
                .unwrap()
                .2;
            assert_eq!(from_export(module, export), Some(mode));
        }
    }
    for (module, wrong) in [
        ("sass", "Sass"),
        ("sass", "scss"),
        ("css", "sass"),
        ("stex", "stexmath"),
        ("stex", "StexMath"),
        ("stex", "mkStex"),
        ("stexMath", "stexMath"),
        ("stylus", "Stylus"),
    ] {
        assert_eq!(from_export(module, wrong), None, "{module}#{wrong}");
    }
}

#[test]
fn matches_upstream_tokens_and_indentation() {
    for case in oracle()["cases"].as_array().unwrap() {
        let module = case["module"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let title = case["title"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = from_export(module, export).unwrap();
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
            let token = token.unwrap_or_else(|error| panic!("{export}/{title}: {error}"));
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
        for (i, start) in line_starts(source).enumerate() {
            let indent = doc.indentation(start, Default::default()).unwrap();
            assert_eq!(
                json!(indent),
                case["indents"][i],
                "{export}/{title}: line {i}"
            );
        }
    }
}

#[test]
fn matches_upstream_language_data() {
    let oracle = oracle();
    for (_, export, mode) in MODES {
        let expected = &oracle["metadata"][export];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert_eq!(json!(mode.merge_tokens()), expected["mergeTokens"]);
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
        let autocomplete = data.get::<&[&str]>("autocomplete").copied();
        if expected["autocomplete"] == "css" {
            assert_eq!(autocomplete, Some(CSS_KEYWORDS.all));
        } else {
            assert_eq!(json!(autocomplete), expected["autocomplete"], "{export}");
        }
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput");
        if let Some(probes) = expected["indentOnInput"].as_array() {
            let trigger = trigger.unwrap();
            for probe in probes {
                let input = probe[0].as_str().unwrap();
                assert_eq!(json!(trigger(input)), probe[1], "{export}: {input:?}");
            }
        } else {
            assert!(trigger.is_none(), "{export}");
        }
    }
}

#[test]
fn replays_edits_from_open_typeset_states() {
    let settings = StreamSettings::default();
    for (mode, source, old, new) in [
        (
            LegacyMode::Sass,
            ".part\n  content: \"a#{\n$old\n}c\"\n  color: red",
            "$old",
            "$new\n",
        ),
        (
            LegacyMode::Stex,
            "\\importmodule[site\nold]{package}\ntext",
            "old",
            "root\nitem",
        ),
        (
            LegacyMode::StexMath,
            "a + b^2\nold + \\sqrt{2}\n\nplain",
            "old",
            ".23\n x",
        ),
        (
            LegacyMode::Stylus,
            ".main\n  background url(\nold/path\n)\n  color red",
            "old/path",
            "new\n/path",
        ),
    ] {
        let language = mode.language();
        let mut document = language.parse(Text::new(source), settings).unwrap();
        let start = source.find(old).unwrap();
        let change = ChangeSet::single(Change::new(start..start + old.len(), new)).unwrap();
        document.apply_changes(&change).unwrap();
        let updated = source.replacen(old, new, 1);
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            document
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?}"
        );
        for start in line_starts(&updated) {
            assert_eq!(
                document.indentation(start, Default::default()),
                fresh.indentation(start, Default::default()),
                "{mode:?}: {start}"
            );
        }
    }
}

#[test]
fn copied_stex_command_state_keeps_its_argument_index() {
    let mode = LegacyMode::Stex;
    let settings = StreamSettings::default();
    let mut prefix = Tokenizer::new(
        &mode,
        "\\importmodule[old",
        settings.tab_size,
        settings.indent_unit,
    );
    for token in &mut prefix {
        token.unwrap();
    }
    let mut fork = mode.copy_state(prefix.state());
    let mut other = mode.copy_state(prefix.state());
    let mut stream = StringStream::new("]{new}", settings.tab_size, settings.indent_unit);
    for (text, style) in [
        ("]", "bracket"),
        ("{", "bracket"),
        ("new", "builtin"),
        ("}", "bracket"),
    ] {
        stream.start_token();
        assert_eq!(mode.token(&mut stream, &mut fork).as_deref(), Some(style));
        assert_eq!(stream.current(), text);
    }
    let mut stream = StringStream::new("next", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut stream, &mut other).as_deref(),
        Some("string")
    );
}
