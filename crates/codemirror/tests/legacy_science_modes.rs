//! Oracle: unmodified ESM `mode/{r,julia,lua,perl}.js` from `@codemirror/legacy-modes`
//! 6.5.5, with `StringStream` from `@codemirror/streamparser` 6.0.0. The tarballs
//! matched their SRI digests in `docs/codemirror-upstream.json`.
//! The scratch runner used tab size 4 and indent unit 2. It recorded raw styles
//! and byte ranges for advancing tokens only, excluding line endings. The
//! runner joined surrogate halves if their styles agreed. Indents clamp at
//! zero. End-state probes also use indent unit 3 with the same state.
//! Only Rust runs these tests.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyMode, LegacyState},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_science_modes.json");

#[test]
fn matches_sri_pinned_streams_and_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (name, expected_mode) in oracle["modes"].as_object().unwrap() {
        let mode = LegacyMode::from_export(&format!("{PREFIX}{name}"), name).unwrap();
        assert_eq!(json!(mode.name()), expected_mode["name"]);
        assert!(mode.merge_tokens());
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
                    &json!([token.range.start, token.range.end, token.style]),
                    expected.next().expect("extra token"),
                    "{name}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(expected.next().is_none(), "{name}/{label}: missing tokens");
            let after = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    let input = input.as_str().unwrap();
                    [
                        mode.indent(tokenizer.state(), input, 2),
                        mode.indent(tokenizer.state(), input, 3),
                    ]
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(after),
                case["lastIndents"],
                "{name}/{label}: end state"
            );

            let language = mode.language();
            assert_eq!(language.name(), name);
            let doc = language.parse(Text::new(source), settings).unwrap();
            let line_starts =
                std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
            let indents = line_starts
                .map(|pos| doc.indentation(pos, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{name}/{label}: indentation"
            );

            let expected_data = &expected_mode["data"];
            let data = doc.language_data_at(0).unwrap();
            let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
            assert_eq!(json!(comments.line), expected_data["commentTokens"]["line"]);
            assert_eq!(
                comments
                    .block
                    .map(|(open, close)| json!({"open": open, "close": close})),
                expected_data["commentTokens"].get("block").cloned()
            );
            assert_eq!(
                json!(data.get::<&str>("wordChars")),
                expected_data["wordChars"]
            );
            assert_eq!(
                json!(data.get::<&[&str]>("closeBrackets")),
                expected_data["closeBrackets"]
            );
            assert_eq!(
                json!(data.get::<&[&str]>("autocomplete")),
                expected_data["autocomplete"]
            );
            let trigger = data.get::<fn(&str) -> bool>("indentOnInput");
            let triggers = trigger.map(|trigger| {
                oracle["triggerInputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| trigger(input.as_str().unwrap()))
                    .collect::<Vec<_>>()
            });
            assert_eq!(
                json!(triggers),
                expected_data["triggers"],
                "{name}: indentOnInput"
            );
        }
    }
}

#[test]
fn recognizes_only_the_exact_exports() {
    for (name, mode) in [
        ("r", LegacyMode::R),
        ("julia", LegacyMode::Julia),
        ("lua", LegacyMode::Lua),
        ("perl", LegacyMode::Perl),
    ] {
        let module = format!("{PREFIX}{name}");
        assert_eq!(LegacyMode::from_export(&module, name), Some(mode));
        assert_eq!(LegacyMode::from_export(&module, &name.to_uppercase()), None);
        assert_eq!(LegacyMode::from_export(&format!("{module}.js"), name), None);
    }
}

