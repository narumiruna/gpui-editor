//! Oracle: unchanged `mkCSS` from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both tarballs match
//! their SHA-512 SRI values in `docs/codemirror-upstream.json`. The scratch
//! harness called `indent` on each line before tokenizing with indent unit 2
//! and tab size 4. It converted token ranges from UTF-16 to UTF-8 bytes.

use std::collections::{BTreeMap, HashSet};

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{
        CSS_KEYWORDS, LegacyCss, LegacyCssConfig, LegacyCssHookToken, LegacyCssKeywordOverrides,
        LegacyCssTokenHook, LegacyCssTokenKind, mk_css,
    },
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/legacy_css_factory.json")).unwrap()
}

fn keys(table: &Value) -> Option<HashSet<String>> {
    table
        .as_object()
        .map(|table| table.keys().cloned().collect())
}

fn media_hook(stream: &mut StringStream<'_>) -> Option<LegacyCssHookToken> {
    stream
        .match_str("media", true, false)
        .then(|| LegacyCssHookToken::new(LegacyCssTokenKind::AtRule, Some("meta")))
}

fn colon_hook(_stream: &mut StringStream<'_>) -> Option<LegacyCssHookToken> {
    Some(LegacyCssHookToken::new(LegacyCssTokenKind::Mark(':'), None))
}

fn interpolation_hook(stream: &mut StringStream<'_>) -> Option<LegacyCssHookToken> {
    stream
        .eat('{')
        .map(|_| LegacyCssHookToken::new(LegacyCssTokenKind::Interpolation, Some("special")))
}

fn comment_until_percent(stream: &mut StringStream<'_>) -> LegacyCssHookToken {
    let comment = LegacyCssHookToken::new(LegacyCssTokenKind::Comment, Some("comment"));
    if stream.any(|ch| ch == '%') {
        comment
    } else {
        comment.with_continuation(comment_until_percent)
    }
}

fn comment_hook(stream: &mut StringStream<'_>) -> Option<LegacyCssHookToken> {
    Some(comment_until_percent(stream))
}

fn config(value: &Value) -> LegacyCssConfig {
    LegacyCssConfig {
        name: value["name"].as_str().map(str::to_owned),
        inline: value["inline"].as_bool().unwrap_or(false),
        allow_nested: value["allowNested"].as_bool().unwrap_or(false),
        supports_at_component: value["supportsAtComponent"].as_bool().unwrap_or(false),
        line_comment: match value["lineComment"].as_str() {
            Some("//") => Some("//"),
            Some("%%") => Some("%%"),
            None => None,
            marker => panic!("unexpected line comment: {marker:?}"),
        },
        highlight_nonstandard_property_keywords: value["highlightNonStandardPropertyKeywords"]
            .as_bool()
            .unwrap_or(true),
        keywords: LegacyCssKeywordOverrides {
            document_types: keys(&value["documentTypes"]),
            media_types: keys(&value["mediaTypes"]),
            media_features: keys(&value["mediaFeatures"]),
            media_value_keywords: keys(&value["mediaValueKeywords"]),
            property_keywords: keys(&value["propertyKeywords"]),
            nonstandard_property_keywords: keys(&value["nonStandardPropertyKeywords"]),
            font_properties: keys(&value["fontProperties"]),
            counter_descriptors: keys(&value["counterDescriptors"]),
            color_keywords: keys(&value["colorKeywords"]),
            value_keywords: keys(&value["valueKeywords"]),
        },
        token_hooks: match value["hooks"].as_str() {
            Some("scanner") => Some(BTreeMap::<char, LegacyCssTokenHook>::from([
                ('@', media_hook as LegacyCssTokenHook),
                ('?', colon_hook),
                ('~', interpolation_hook),
                ('%', comment_hook),
            ])),
            None if value["tokenHooks"].is_object() => Some(BTreeMap::new()),
            None => None,
            hooks => panic!("unexpected hooks: {hooks:?}"),
        },
    }
}

fn scanner() -> LegacyCss {
    let fixture = oracle();
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["config"]["hooks"] == "scanner")
        .unwrap();
    mk_css(config(&case["config"]))
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}

#[test]
fn matches_pinned_factory_tokens_indents_and_metadata() {
    let fixture = oracle();
    let settings = StreamSettings::default();
    for case in fixture["cases"].as_array().unwrap() {
        let title = case["title"].as_str().unwrap();
        let mode = mk_css(config(&case["config"]));
        let source = case["source"].as_str().unwrap();
        let lang = &case["lang"];
        assert_eq!(json!(mode.name()), lang["name"], "{title}");
        assert_eq!(json!(mode.merge_tokens()), lang["mergeTokens"], "{title}");

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
                "{title}: {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert!(expected.next().is_none(), "{title}: missing tokens");

        let language = mode.language();
        assert_eq!(language.name(), lang["name"].as_str().unwrap_or_default());
        let doc = language.parse(Text::new(source), settings).unwrap();
        let indents = line_starts(source)
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{title}: indentation");

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        let (open, close) = comments.block.unwrap();
        assert_eq!(
            json!({"line": comments.line, "block": {"open": open, "close": close}}),
            lang["commentTokens"],
            "{title}"
        );
        assert_eq!(
            data.get::<&[&str]>("autocomplete").copied(),
            Some(CSS_KEYWORDS.all),
            "{title}: autocomplete"
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        for probe in fixture["triggers"].as_array().unwrap() {
            let text = probe[0].as_str().unwrap();
            assert_eq!(json!(trigger(text)), probe[1], "{title}: {text:?}");
        }
    }
}

#[test]
fn copied_continuation_is_independent() {
    let mode = scanner();
    let settings = StreamSettings::default();
    let mut tokens = Tokenizer::new(&mode, "a { color? %open", settings.tab_size, 2);
    for token in tokens.by_ref() {
        token.unwrap();
    }
    let mut ended = mode.copy_state(tokens.state());
    let mut open = mode.copy_state(tokens.state());
    let mut line = StringStream::new("close% red", settings.tab_size, 2);
    assert_eq!(
        mode.token(&mut line, &mut ended).as_deref(),
        Some("comment")
    );
    assert_eq!(line.current(), "close%");
    line.start_token();
    assert_eq!(mode.token(&mut line, &mut ended), None);
    line.start_token();
    assert_eq!(
        mode.token(&mut line, &mut ended).as_deref(),
        Some("keyword")
    );
    let mut line = StringStream::new("still open", settings.tab_size, 2);
    assert_eq!(mode.token(&mut line, &mut open).as_deref(), Some("comment"));
    assert_eq!(line.current(), "still open");
}

#[test]
fn incremental_parse_replays_a_custom_hook_continuation() {
    let settings = StreamSettings::default();
    let language = scanner().language();
    let source = "a {\n  color? %open\n  still open\n  close% red;\n}";
    let mut replay = language.parse(Text::new(source), settings).unwrap();
    let old = "still open";
    let new = "more\n  comment text";
    let start = source.find(old).unwrap();
    let change = ChangeSet::single(Change::new(start..start + old.len(), new)).unwrap();
    replay.apply_changes(&change).unwrap();
    let updated = source.replacen(old, new, 1);
    let fresh = language.parse(Text::new(&updated), settings).unwrap();
    let range = 0..updated.len();
    assert!(
        replay
            .tokens(range.clone())
            .unwrap()
            .eq(fresh.tokens(range).unwrap())
    );
    for start in line_starts(&updated) {
        assert_eq!(
            replay.indentation(start, Default::default()),
            fresh.indentation(start, Default::default()),
            "indent at {start}"
        );
    }
}
