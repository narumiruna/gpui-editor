//! Oracle: unmodified `@codemirror/legacy-modes` 6.5.5 exports and `StringStream`
//! from `@codemirror/streamparser` 6.0.0. Both tarballs matched the SHA-512
//! digests in `docs/codemirror-upstream.json`. The external runner used tabs of
//! four columns and indent units of two and three. It records raw styles and
//! UTF-8 ranges, joins equal styles across surrogate halves, and clamps negative
//! indents to zero. At a Yacas stall it verifies that the mode returns `error`
//! without consuming input, then advances one scalar. Only Rust runs these tests.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::{HighlightTag, StreamSettings, TagModifier},
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const ORACLE: &str = include_str!("fixtures/legacy_historical_modes.json");
const EXPORTS: [(&str, LegacyMode); 5] = [
    ("dylan", LegacyMode::Dylan),
    ("eiffel", LegacyMode::Eiffel),
    ("mumps", LegacyMode::Mumps),
    ("oz", LegacyMode::Oz),
    ("yacas", LegacyMode::Yacas),
];

#[test]
fn recognizes_exact_exports_and_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (name, mode) in EXPORTS {
        let specifier = format!("@codemirror/legacy-modes/mode/{name}");
        assert_eq!(LegacyMode::from_export(&specifier, name), Some(mode));
        assert_eq!(
            LegacyMode::from_export(&format!("{specifier}.js"), name),
            None
        );
        assert_eq!(
            LegacyMode::from_export(&specifier, &name.to_uppercase()),
            None
        );
        for (other, _) in EXPORTS.into_iter().filter(|(other, _)| *other != name) {
            assert_eq!(LegacyMode::from_export(&specifier, other), None);
        }
        let expected = &oracle["modes"][name];
        assert_eq!(json!(mode.name()), expected["name"]);
        assert_eq!(mode.language().name(), name);
        assert_eq!(json!(mode.merge_tokens()), expected["mergeTokens"]);

        let doc = mode
            .language()
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data
            .get::<CommentTokens<'static>>("commentTokens")
            .map(|tokens| {
                let mut value = json!({});
                if let Some(line) = tokens.line {
                    value["line"] = json!(line);
                }
                if let Some((open, close)) = tokens.block {
                    value["block"] = json!({"open": open, "close": close});
                }
                value
            });
        assert_eq!(json!(comments), expected["commentTokens"], "{name}");
        for key in ["indentOnInput", "indentOnInut", "electricInput"] {
            let matches = data.get::<fn(&str) -> bool>(key).map(|trigger| {
                oracle["triggerInputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| trigger(input.as_str().unwrap()))
                    .collect::<Vec<_>>()
            });
            assert_eq!(json!(matches), expected["triggers"][key], "{name}/{key}");
        }
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
                let token = token.unwrap_or_else(|error| panic!("{name}/{label}: {error}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                assert_eq!(
                    &json!([token.range.start, token.range.end, token.style]),
                    expected
                        .next()
                        .unwrap_or_else(|| panic!("{name}/{label}: extra token")),
                    "{name}/{label}: {:?} in {source:?}",
                    token.text,
                );
            }
            assert!(expected.next().is_none(), "{name}/{label}: missing tokens");

            let end_indents = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    [2, 3].map(|unit| mode.indent(tokenizer.state(), input.as_str().unwrap(), unit))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(end_indents),
                case["endIndents"],
                "{name}/{label}: end state"
            );

            let doc = mode.language().parse(Text::new(source), settings).unwrap();
            let indents = line_starts(source)
                .map(|position| doc.indentation(position, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{name}/{label}: {source:?}"
            );
        }
    }
}

#[test]
fn copies_active_delimiters_and_scopes() {
    let settings = StreamSettings::default();
    for (mode, open, close, probe, closed, active) in [
        (
            LegacyMode::Dylan,
            "/* open",
            "*/",
            "method",
            "def",
            "comment",
        ),
        (
            LegacyMode::Eiffel,
            "\"open",
            "\"",
            "class",
            "keyword",
            "string",
        ),
        (
            LegacyMode::Oz,
            "proc {",
            "Name",
            "Candidate",
            "variable",
            "def",
        ),
        (
            LegacyMode::Yacas,
            "[\"open",
            "tail\"];",
            "Rank()",
            "bracket",
            "string",
        ),
    ] {
        let mut tokenizer = Tokenizer::new(&mode, open, settings.tab_size, settings.indent_unit);
        assert!(tokenizer.by_ref().all(|token| token.is_ok()));
        let mut original = mode.copy_state(tokenizer.state());
        let mut fork = mode.copy_state(&original);
        let scan = |line, state: &mut _| {
            let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
            let mut attempts = 0;
            let mut last = None;
            while !stream.eol() {
                stream.start_token();
                last = mode.token(&mut stream, state);
                attempts += 1;
                assert!(attempts <= line.len() + 1);
            }
            last
        };
        scan(close, &mut fork);
        if mode == LegacyMode::Yacas {
            assert_eq!(mode.indent(&fork, "next", 2), Some(0));
            assert_eq!(mode.indent(&original, "next", 2), None);
        }
        assert_eq!(scan(probe, &mut fork).as_deref(), Some(closed), "{mode:?}");
        assert_eq!(
            scan(probe, &mut original).as_deref(),
            Some(active),
            "{mode:?}"
        );
    }
}

#[test]
fn replays_edits_inside_stateful_regions() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Dylan,
            "/* one\n  two */\nend method",
            "two",
            "*/ if\n/* two",
        ),
        (
            LegacyMode::Eiffel,
            "X := \"line\n  middle\nlast\"\nend",
            "middle",
            "\"\nResult := '",
        ),
        (
            LegacyMode::Mumps,
            "A(x)\n\tW foo\n\tD:x read\n",
            "foo",
            " foo\n\tW $a",
        ),
        (
            LegacyMode::Oz,
            "proc {\n  Name X}\n  if X then\n  end\nend",
            "Name",
            "$\n  Y",
        ),
        (
            LegacyMode::Yacas,
            "While(x) [\n f(\n     x\n );\n];\n",
            "x\n );",
            "[x];\n /*off\n*/ );",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let from = source.find(before).unwrap();
        replay
            .apply_changes(
                &ChangeSet::single(Change::new(from..from + before.len(), after)).unwrap(),
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
        for start in line_starts(&updated) {
            assert_eq!(
                replay.indentation(start, Default::default()).unwrap(),
                fresh.indentation(start, Default::default()).unwrap(),
                "{mode:?} at {start}",
            );
        }
    }
}

#[test]
fn resolves_modified_tags_and_highlights_after_yacas_errors() {
    let source = "🙂f(x_pattern)";
    let doc = LegacyMode::Yacas
        .language()
        .parse(Text::new(source), StreamSettings::default())
        .unwrap();
    let spans = doc.tokens(0..source.len()).unwrap().collect::<Vec<_>>();
    for (text, tag, modifier) in [
        ("f", "variableName", TagModifier::Function),
        ("x_pattern", "variableName", TagModifier::Special),
    ] {
        let span = spans
            .iter()
            .find(|span| &source[span.range.clone()] == text)
            .unwrap();
        assert_eq!(span.tags, &[HighlightTag::new(tag).with_modifier(modifier)]);
    }
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}
