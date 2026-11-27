//! Oracle: unchanged `simpleMode`, Factor, NSIS, and WAST exports from
//! `@codemirror/legacy-modes` 6.5.5 with the `StringStream` export from
//! `@codemirror/streamparser` 6.0.0. Both tarballs match the SHA-512 pins in
//! `docs/codemirror-upstream.json`. The oracle runs in scratch only.
//!
//! Tab size is 4 and indent unit is 2. Indentation is queried before each
//! line, with leading whitespace removed. Token offsets use UTF-8 bytes.
//! Adjacent halves of a surrogate pair are joined when their styles agree.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::{LanguageData, StreamLanguage, StreamSettings},
    legacy_modes::{
        LegacyMode,
        simple_mode::{
            SimpleMode, SimpleModeError, SimpleModeOptions, SimpleRule, SimpleToken,
            SimpleTokenValue, simple_mode,
        },
    },
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const ORACLE: &str = include_str!("fixtures/legacy_simple_modes.json");

fn cases() -> Vec<Value> {
    serde_json::from_str(ORACLE).unwrap()
}

#[test]
fn bundled_modes_match_the_pinned_oracle() {
    for (case, mode) in cases()
        .iter()
        .zip([LegacyMode::Factor, LegacyMode::Nsis, LegacyMode::Wast])
    {
        let name = case["name"].as_str().unwrap();
        assert_eq!(
            LegacyMode::from_export(&format!("@codemirror/legacy-modes/mode/{name}"), name),
            Some(mode)
        );
        assert_tokens(&mode, case);
        assert_document(mode.language(), case);
    }
    assert!(
        LegacyMode::from_export("@codemirror/legacy-modes/mode/simple-mode", "simpleMode")
            .is_none()
    );
}

fn assert_tokens(mode: &impl StreamParser, case: &Value) {
    let settings = StreamSettings::default();
    let source = case["source"].as_str().unwrap();
    assert_eq!(json!(mode.name()), case["name"]);
    assert_eq!(json!(mode.merge_tokens()), case["mergeTokens"]);
    let mut expected = case["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap());
    for token in Tokenizer::new(mode, source, settings.tab_size, settings.indent_unit) {
        let token = token.unwrap();
        assert_eq!(token.text, &source[token.range.clone()]);
        assert_eq!(
            &json!([token.range.start, token.range.end, token.style]),
            expected.next().expect("extra token"),
            "{} at {:?}: {:?}",
            case["name"],
            token.range,
            token.text,
        );
    }
    assert!(
        expected.next().is_none(),
        "{}: missing tokens",
        case["name"]
    );
}

fn assert_document(mode: &StreamLanguage<impl StreamParser>, case: &Value) {
    let source = case["source"].as_str().unwrap();
    assert_eq!(mode.name(), case["name"].as_str().unwrap());
    let document = mode
        .parse(Text::new(source), StreamSettings::default())
        .unwrap();
    let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(byte, _)| byte + 1));
    let indents: Vec<_> = starts
        .map(|pos| document.indentation(pos, Default::default()).unwrap())
        .collect();
    assert_eq!(
        json!(indents),
        case["indents"],
        "{} indentation",
        case["name"]
    );
    let data = document.language_data_at(0).unwrap();
    assert_eq!(
        json!(data.get::<&[&str]>("dontIndentStates")),
        case["dontIndentStates"]
    );
    let comments = data.get::<CommentTokens<'static>>("commentTokens");
    assert_eq!(
        comments.and_then(|tokens| tokens.line),
        case["commentTokens"]["line"].as_str()
    );
    let block = &case["commentTokens"]["block"];
    assert_eq!(
        comments.and_then(|tokens| tokens.block),
        block["open"].as_str().zip(block["close"].as_str())
    );
    let trigger = data.get::<fn(&str) -> bool>("indentOnInput");
    if let Some(rows) = case["indentOnInput"].as_object() {
        for (line, expected) in rows {
            assert_eq!(
                json!(trigger.unwrap()(line)),
                *expected,
                "indentOnInput: {line}"
            );
        }
    } else {
        assert!(trigger.is_none());
    }
}

