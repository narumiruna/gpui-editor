//! Oracle: unchanged `mkPython` from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs
//! match the SRI digests in `docs/codemirror-upstream.json`. The external runner
//! used indent unit 2 and tab size 4, recorded advancing raw tokens, converted
//! offsets to UTF-8 bytes, and clamped negative indents to zero. End-state
//! probes use both units 2 and 3. These tests run Rust only.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyPythonConfig, mk_python},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use regex::Regex;
use serde_json::{Value, json};

fn config(data: &Value) -> LegacyPythonConfig {
    let pattern = |key| data[key].as_str().map(|pat| Regex::new(pat).unwrap());
    let words = |key| {
        data[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|word| word.as_str().unwrap().to_owned())
            .collect()
    };
    LegacyPythonConfig {
        version: data["version"].as_u64().map(|v| u32::try_from(v).unwrap()),
        identifiers: pattern("identifiers"),
        extra_keywords: words("extra_keywords"),
        extra_builtins: words("extra_builtins"),
        operators: pattern("operators"),
        single_operators: pattern("singleOperators"),
        double_operators: pattern("doubleOperators"),
        double_delimiters: pattern("doubleDelimiters"),
        triple_delimiters: pattern("tripleDelimiters"),
        delimiters: pattern("delimiters"),
        single_delimiters: pattern("singleDelimiters"),
        hanging_indent: data["hangingIndent"]
            .as_u64()
            .map(|width| usize::try_from(width).unwrap()),
        single_line_string_errors: data["singleLineStringErrors"].as_bool().unwrap_or_default(),
    }
}

#[test]
fn matches_pinned_python_factory_cases() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/legacy_python_factory.json")).unwrap();
    let metadata = &oracle["metadata"];
    let settings = StreamSettings::default();
    for case in oracle["cases"].as_array().unwrap() {
        let title = case["title"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = mk_python(config(&case["config"]));
        let language = mode.language();
        assert_eq!(json!(language.name()), metadata["name"], "{title}");
        assert!(mode.merge_tokens(), "{title}");

        let expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        let mut tokenizer = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        let tokens: Vec<_> = tokenizer
            .by_ref()
            .map(|token| {
                let token = token.unwrap_or_else(|err| panic!("{title}: {err}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                json!([token.range.start, token.range.end, token.style])
            })
            .collect();
        assert!(tokens.iter().eq(expected), "{title}: tokens {tokens:?}");

        let doc = language.parse(Text::new(source), settings).unwrap();
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
        let indents: Vec<_> = starts
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect();
        assert_eq!(json!(indents), case["indents"], "{title}: line indentation");
        let last_indents: Vec<_> = oracle["afterInputs"]
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
            metadata["commentTokens"]["line"],
            "{title}"
        );
        assert!(comments.block.is_none(), "{title}");
        for key in ["autocomplete", "closeBrackets"] {
            assert_eq!(
                json!(data.get::<&[&str]>(key)),
                metadata[key],
                "{title}: {key}"
            );
        }
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        let triggers: Vec<_> = oracle["triggerInputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| trigger(input.as_str().unwrap()))
            .collect();
        assert_eq!(
            json!(triggers),
            metadata["triggers"],
            "{title}: indentOnInput"
        );
    }
}

#[test]
fn copies_unterminated_string_checkpoints() {
    let mode = mk_python(LegacyPythonConfig {
        single_line_string_errors: true,
        ..Default::default()
    });
    let settings = StreamSettings::default();
    let mut tokenizer = Tokenizer::new(
        &mode,
        "f\"{value} tail",
        settings.tab_size,
        settings.indent_unit,
    );
    for token in tokenizer.by_ref() {
        token.unwrap();
    }
    let original = tokenizer.state();
    let mut closed = mode.copy_state(original);
    let mut open = mode.copy_state(original);
    let mut closing = StringStream::new("end\"", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut closing, &mut closed).as_deref(),
        Some("string")
    );
    assert!(closing.eol());
    assert_eq!(mode.indent(&closed, "", 2), Some(0));
    assert_eq!(mode.indent(original, "", 2), None);
    let mut continuation = StringStream::new("end", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut continuation, &mut open).as_deref(),
        Some("error")
    );
    assert!(continuation.eol());
}

