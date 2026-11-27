//! Oracle: unmodified `@codemirror/legacy-modes` 6.5.5 ESM exports and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both tarballs matched
//! the SHA-512 SRI digests in `docs/codemirror-upstream.json`.
//! The scratch runner used four-column tabs and indent unit two. It recorded
//! raw styles and UTF-8 byte ranges, excluding line endings. It joined surrogate
//! halves with equal styles and truncated indents to non-negative whole columns.
//! At the end of each case it also probed indentation with indent unit three.
//! Only Rust runs these tests.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::{HighlightTag, IndentOptions, StreamSettings, TagModifier},
    legacy_modes::{LegacyMode, LegacyState},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_classic_modes.json");
const EXPORTS: [(&str, LegacyMode); 5] = [
    ("apl", LegacyMode::Apl),
    ("fortran", LegacyMode::Fortran),
    ("cobol", LegacyMode::Cobol),
    ("pascal", LegacyMode::Pascal),
    ("smalltalk", LegacyMode::Smalltalk),
];

#[test]
fn recognizes_only_the_exact_exports() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (name, mode) in EXPORTS {
        let module = format!("{PREFIX}{name}");
        assert_eq!(LegacyMode::from_export(&module, name), Some(mode));
        assert_eq!(LegacyMode::from_export(&module, &name.to_uppercase()), None);
        assert_eq!(LegacyMode::from_export(&format!("{module}.js"), name), None);
        for (other, _) in EXPORTS.into_iter().filter(|(other, _)| *other != name) {
            assert_eq!(LegacyMode::from_export(&module, other), None);
        }
        assert_eq!(json!(mode.name()), oracle["modes"][name]["name"]);
        assert_eq!(mode.language().name(), name);
        assert!(mode.merge_tokens());
    }
}

#[test]
fn matches_pinned_tokens_and_indentation() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (name, mode) in EXPORTS {
        for case in oracle["modes"][name]["cases"].as_array().unwrap() {
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
            let last_indents = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    [2, 3].map(|unit| mode.indent(tokenizer.state(), input.as_str().unwrap(), unit))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(last_indents),
                case["lastIndents"],
                "{name}/{label}: end state"
            );

            let doc = mode.language().parse(Text::new(source), settings).unwrap();
            let indents = line_starts(source)
                .map(|position| doc.indentation(position, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{name}/{label}: indentation"
            );
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
            .map(|tokens| {
                assert_eq!(tokens.line, None, "{name}");
                let (open, close) = tokens.block.unwrap();
                json!({"block": {"open": open, "close": close}})
            });
        assert_eq!(json!(comments), expected["commentTokens"], "{name}");
        let triggers = data
            .get::<fn(&str) -> bool>("indentOnInput")
            .map(|trigger| {
                oracle["triggerInputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| trigger(input.as_str().unwrap()))
                    .collect::<Vec<_>>()
            });
        assert_eq!(json!(triggers), expected["triggers"], "{name}");
    }
}

#[test]
fn copies_active_state_for_independent_scans() {
    let settings = StreamSettings::default();
    for (mode, start, close, probe, active, closed) in [
        (
            LegacyMode::Fortran,
            "'start\\",
            "'",
            "PROGRAM",
            "string",
            "keyword",
        ),
        (
            LegacyMode::Cobol,
            "       \"start",
            "'",
            "       MOVE",
            "string",
            "keyword",
        ),
        (
            LegacyMode::Pascal,
            "(*start",
            "*)",
            "BEGIN",
            "comment",
            "keyword",
        ),
        (
            LegacyMode::Smalltalk,
            "[ 'start",
            "'",
            ".",
            "string",
            "operator",
        ),
    ] {
        let scan_line = |line: &str, state: &mut LegacyState| {
            let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
            let mut last = None;
            while !stream.eol() {
                stream.start_token();
                last = mode.token(&mut stream, state);
                assert!(stream.pos() > stream.start());
            }
            last
        };
        let mut state = mode.start_state(settings.indent_unit);
        assert_eq!(
            scan_line(start, &mut state).as_deref(),
            Some(active),
            "{mode:?}"
        );
        let mut fork = mode.copy_state(&state);
        assert_eq!(
            scan_line(close, &mut fork).as_deref(),
            Some(active),
            "{mode:?}"
        );
        assert_eq!(
            scan_line(probe, &mut fork).as_deref(),
            Some(closed),
            "{mode:?}"
        );
        assert_eq!(
            scan_line(probe, &mut state).as_deref(),
            Some(active),
            "{mode:?}"
        );
    }
}

#[test]
fn replays_edits_across_state_boundaries() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (LegacyMode::Apl, "A←⍳2\n∘. ⍬\n", "⍳2", "⍳¯2 ⍝ 日本"),
        (
            LegacyMode::Fortran,
            "PRINT *, 'one\\\ncontinued'\nEND\n",
            "continued",
            "! end'\nnext",
        ),
        (
            LegacyMode::Cobol,
            "       DISPLAY 'one\n       still\n       here'\n       MOVE ZERO.\n",
            "still",
            "now\" ADD\n       DISPLAY '",
        ),
        (
            LegacyMode::Pascal,
            "Begin (*one\n body\n*) null\nEnd.\n",
            "body",
            "inside *) {new\n finish}",
        ),
        (
            LegacyMode::Smalltalk,
            "[\n  [\n  'one\ncontinued'\n  ]\n]\n",
            "continued",
            "tail'\n   [here",
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
                "{mode:?} at {pos}"
            );
        }
    }
}

