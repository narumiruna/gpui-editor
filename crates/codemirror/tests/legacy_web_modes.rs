//! Oracle: unchanged `@codemirror/legacy-modes` 6.5.5 ESM exports and
//! `@codemirror/streamparser` 6.0.0 `StringStream`. Both tarballs match
//! their SRI digests in `docs/codemirror-upstream.json`. The harness called
//! `indent` before each line and `token` until the cursor advanced, with
//! indent unit 2 and tab size 4. Styles are raw and positions are UTF-8
//! bytes. The fixture includes CRLF lines; line endings do not produce tokens.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_web_modes.json");

fn mode(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_web_mode_oracle() {
    let settings = StreamSettings::default();
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    for case in cases {
        let title = case["title"].as_str().unwrap();
        let module = case["module"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = mode(module, export).unwrap();
        assert_eq!(json!(mode.name()), case["name"]);
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
                "{export}/{title}: {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert!(
            expected.next().is_none(),
            "{export}/{title}: missing tokens"
        );

        let language = mode.language();
        assert_eq!(language.name(), mode.name().unwrap());
        let doc = language.parse(Text::new(source), settings).unwrap();
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
        let indents = starts
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            json!(indents),
            case["indents"],
            "{export}/{title}: indentation"
        );

        let data = doc.language_data_at(0).unwrap();
        let comments = data
            .get::<CommentTokens<'static>>("commentTokens")
            .map(|tokens| {
                let mut value = serde_json::Map::new();
                if let Some(line) = tokens.line {
                    value.insert("line".into(), json!(line));
                }
                if let Some((open, close)) = tokens.block {
                    value.insert("block".into(), json!({"open": open, "close": close}));
                }
                value
            });
        assert_eq!(json!(comments), case["comments"], "{export}: comments");
        assert_eq!(
            json!(data.get::<&[&str]>("closeBrackets")),
            case["brackets"]
        );
        assert_eq!(json!(data.get::<&str>("wordChars")), case["wordChars"]);
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        for probe in case["triggers"].as_array().unwrap() {
            let input = probe[0].as_str().unwrap();
            assert_eq!(
                json!(trigger(input)),
                probe[1],
                "{export}: trigger for {input:?}"
            );
        }
    }
}

#[test]
fn registers_only_the_concrete_upstream_web_exports() {
    for (module, export, expected) in [
        ("javascript", "javascript", LegacyMode::JavaScript),
        ("javascript", "typescript", LegacyMode::TypeScript),
        ("javascript", "json", LegacyMode::Json),
        ("javascript", "jsonld", LegacyMode::JsonLd),
        ("xml", "xml", LegacyMode::Xml),
        ("xml", "html", LegacyMode::Html),
    ] {
        assert_eq!(mode(module, export), Some(expected));
    }
    for (module, export) in [
        ("javascript", "jsx"),
        ("javascript", "tsx"),
        ("javascript", "jsonLd"),
        ("xml", "mkXML"),
        ("htmlmixed", "htmlMixed"),
        ("htmlmixed", "html"),
    ] {
        assert_eq!(mode(module, export), None);
    }
}

#[test]
fn replays_edits_from_open_web_mode_state() {
    let settings = StreamSettings::default();
    for (mode, source, old, new) in [
        (
            LegacyMode::JavaScript,
            "function f(x) {\n  const msg = `text\n    ${x} tail`;\n  return msg;\n}\n",
            "${x} tail",
            "${x + 1}\n    tail",
        ),
        (
            LegacyMode::TypeScript,
            "interface Find {\n  get(x: X): Y;\n}\n",
            "x: X",
            "x: Map<string, X>",
        ),
        (
            LegacyMode::Xml,
            "<root attr=\"first\nsecond\">\n<child/>\n</root>\n",
            "second",
            "second\nthird",
        ),
        (
            LegacyMode::Html,
            "<ul>\n  <li>one\n  <li>two\n</ul>\n",
            "one",
            "one<br>\n  <li>more",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
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
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?}"
        );
        for start in std::iter::once(0).chain(updated.match_indices('\n').map(|(i, _)| i + 1)) {
            assert_eq!(
                replay.indentation(start, Default::default()),
                fresh.indentation(start, Default::default()),
                "{mode:?} at {start}"
            );
        }
    }
}

#[test]
fn copies_scope_and_template_state_without_sharing_changes() {
    let mode = LegacyMode::JavaScript;
    let settings = StreamSettings::default();
    let mut prefix = Tokenizer::new(
        &mode,
        "function f(x) {\n  return `open",
        settings.tab_size,
        settings.indent_unit,
    );
    for token in prefix.by_ref() {
        token.unwrap();
    }
    let mut unclosed = mode.copy_state(prefix.state());
    let mut closed = mode.copy_state(prefix.state());
    for (line, state, expected) in [
        (
            "tail`; }",
            &mut closed,
            vec![Some("string.special"), None, None, None],
        ),
        (
            "${x}`",
            &mut unclosed,
            vec![
                Some("string.special"),
                Some("variableName.local"),
                Some("string.special"),
                Some("string.special"),
            ],
        ),
    ] {
        let mut stream = StringStream::new(line, settings.tab_size, settings.indent_unit);
        let mut styles = Vec::new();
        while !stream.eol() {
            stream.start_token();
            styles.push(
                mode.token(&mut stream, state)
                    .map(|style| style.into_owned()),
            );
            assert!(stream.pos() > stream.start());
        }
        assert_eq!(
            styles.iter().map(Option::as_deref).collect::<Vec<_>>(),
            expected
        );
    }
}