#[test]
fn replays_an_edit_inside_a_format_expression() {
    let language = mk_python(LegacyPythonConfig {
        identifiers: Some(Regex::new(r"^\$[a-z]+").unwrap()),
        hanging_indent: Some(4),
        ..Default::default()
    })
    .language();
    let settings = StreamSettings::default();
    let source = "$msg = f\"\"\"before {\n$old\n} tail\"\"\"\n$end";
    let mut replay = language.parse(Text::new(source), settings).unwrap();
    let start = source.find("$old").unwrap();
    let replacement = "$call(\n    $new\n)";
    let edit = ChangeSet::single(Change::new(start..start + "$old".len(), replacement)).unwrap();
    replay.apply_changes(&edit).unwrap();
    let updated = source.replace("$old", replacement);
    let fresh = language.parse(Text::new(&updated), settings).unwrap();
    let range = 0..updated.len();
    assert!(
        replay
            .tokens(range.clone())
            .unwrap()
            .eq(fresh.tokens(range).unwrap())
    );
    for pos in std::iter::once(0).chain(updated.match_indices('\n').map(|(i, _)| i + 1)) {
        assert_eq!(
            replay.indentation(pos, Default::default()),
            fresh.indentation(pos, Default::default()),
            "at {pos}"
        );
    }
}

#[test]
fn retains_definition_style_when_an_edit_joins_declaration_names() {
    use gpui_editor_codemirror::{
        language::{StreamDocument, TagModifier},
        legacy_modes::LegacyPython,
    };

    let settings = StreamSettings::default();
    let mode = mk_python(LegacyPythonConfig {
        version: Some(2),
        ..Default::default()
    });
    let language = mode.language();
    for row in include_str!("fixtures/legacy_python_declaration_edit.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let byte = |key: &Value| usize::try_from(key.as_u64().unwrap()).unwrap();
        let range = byte(&case["range"][0])..byte(&case["range"][1]);
        let name = case["name"].as_str().unwrap();
        let check = |doc: &StreamDocument<LegacyPython>, expected: &Value| {
            let text = doc.text().as_str();
            let token = Tokenizer::new(&mode, text, settings.tab_size, settings.indent_unit)
                .map(Result::unwrap)
                .find(|token| token.range == range)
                .unwrap();
            assert_eq!(token.text, name);
            let expected = expected.as_str();
            assert_eq!(token.style.as_deref(), expected, "token for {name}");
            let tags = doc.tokens(range.clone()).unwrap().next().unwrap().tags;
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].name(), "variableName");
            assert_eq!(
                tags[0].modifiers().contains(&TagModifier::Definition),
                expected == Some("def"),
                "highlight for {name}"
            );
        };
        let source = case["source"].as_str().unwrap();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        check(&replay, &case["before"]);
        let at = byte(&case["at"]);
        let edit =
            ChangeSet::single(Change::new(at..at + 1, case["insert"].as_str().unwrap())).unwrap();
        replay.apply_changes(&edit).unwrap();
        let changed = case["changed"].as_str().unwrap();
        assert_eq!(replay.text().as_str(), changed);
        check(&replay, &case["after"]);
        check(
            &language.parse(Text::new(changed), settings).unwrap(),
            &case["after"],
        );
        replay
            .apply_changes(&ChangeSet::single(Change::new(at..at + 1, "(")).unwrap())
            .unwrap();
        assert_eq!(replay.text().as_str(), source);
        check(&replay, &case["before"]);
    }
}

#[test]
fn native_overrides_require_progress_at_the_cursor() {
    let mode = mk_python(LegacyPythonConfig {
        identifiers: Some(Regex::new(r"[A-Z]+").unwrap()),
        operators: Some(Regex::new(r"^").unwrap()),
        delimiters: Some(Regex::new(r"^").unwrap()),
        extra_keywords: vec![String::new(), "do+".into()],
        ..Default::default()
    });
    let tokens: Vec<_> = Tokenizer::new(&mode, "!X do+y", StreamSettings::default().tab_size, 2)
        .map(|token| {
            let token = token.unwrap();
            (token.text, token.style)
        })
        .collect();
    assert_eq!(
        tokens,
        [
            ("!", Some("error".into())),
            ("X", Some("variable".into())),
            (" ", None),
            ("do+", Some("keyword".into())),
            ("y", Some("error".into())),
        ]
    );
}
