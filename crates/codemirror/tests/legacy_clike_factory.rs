//! Oracle: unchanged `clike` from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs
//! match the SHA-512 digests in `docs/codemirror-upstream.json`. The external
//! runner recorded raw tokens, converted UTF-16 offsets to UTF-8 bytes, and
//! clamped negative indents to zero. Only Rust runs these tests.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::{LanguageData, StreamSettings},
    legacy_modes::{LegacyClikeConfig, LegacyClikeHookResult, LegacyClikeWords, clike},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use regex::Regex;
use serde_json::{Value, json};

fn config(options: &Value) -> LegacyClikeConfig {
    let pattern = |key| options[key].as_str().map(|pat| Regex::new(pat).unwrap());
    let words = |key| match &options[key] {
        Value::Array(list) => LegacyClikeWords::List(
            list.iter()
                .map(|word| word.as_str().unwrap().to_owned())
                .collect(),
        ),
        Value::String(test) if test == "upper" => {
            LegacyClikeWords::Predicate(|word| word.starts_with(|ch: char| ch.is_ascii_uppercase()))
        }
        Value::Null => LegacyClikeWords::default(),
        unexpected => panic!("unknown word list {unexpected}"),
    };
    let enabled = |key| options[key].as_bool().unwrap_or(false);
    let string = |key| options[key].as_str().map(str::to_owned);
    let unit = |key| {
        options[key]
            .as_u64()
            .map_or(0, |n| usize::try_from(n).unwrap())
    };
    let mut config = LegacyClikeConfig {
        name: string("name"),
        keywords: words("keywords"),
        types: words("types"),
        builtin: words("builtin"),
        atoms: words("atoms"),
        def_keywords: words("defKeywords"),
        block_keywords: words("blockKeywords"),
        is_reserved_identifier: match options["isReservedIdentifier"].as_str() {
            Some("underscores") => Some(|word| word.starts_with("__")),
            _ => None,
        },
        is_punctuation_char: pattern("isPunctuationChar"),
        number_start: pattern("numberStart"),
        number: pattern("number"),
        is_operator_char: pattern("isOperatorChar"),
        is_identifier_char: pattern("isIdentifierChar"),
        namespace_separator: string("namespaceSeparator"),
        multi_line_strings: enabled("multiLineStrings"),
        type_first_definitions: enabled("typeFirstDefinitions"),
        style_defs: options["styleDefs"].as_bool().unwrap_or(true),
        statement_indent_unit: unit("statementIndentUnit"),
        dont_align_calls: enabled("dontAlignCalls"),
        indent_statements: options["indentStatements"].as_bool().unwrap_or(true),
        indent_switch: options["indentSwitch"].as_bool().unwrap_or(true),
        dont_indent_statements: pattern("dontIndentStatements"),
        allman_indentation: enabled("allmanIndentation"),
        ..Default::default()
    };
    if options["hooks"] == "sample" {
        use LegacyClikeHookResult::{Continue, Style, Unstyled};
        config.hooks = vec![
            ('@', |s| {
                s.eat_while(|ch| ch.is_ascii_lowercase());
                Style("meta")
            }),
            ('~', |s| {
                s.eat_while(|ch| ch.is_ascii_lowercase());
                Unstyled
            }),
            ('%', |_| Continue),
        ];
        config.token_hook = Some(|s, _| match s.current() {
            "magic" => Style("def"),
            "removeme" => Unstyled,
            _ => Continue,
        });
    }
    if options["languageData"] == "sample" {
        config.language_data = Some(|data: LanguageData| {
            data.with(
                "commentTokens",
                CommentTokens {
                    line: Some("#"),
                    block: None,
                },
            )
            .with(
                "indentOnInput",
                (|line| line == "reindent") as fn(&str) -> bool,
            )
            .with("autocomplete", vec![String::from("manual")])
            .with("custom", "metadata")
        });
    }
    config
}

