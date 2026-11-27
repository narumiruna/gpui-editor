//! Oracle: `mkXML` from the unchanged `@codemirror/legacy-modes` 6.5.5 ESM
//! export and `StringStream` from `@codemirror/streamparser` 6.0.0. Both
//! tarballs match their SRI digests in `docs/codemirror-upstream.json`.
//! The harness called `indent` before each line with leading whitespace
//! removed, then read raw `token` results with indent unit 2 and tab size 4.
//! The harness converted token ranges from UTF-16 to UTF-8 byte positions.

use std::collections::HashSet;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::{LegacyXml, LegacyXmlConfig, LegacyXmlState, mk_xml},
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/legacy_xml_factory.json")).unwrap()
}

fn keys(table: &Value) -> Option<HashSet<String>> {
    table
        .as_object()
        .map(|table| table.keys().cloned().collect())
}

fn config(value: &Value) -> LegacyXmlConfig {
    LegacyXmlConfig {
        html_mode: value["htmlMode"].as_bool().unwrap_or(false),
        auto_self_closers: keys(&value["autoSelfClosers"]),
        implicitly_closed: keys(&value["implicitlyClosed"]),
        context_grabbers: value["contextGrabbers"].as_object().map(|table| {
            table
                .iter()
                .map(|(parent, tags)| (parent.clone(), keys(tags).unwrap()))
                .collect()
        }),
        do_not_indent: keys(&value["doNotIndent"]),
        allow_unquoted: value["allowUnquoted"].as_bool(),
        allow_missing: value["allowMissing"].as_bool(),
        allow_missing_tag_name: value["allowMissingTagName"].as_bool().unwrap_or(false),
        match_closing: value["matchClosing"].as_bool().unwrap_or(true),
        multiline_tag_indent_past_tag: value["multilineTagIndentPastTag"].as_bool().unwrap_or(true),
        multiline_tag_indent_factor: value["multilineTagIndentFactor"]
            .as_u64()
            .map_or(1, |factor| usize::try_from(factor).unwrap()),
        align_cdata: value["alignCDATA"].as_bool().unwrap_or(false),
    }
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> {
    std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1))
}

#[test]
fn matches_pinned_factory_tokens_indents_and_metadata() {
    let samples = oracle();
    let settings = StreamSettings::default();
    for case in samples["cases"].as_array().unwrap() {
        let title = case["title"].as_str().unwrap();
        let mode = mk_xml(config(&case["config"]));
        let source = case["source"].as_str().unwrap();
        assert_eq!(mode.name(), Some("xml"));
        assert_eq!(mode.configuration(), case["configuration"]);
        assert!(mode.merge_tokens());

        let mut expected = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|line| line.as_array().unwrap());
        for token in Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit) {
            let token = token.unwrap();
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                expected.next().expect("extra token"),
                "{title}: {:?}",
                token.text,
            );
        }
        assert!(expected.next().is_none(), "{title}: missing tokens");

        let language = mode.language();
        let doc = language.parse(Text::new(source), settings).unwrap();
        let indents = line_starts(source)
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{title}: indentation");

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        let (open, close) = comments.block.unwrap();
        assert!(comments.line.is_none());
        assert_eq!(
            json!({"block": {"open": open, "close": close}}),
            samples["comments"]
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        for probe in samples["triggers"].as_array().unwrap() {
            let text = probe[0].as_str().unwrap();
            assert_eq!(
                json!(trigger(text)),
                probe[1],
                "{title}: trigger for {text:?}"
            );
        }
    }
}

fn read_line(mode: &LegacyXml, state: &mut LegacyXmlState, line: &str) -> Vec<Value> {
    let settings = StreamSettings::default();
    let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
    let mut tokens = Vec::new();
    while !stream.eol() {
        stream.start_token();
        let style = mode.token(&mut stream, state);
        assert!(stream.pos() > stream.start());
        tokens.push(json!([stream.start(), stream.pos(), style]));
    }
    tokens
}

#[test]
fn matches_pinned_state_hooks_and_copies_open_state() {
    let mode = mk_xml(LegacyXmlConfig {
        allow_missing_tag_name: true,
        ..LegacyXmlConfig::default()
    });
    let mut state = mode.start_state(2);
    for step in oracle()["steps"].as_array().unwrap() {
        let input = step["input"].as_str().unwrap();
        if step["skip"].as_bool().unwrap() {
            state.skip_attribute();
        }
        assert_eq!(
            json!(read_line(&mode, &mut state, input)),
            step["tokens"],
            "{input:?}"
        );
        assert_eq!(
            mode.copy_state(&state).xml_current_tag(),
            step["tag"]["name"].as_str()
        );
        if state.xml_current_tag().is_some() {
            assert_eq!(step["tag"]["close"], false);
        }
        assert_eq!(
            json!(state.xml_current_context().collect::<Vec<_>>()),
            step["context"]
        );
    }

    read_line(&mode, &mut state, "<root title=");
    let mut branch = mode.copy_state(&state);
    branch.skip_attribute();
    assert_eq!(read_line(&mode, &mut branch, ">")[0][2], "angleBracket");
    assert_eq!(state.xml_current_tag(), Some("root"));
    assert_eq!(read_line(&mode, &mut state, ">")[0][2], "error");
    read_line(&mode, &mut branch, "</root>");
    assert_eq!(branch.xml_current_context().next(), None);
    assert_eq!(state.xml_current_context().next(), Some("root"));
}

#[test]
fn uses_scalar_columns_instead_of_upstream_utf16_columns() {
    let samples = oracle();
    let unicode = &samples["unicodeIndent"];
    let mode = mk_xml(LegacyXmlConfig::default());
    let mut state = mode.start_state(2);
    let line = unicode["firstLine"].as_str().unwrap();
    let tokens = read_line(&mode, &mut state, line);
    assert_eq!(tokens[1], json!([1, line.len(), "tag"]));
    assert_eq!(unicode["indent"], 4);
    assert_eq!(
        mode.indent(&state, unicode["nextLine"].as_str().unwrap(), 2),
        Some(3)
    );
}

#[test]
fn replays_edits_through_a_multiline_attribute() {
    let language = mk_xml(LegacyXmlConfig {
        auto_self_closers: Some(HashSet::from(["leaf".to_owned()])),
        align_cdata: true,
        ..LegacyXmlConfig::default()
    })
    .language();
    let settings = StreamSettings::default();
    let source = "<root title=\"first\nsecond\">\n  <Leaf>\n  <![CDATA[text]]>\n</root>\n";
    let new = "new\nsecond";
    let start = source.find("second").unwrap();
    let mut replay = language.parse(Text::new(source), settings).unwrap();
    replay
        .apply_changes(&ChangeSet::single(Change::new(start..start + "second".len(), new)).unwrap())
        .unwrap();
    let updated = source.replacen("second", new, 1);
    let fresh = language.parse(Text::new(&updated), settings).unwrap();
    let range = 0..updated.len();
    assert!(
        replay
            .tokens(range.clone())
            .unwrap()
            .eq(fresh.tokens(range).unwrap())
    );
    for pos in line_starts(&updated) {
        assert_eq!(
            replay.indentation(pos, Default::default()),
            fresh.indentation(pos, Default::default())
        );
    }
}