#[test]
fn copies_active_state_without_sharing_mutations() {
    let settings = StreamSettings::default();
    for (mode, opener, closer, expected, remaining_style) in [
        (
            LegacyMode::R,
            "if (x) \"body",
            "\"",
            "string",
            "string.special",
        ),
        (
            LegacyMode::Julia,
            "function f() #= #=",
            "=# =#",
            "comment",
            "comment",
        ),
        (
            LegacyMode::Lua,
            "function f() [=[",
            "]=]",
            "string",
            "string",
        ),
        (
            LegacyMode::Perl,
            "s/a/",
            "b/g",
            "string.special",
            "string.special",
        ),
    ] {
        let mut tokenizer = Tokenizer::new(&mode, opener, settings.tab_size, settings.indent_unit);
        for token in tokenizer.by_ref() {
            token.unwrap();
        }
        let mut first = mode.copy_state(tokenizer.state());
        let mut fork = mode.copy_state(&first);
        let scan = |line: &str, state: &mut LegacyState| {
            let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
            loop {
                stream.start_token();
                let style = mode.token(&mut stream, state);
                assert!(stream.pos() > stream.start());
                if stream.eol() {
                    break style.unwrap().into_owned();
                }
            }
        };
        assert_eq!(scan(closer, &mut first), expected, "{mode:?}");
        let body = if mode == LegacyMode::R {
            "\\x41"
        } else {
            "continued"
        };
        assert_eq!(scan(body, &mut fork), remaining_style, "{mode:?}");
        assert_eq!(scan(closer, &mut fork), expected, "{mode:?}");
    }
}

#[test]
fn r_uses_scalar_token_boundaries_and_columns() {
    let mode = LegacyMode::R;
    let settings = StreamSettings::default();
    let tokens = Tokenizer::new(&mode, "\"\\🙂\"", settings.tab_size, settings.indent_unit)
        .map(|token| {
            let token = token.unwrap();
            (token.text, token.style.unwrap())
        })
        .collect::<Vec<_>>();
    let styles = tokens
        .iter()
        .map(|(text, style)| (*text, style.as_ref()))
        .collect::<Vec<_>>();
    assert_eq!(
        styles,
        [
            ("\"", "string"),
            ("\\🙂", "string.special"),
            ("\"", "string")
        ]
    );

    let source = "🙂 f(first,\nsecond)";
    let doc = mode.language().parse(Text::new(source), settings).unwrap();
    let second = source.find('\n').unwrap() + 1;
    assert_eq!(
        doc.indentation(second, Default::default()).unwrap(),
        Some(4)
    );
}

#[test]
fn keeps_r_punctuation_in_its_own_state() {
    let mode = LegacyMode::R;
    let settings = StreamSettings::default();
    let tokenize_line = |line: &str, state: &mut LegacyState| {
        let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
        while !stream.eol() {
            stream.start_token();
            mode.token(&mut stream, state);
        }
    };
    let mut string = mode.start_state(settings.indent_unit);
    let mut delimiter = mode.start_state(settings.indent_unit);
    tokenize_line("\"opening", &mut string);
    tokenize_line("f(", &mut delimiter);
    tokenize_line("closing\"", &mut string);
    assert_eq!(mode.indent(&string, "x", settings.indent_unit), Some(0));
    assert_eq!(mode.indent(&delimiter, "x", settings.indent_unit), Some(2));
}

#[test]
fn replays_edits_across_state_boundaries() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::R,
            "if (x) {\n  call(one,\n       two)\n}\nnext\n",
            "one,",
            "\"one\ncontinued\",",
        ),
        (
            LegacyMode::Julia,
            "function f()\n#= open #= inner\n=#\n=#\n  x::Tuple{Int}\nend\n",
            "inner",
            "inner =#",
        ),
        (
            LegacyMode::Lua,
            "function f()\n[=[ open\nbody\n]=]\nend\n",
            "body",
            "body ]=]\nif true then",
        ),
        (
            LegacyMode::Perl,
            "my $x = <<DATA;\nbody\nDATA\nprint $x;\n",
            "body",
            "DATA\nbody",
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
        for pos in std::iter::once(0).chain(updated.match_indices('\n').map(|(i, _)| i + 1)) {
            assert_eq!(
                replay.indentation(pos, Default::default()).unwrap(),
                fresh.indentation(pos, Default::default()).unwrap(),
                "{mode:?} at {pos}"
            );
        }
    }
}