#[test]
fn matches_pinned_clike_factory_cases() {
    let mut lines = include_str!("fixtures/legacy_clike_factory.jsonl").lines();
    let inputs: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let settings = StreamSettings::default();
    for line in lines {
        let case: Value = serde_json::from_str(line).unwrap();
        let title = case["title"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = clike(config(&case["options"]));
        let language = mode.language();
        assert_eq!(json!(mode.name()), case["name"], "{title}");
        assert_eq!(language.name(), mode.name().unwrap_or(""), "{title}");
        assert!(mode.merge_tokens(), "{title}");

        let mut expected = case["tokens"].as_array().unwrap().iter();
        let mut tokenizer = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        for token in tokenizer.by_ref() {
            let token = token.unwrap_or_else(|err| panic!("{title}: {err}"));
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                expected.next().expect("extra token"),
                "{title} at bytes {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert!(expected.next().is_none(), "{title}: missing tokens");

        let doc = language.parse(Text::new(source), settings).unwrap();
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
        let indents: Vec<_> = starts
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect();
        assert_eq!(json!(indents), case["indents"], "{title}: indentation");
        let last_indents: Vec<_> = inputs["afterInputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| {
                [2, 3].map(|unit| mode.indent(tokenizer.state(), input.as_str().unwrap(), unit))
            })
            .collect();
        assert_eq!(
            json!(last_indents),
            case["lastIndents"],
            "{title}: final indentation"
        );

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        assert_eq!(
            json!(comments.line),
            case["commentTokens"]["line"],
            "{title}"
        );
        let block = comments
            .block
            .map(|(open, close)| json!({"open": open, "close": close}));
        assert_eq!(json!(block), case["commentTokens"]["block"], "{title}");
        assert_eq!(
            json!(data.get::<Vec<String>>("autocomplete")),
            case["autocomplete"],
            "{title}"
        );
        assert_eq!(json!(data.get::<&str>("custom")), case["custom"], "{title}");
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        let triggered: Vec<_> = inputs["triggerInputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| trigger(input.as_str().unwrap()))
            .collect();
        assert_eq!(json!(triggered), case["trigger"], "{title}: indentOnInput");
    }
}

#[test]
fn copies_multiline_string_state() {
    let settings = StreamSettings::default();
    let mode = clike(LegacyClikeConfig {
        multi_line_strings: true,
        ..Default::default()
    });
    let mut tokenizer = Tokenizer::new(&mode, "\"open", settings.tab_size, settings.indent_unit);
    for token in tokenizer.by_ref() {
        token.unwrap();
    }
    let original = tokenizer.state();
    let mut closed = mode.copy_state(original);
    let mut line = StringStream::new("close\"", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut line, &mut closed).as_deref(),
        Some("string")
    );
    assert!(line.eol());
    assert!(mode.indent(&closed, "", 2).is_some());
    assert_eq!(mode.indent(original, "", 2), None);
}

#[test]
fn replays_edits_in_a_multiline_string() {
    let settings = StreamSettings::default();
    let language = clike(LegacyClikeConfig {
        multi_line_strings: true,
        dont_align_calls: true,
        statement_indent_unit: 5,
        ..Default::default()
    })
    .language();
    let source = "call(first,\n\"opening\nfollow\nend\");\nnext();\n";
    let mut incremental = language.parse(Text::new(source), settings).unwrap();
    let start = source.find("follow").unwrap();
    let after = "different\nline";
    incremental
        .apply_changes(&ChangeSet::single(Change::new(start..start + 6, after)).unwrap())
        .unwrap();
    let updated = source.replacen("follow", after, 1);
    let fresh = language.parse(Text::new(&updated), settings).unwrap();
    let range = 0..updated.len();
    assert!(
        incremental
            .tokens(range.clone())
            .unwrap()
            .eq(fresh.tokens(range).unwrap())
    );
    for (end, _) in updated.match_indices('\n') {
        assert_eq!(
            incremental
                .indentation(end + 1, Default::default())
                .unwrap(),
            fresh.indentation(end + 1, Default::default()).unwrap(),
        );
    }
}

#[test]
fn ignores_empty_matches_without_splitting_a_scalar() {
    let settings = StreamSettings::default();
    let mode = clike(LegacyClikeConfig {
        number_start: Some(Regex::new("[#§]").unwrap()),
        number: Some(Regex::new("^(?:§[0-9]+)?").unwrap()),
        namespace_separator: Some(String::new()),
        ..Default::default()
    });
    let source = "#name §name §42";
    let tokens: Vec<_> = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit)
        .map(|token| {
            let token = token.unwrap();
            (token.text, token.style.unwrap_or_default())
        })
        .collect();
    assert_eq!(
        tokens
            .iter()
            .map(|(text, style)| (*text, style.as_ref()))
            .collect::<Vec<_>>(),
        [
            ("#name", "variable"),
            (" ", ""),
            ("§name", "variable"),
            (" ", ""),
            ("§42", "number")
        ],
    );
}
