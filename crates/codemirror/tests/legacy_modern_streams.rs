//! Oracle: unchanged ESM modes from `@codemirror/legacy-modes` 6.5.5 with
//! `@codemirror/streamparser` 6.0.0 `StringStream`. Both tarballs match the
//! SHA-512 SRI values in `docs/codemirror-upstream.json`. The oracle ran outside
//! the repository and queried indentation before tokenizing each line. The
//! fixture records raw token styles and UTF-8 byte ranges, excluding line endings.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const MODES: [(&str, LegacyMode); 3] = [
    ("go", LegacyMode::Go),
    ("rust", LegacyMode::Rust),
    ("jinja2", LegacyMode::Jinja2),
];

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/legacy_modern_streams.json")).unwrap()
}

fn from_export(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("@codemirror/legacy-modes/mode/{module}"), export)
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1))
}

#[test]
fn resolves_only_the_published_stream_exports() {
    let metadata = &oracle()["metadata"];
    for (name, mode) in MODES {
        assert_eq!(metadata[name]["exports"], json!([name]));
        assert_eq!(from_export(name, name), Some(mode));
        assert_eq!(from_export(name, &name.to_ascii_uppercase()), None);
        assert_eq!(from_export(&name.to_ascii_uppercase(), name), None);
        assert_eq!(from_export(&format!("{name}.js"), name), None);
    }
    assert_eq!(from_export("jinja2", "jinja"), None);
    for (package, export) in [
        ("lang-go", "go"),
        ("lang-rust", "rust"),
        ("lang-jinja", "jinja"),
    ] {
        assert_eq!(
            LegacyMode::from_export(&format!("@codemirror/{package}"), export),
            None
        );
    }
}

#[test]
fn matches_pinned_tokens_and_indentation() {
    for case in oracle()["cases"].as_array().unwrap() {
        let name = case["mode"].as_str().unwrap();
        let title = case["title"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = from_export(name, name).unwrap();
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
            let token = token.unwrap_or_else(|error| panic!("{name}/{title}: {error}"));
            let actual = json!([token.range.start, token.range.end, token.style]);
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                Some(&actual),
                expected.next(),
                "{name}/{title} at {:?}: {:?}",
                token.range,
                token.text
            );
        }
        assert_eq!(expected.next(), None, "{name}/{title}: missing tokens");

        let document = mode.language().parse(Text::new(source), settings).unwrap();
        let actual = line_starts(source)
            .map(|position| document.indentation(position, Default::default()).unwrap())
            .collect::<Vec<_>>();
        let expected = case["indents"]
            .as_array()
            .unwrap()
            .iter()
            // Go's root can yield a negative indent. The native API clamps it.
            .map(|value| value.as_i64().map(|n| usize::try_from(n).unwrap_or(0)))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{name}/{title}: indentation");
    }
}

#[test]
fn matches_pinned_language_data() {
    let metadata = &oracle()["metadata"];
    for (name, mode) in MODES {
        let expected = &metadata[name];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert_eq!(json!(mode.language().name()), expected["name"]);
        assert_eq!(json!(mode.merge_tokens()), expected["mergeTokens"]);
        let doc = mode
            .language()
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        let expected_comments = &expected["commentTokens"];
        let (open, close) = comments.block.unwrap();
        assert_eq!(json!(comments.line), expected_comments["line"]);
        assert_eq!(json!(open), expected_comments["block"]["open"]);
        assert_eq!(json!(close), expected_comments["block"]["close"]);
        assert_eq!(json!(data.get::<&str>("name")), expected["dataName"]);
        assert_eq!(
            json!(data.get::<&[&str]>("dontIndentStates")),
            expected["dontIndentStates"]
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput");
        if let Some(probes) = expected["indentOnInput"].as_array() {
            let trigger = trigger.unwrap();
            for probe in probes {
                let input = probe[0].as_str().unwrap();
                assert_eq!(json!(trigger(input)), probe[1], "{name}: {input:?}");
            }
        } else {
            assert!(trigger.is_none(), "{name}");
        }
    }
}

#[test]
fn replays_edits_from_multiline_state() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Go,
            "func f() {\n  print(`raw\nold text`)\n}\n",
            "old text",
            "new\n text",
        ),
        (
            LegacyMode::Rust,
            "fn f() {\n  call([\n    /* open\nold */ 7,\n  ])\n}\n",
            "old */",
            "new\n text */",
        ),
        (
            LegacyMode::Jinja2,
            "#for x in call([\n'old',\n])\n{{ x }}\n#endfor\n",
            "'old'",
            "'new\n text'",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let start = source.find(before).unwrap();
        let change = ChangeSet::single(Change::new(start..start + before.len(), after)).unwrap();
        replay.apply_changes(&change).unwrap();
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
        for position in line_starts(&updated) {
            assert_eq!(
                replay.indentation(position, Default::default()),
                fresh.indentation(position, Default::default()),
                "{mode:?} at {position}"
            );
        }
    }
}

#[test]
fn rust_pending_definition_survives_independent_state_copies() {
    let mode = LegacyMode::Rust;
    let settings = StreamSettings::default();
    let mut state = mode.start_state(settings.indent_unit);
    let mut stream =
        StringStream::new("let mut\u{a0}name", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut stream, &mut state).as_deref(),
        Some("keyword")
    );
    assert_eq!(stream.current(), "let mut");
    let checkpoint = stream.pos();
    let mut copy = mode.copy_state(&state);
    for snapshot in [&mut state, &mut copy] {
        stream.set_pos(checkpoint).unwrap();
        stream.start_token();
        assert_eq!(mode.token(&mut stream, snapshot), None);
        assert_eq!(stream.current(), "\u{a0}");
        stream.start_token();
        assert_eq!(mode.token(&mut stream, snapshot).as_deref(), Some("def"));
        assert_eq!(stream.current(), "name");
        assert!(stream.eol());
    }
}

#[test]
fn preserves_scalar_boundaries_and_advances_through_line_separators() {
    let settings = StreamSettings::default();
    for (mode, source) in [
        (
            LegacyMode::Rust,
            "/* x\u{2028} y */ \"abc\\\u{2029}next\" '😺'",
        ),
        (LegacyMode::Jinja2, "😺{{ '😺' }}"),
    ] {
        let actual = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit)
            .map(|token| token.unwrap().text)
            .collect::<String>();
        assert_eq!(actual, source, "{mode:?}");
    }
}