#[test]
fn resolves_and_merges_apl_function_highlights() {
    let source = "⍳×∘.⍬";
    let doc = LegacyMode::Apl
        .language()
        .parse(Text::new(source), StreamSettings::default())
        .unwrap();
    let mut tokens = doc.tokens(0..source.len()).unwrap();
    let builtin = tokens.next().unwrap();
    assert_eq!(&source[builtin.range], "⍳×");
    assert_eq!(
        builtin.tags,
        &[HighlightTag::new("variableName")
            .with_modifier(TagModifier::Function)
            .with_modifier(TagModifier::Standard)]
    );
    let function = tokens.next().unwrap();
    assert_eq!(&source[function.range], "∘.");
    assert_eq!(
        function.tags,
        &[HighlightTag::new("variableName").with_modifier(TagModifier::Function)]
    );
    assert_eq!(tokens.next().unwrap().tags, &[HighlightTag::new("atom")]);
    assert!(tokens.next().is_none());
}

#[test]
fn cobol_fixed_columns_count_unicode_scalars() {
    let source = format!("       🙂{}MOVE", " ".repeat(63));
    let settings = StreamSettings::default();
    let tokens = Tokenizer::new(
        &LegacyMode::Cobol,
        &source,
        settings.tab_size,
        settings.indent_unit,
    )
    .map(Result::unwrap)
    .collect::<Vec<_>>();
    assert_eq!(tokens[1].text, "🙂");
    assert_eq!(tokens[3].text, "M");
    assert_eq!(tokens[3].style, None);
    assert_eq!(tokens[4].text, "OVE");
    assert_eq!(tokens[4].style.as_deref(), Some("header"));
}

#[test]
fn smalltalk_uses_indentation_override_on_prior_lines() {
    let source = "[\n   one\ntwo";
    let settings = StreamSettings::default();
    let doc = LegacyMode::Smalltalk
        .language()
        .parse(Text::new(source), settings)
        .unwrap();
    let third = source.find("two").unwrap();
    assert_eq!(
        doc.indentation(third, IndentOptions::default()).unwrap(),
        Some(3)
    );
    let options = IndentOptions {
        override_indentation: Some(&|position| (position == 2).then_some(5)),
        ..Default::default()
    };
    assert_eq!(doc.indentation(third, options).unwrap(), Some(5));
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}
