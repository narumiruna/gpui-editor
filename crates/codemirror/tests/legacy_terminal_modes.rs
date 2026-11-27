//! The oracle used the unchanged `@codemirror/legacy-modes` 6.5.5 ESM exports
//! and `StringStream` from `@codemirror/streamparser` 6.0.0 outside the repository.
//! Both tarballs matched the SHA-512 SRI in `docs/codemirror-upstream.json`.
//! The fixture records raw styles and UTF-8 byte ranges without line endings.
//! Surrogate halves with the same style form one scalar token. Indentation
//! uses four-column tabs, an indent unit of two, and clamps negative values
//! to zero. Final-state probes use indent units of two and three.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_terminal_modes.json");
const EXPORTS: [(&str, LegacyMode); 3] = [
    ("mirc", LegacyMode::Mirc),
    ("tcl", LegacyMode::Tcl),
    ("vb", LegacyMode::Vb),
];

#[test]
fn resolves_only_published_exports() {
    for (name, mode) in EXPORTS {
        let module = format!("{PREFIX}{name}");
        assert_eq!(LegacyMode::from_export(&module, name), Some(mode));
        assert_eq!(LegacyMode::from_export(&module, &name.to_uppercase()), None);
        assert_eq!(LegacyMode::from_export(&format!("{module}.js"), name), None);
        for (other, _) in EXPORTS.into_iter().filter(|(other, _)| *other != name) {
            assert_eq!(LegacyMode::from_export(&module, other), None);
        }
        assert_eq!(mode.language().name(), name);
        assert!(mode.merge_tokens());
    }
}

#[test]
fn matches_pinned_tokens_and_indentation() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (name, mode) in EXPORTS {
        let expected_mode = &oracle["modes"][name];
        assert_eq!(json!(mode.name()), expected_mode["name"]);
        for case in expected_mode["cases"].as_array().unwrap() {
            let source = case["source"].as_str().unwrap();
            let label = case["label"].as_str().unwrap();
            let mut expected = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|line| line.as_array().unwrap());
            let mut tokenizer =
                Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
            for token in tokenizer.by_ref() {
                let token = token.unwrap_or_else(|err| panic!("{name}/{label}: {err}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                assert_eq!(
                    Some(&json!([token.range.start, token.range.end, token.style])),
                    expected.next(),
                    "{name}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(expected.next().is_none(), "{name}/{label}: missing tokens");

            let last_indents = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    [2, 3].map(|unit| mode.indent(tokenizer.state(), input.as_str().unwrap(), unit))
                })
                .collect::<Vec<_>>();
            assert_eq!(json!(last_indents), case["lastIndents"], "{name}/{label}");

            let doc = mode.language().parse(Text::new(source), settings).unwrap();
            let indents = line_starts(source)
                .map(|position| doc.indentation(position, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(json!(indents), case["indents"], "{name}/{label}");
        }
    }
}

#[test]
fn matches_pinned_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (name, mode) in EXPORTS {
        let expected = &oracle["modes"][name];
        let doc = mode
            .language()
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data
            .get::<CommentTokens<'static>>("commentTokens")
            .map(|value| {
                assert_eq!(value.block, None, "{name}");
                json!({"line": value.line})
            });
        assert_eq!(json!(comments), expected["comments"], "{name}");
        assert_eq!(
            json!(data.get::<&[&str]>("closeBrackets")),
            expected["closeBrackets"],
            "{name}"
        );
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            expected["autocomplete"],
            "{name}"
        );
        assert!(
            data.get::<fn(&str) -> bool>("indentOnInput").is_none(),
            "{name}"
        );
    }
}

#[test]
fn copies_open_comments_for_independent_scans() {
    let settings = StreamSettings::default();
    for (mode, open, close, command) in [
        (LegacyMode::Mirc, "/* open", "*/", "echo"),
        (LegacyMode::Tcl, "#* open", "*#", "puts"),
    ] {
        let scan = |line, state: &mut _| {
            let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
            let style = mode.token(&mut stream, state);
            assert!(stream.eol());
            style
        };
        let mut state = mode.start_state(settings.indent_unit);
        assert_eq!(scan(open, &mut state).as_deref(), Some("comment"));
        let mut fork = mode.copy_state(&state);
        assert_eq!(scan(close, &mut fork).as_deref(), Some("comment"));
        assert_eq!(scan(command, &mut fork).as_deref(), Some("keyword"));
        assert_eq!(scan(command, &mut state).as_deref(), Some("comment"));
    }
}

#[test]
fn replays_edits_across_state_boundaries() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Mirc,
            "alias main {\n /* open\n */ echo\n}\n",
            "*/",
            "still open\n */",
        ),
        (
            LegacyMode::Tcl,
            "set value $p(\"open\n next\")\nputs value\n",
            "next\"",
            "more\n then\"",
        ),
        (
            LegacyMode::Vb,
            "Class test\nobj.\"string\"\nSub Test()\nEnd Sub\nEnd Class\n",
            "obj.",
            "obj = ",
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
        assert_eq!(replay.text().as_str(), updated);
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            replay
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?}"
        );
        for pos in line_starts(&updated) {
            assert_eq!(
                replay.indentation(pos, Default::default()).unwrap(),
                fresh.indentation(pos, Default::default()).unwrap(),
                "{mode:?} at {pos}",
            );
        }
    }
}

#[test]
fn mirc_escape_consumes_a_whole_unicode_scalar() {
    let settings = StreamSettings::default();
    let mode = LegacyMode::Mirc;
    let tokens = Tokenizer::new(&mode, r"\🙂x", settings.tab_size, settings.indent_unit)
        .map(|result| {
            let token = result.unwrap();
            (token.text, token.style)
        })
        .collect::<Vec<_>>();
    assert_eq!(tokens, [(r"\🙂", Some("number".into())), ("x", None)]);
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(position, _)| position + 1))
}