#[test]
fn factory_rules_match_the_pinned_oracle() {
    let case = cases().pop().unwrap();
    let mode = factory();
    assert_tokens(&mode, &case);
    assert_eq!(mode.language_data().get::<u8>("example"), Some(&17));
    let language = mode.into_language();
    assert_document(&language, &case);
    assert_eq!(
        language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap()
            .language_data_at(0)
            .unwrap()
            .get::<u8>("example"),
        Some(&17)
    );
}

fn factory() -> SimpleMode {
    fn styled(regex: &str, token: &str) -> SimpleRule {
        SimpleRule {
            token: SimpleToken::Style(token.into()),
            ..SimpleRule::new(regex)
        }
    }
    fn groups(styles: &[Option<&str>]) -> Vec<Option<String>> {
        styles
            .iter()
            .map(|style| style.map(str::to_owned))
            .collect()
    }
    fn common() -> Vec<SimpleRule> {
        vec![
            SimpleRule {
                token: SimpleToken::Groups(groups(&[
                    Some("variableName.constant"),
                    None,
                    Some("number"),
                    Some("punctuation"),
                ])),
                ..SimpleRule::new(r"(é)?(😀)(x)?(!)")
            },
            SimpleRule {
                ignore_case: true,
                token: SimpleToken::Callback(Box::new(|matched| {
                    assert_eq!(matched.get(0), matched.get(1));
                    assert_eq!(matched.captures().len(), 2);
                    if matched.named("word") == Some("--TAG") && matched.input() == "--TAG" {
                        SimpleTokenValue::Style("dynamic.raw".into())
                    } else {
                        SimpleTokenValue::Groups(groups(&[Some("other"), Some("unused")]))
                    }
                })),
                ..SimpleRule::new(r"(?<word>--[a-z]+)")
            },
            SimpleRule {
                token: SimpleToken::Callback(Box::new(|matched| {
                    SimpleTokenValue::Groups(vec![
                        Some(format!("capture-{}", matched.get(1).unwrap())),
                        Some("discarded".into()),
                    ])
                })),
                ..SimpleRule::new("(a)(b)")
            },
            SimpleRule {
                token: SimpleToken::Callback(Box::new(|_| SimpleTokenValue::Style("tag".into()))),
                ..SimpleRule::new("(c)(d)")
            },
            SimpleRule {
                token: SimpleToken::Groups(groups(&[Some("label"), Some("unused")])),
                ..SimpleRule::new("(z)")
            },
            styled(r"(a+)\1(?=!)", "atom"),
            SimpleRule {
                unicode: true,
                ..styled(r"\p{Script=Greek}+", "keyword")
            },
        ]
    }
    let close_quiet = || SimpleRule {
        pop: true,
        dedent: true,
        ..styled(r"\}", "bracket")
    };
    let mut start = vec![
        SimpleRule {
            sol: true,
            push: Some("body".into()),
            indent: true,
            ..styled("@@", "meta.open")
        },
        SimpleRule {
            next: Some("label".into()),
            ..SimpleRule::new("(?=@BEGIN)")
        },
        styled("READY", "flag"),
    ];
    start.extend(common());
    let mut body = vec![
        SimpleRule {
            indent: true,
            ..styled(r"\[", "bracket")
        },
        SimpleRule {
            dedent: true,
            ..styled(r"\]", "bracket")
        },
        SimpleRule {
            dedent: true,
            dedent_if_line_start: Some(false),
            ..styled(r"\)", "bracket")
        },
        SimpleRule {
            indent: true,
            push: Some("quiet".into()),
            ..styled(r"\{", "bracket")
        },
        close_quiet(),
    ];
    body.extend(common());
    let mut quiet = vec![close_quiet()];
    quiet.extend(common());
    let label = vec![SimpleRule {
        next: Some("start".into()),
        push: Some("ignored".into()),
        pop: true,
        ..styled("@BEGIN", "keyword")
    }];
    simple_mode(
        [
            ("start", start),
            ("body", body),
            ("quiet", quiet),
            ("label", label),
        ],
        SimpleModeOptions {
            name: Some("factory".into()),
            dont_indent_states: vec!["quiet".into()],
            merge_tokens: false,
            language_data: LanguageData::default()
                .with("example", 17_u8)
                .with("dontIndentStates", &["quiet"][..]),
        },
    )
    .unwrap()
}

#[test]
fn rejects_bad_state_names_and_regexes() {
    assert!(matches!(
        simple_mode([("other", vec![])], Default::default()),
        Err(SimpleModeError::MissingStart)
    ));
    assert!(
        matches!(simple_mode([("start", vec![]), ("start", vec![])], Default::default()), Err(SimpleModeError::DuplicateState(state)) if state == "start")
    );
    for (next, push) in [
        (Some("missing".into()), None),
        (None, Some("missing".into())),
    ] {
        let rule = SimpleRule {
            next,
            push,
            ..SimpleRule::new("never")
        };
        assert!(
            matches!(simple_mode([("start", vec![rule])], Default::default()), Err(SimpleModeError::UnknownState(state)) if state == "missing")
        );
    }
    assert!(
        matches!(simple_mode([("start", vec![SimpleRule::new("[")])], Default::default()), Err(SimpleModeError::InvalidRegex { state, rule: 0, .. }) if state == "start")
    );
}

#[test]
fn copied_states_own_pending_capture_tokens() {
    let mode = factory();
    let settings = StreamSettings::default();
    let stream = || StringStream::new("é😀x!", settings.tab_size, settings.indent_unit);
    let mut first = stream();
    let mut state = mode.start_state(settings.indent_unit);
    assert_eq!(
        mode.token(&mut first, &mut state).as_deref(),
        Some("variableName constant")
    );
    assert_eq!(first.current(), "é");
    let mut second = stream();
    second.set_pos(first.pos()).unwrap();
    let mut copied = mode.copy_state(&state);
    for (text, style) in [
        ("😀", None),
        ("x", Some("number")),
        ("!", Some("punctuation")),
    ] {
        for (stream, state) in [(&mut first, &mut state), (&mut second, &mut copied)] {
            stream.start_token();
            assert_eq!(mode.token(stream, state).as_deref(), style);
            assert_eq!(stream.current(), text);
        }
    }
}

#[test]
fn advances_at_scalar_boundaries_past_empty_rules() {
    let mode = simple_mode(
        [(
            "start",
            vec![
                SimpleRule::default(),
                SimpleRule {
                    token: SimpleToken::Style("keyword".into()),
                    ..SimpleRule::new(".")
                },
            ],
        )],
        Default::default(),
    )
    .unwrap();
    let settings = StreamSettings::default();
    let tokens = Tokenizer::new(&mode, "😀\u{2028}", settings.tab_size, settings.indent_unit)
        .map(|token| {
            let token = token.unwrap();
            (token.range, token.style)
        })
        .collect::<Vec<_>>();
    assert_eq!(tokens, [(0..4, Some("keyword".into())), (4..7, None)]);
}

#[test]
fn replays_edits_in_multiline_modes() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Factor,
            "\"\"\"first\nlast\"\"\" tail\nUSE: kernel\n",
            "last\"\"\"",
            "last",
        ),
        (
            LegacyMode::Nsis,
            "Section foo\n  /* opened\nclosed */\nSectionEnd\n",
            "closed */",
            "closed",
        ),
        (
            LegacyMode::Wast,
            "(module\n (; opened\n closed ;)\n (func i32))\n",
            "closed ;)",
            "closed",
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
        for pos in std::iter::once(0).chain(updated.match_indices('\n').map(|(byte, _)| byte + 1)) {
            assert_eq!(
                replay.indentation(pos, Default::default()),
                fresh.indentation(pos, Default::default()),
                "{mode:?} at {pos}"
            );
        }
    }
}
