//! Oracle: unchanged `@codemirror/legacy-modes` 6.5.5 exports and
//! `@codemirror/streamparser` 6.0.0 `StringStream`, from tarballs verified
//! against the SRI digests in `docs/codemirror-upstream.json`. The scratch
//! harness queried each line's indentation before tokenizing it, with indent
//! unit 2 and tab size 4. It records raw styles and UTF-8 byte ranges.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_data_modes.json");

fn mode(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_the_pinned_data_mode_oracle() {
    let settings = StreamSettings::default();
    for case in serde_json::from_str::<Vec<Value>>(ORACLE).unwrap() {
        let title = case["title"].as_str().unwrap();
        let module = case["module"].as_str().unwrap();
        let export = case["export"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = mode(module, export).unwrap();
        assert_eq!(json!(mode.name()), case["name"]);
        assert_eq!(json!(mode.merge_tokens()), case["merge"]);
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
                "{export}/{title} at {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert!(expected.next().is_none(), "{export}/{title}");

        let language = mode.language();
        assert_eq!(language.name(), case["name"].as_str().unwrap());
        let doc = language.parse(Text::new(source), settings).unwrap();
        let indents: Vec<_> = line_starts(source)
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect();
        assert_eq!(json!(indents), case["indents"], "{export}/{title}");
        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens");
        assert_eq!(
            json!(comments.map(|tokens| json!({"line": tokens.line}))),
            case["comments"]
        );
        assert!(comments.is_none_or(|tokens| tokens.block.is_none()));
    }
}

#[test]
fn resolves_only_the_present_exports_by_exact_name() {
    for (id, expected) in [
        ("yaml", LegacyMode::Yaml),
        ("turtle", LegacyMode::Turtle),
        ("ebnf", LegacyMode::Ebnf),
    ] {
        assert_eq!(mode(id, id), Some(expected));
    }
    for (module, export) in [
        ("yaml", "YAML"),
        ("yaml-frontmatter", "yamlFrontmatter"),
        ("markdown", "markdown"),
        ("edn", "edn"),
        ("ebnf", "EBNF"),
        ("turtle", "Turtle"),
    ] {
        assert_eq!(mode(module, export), None);
    }
}

#[test]
fn replays_edits_inside_open_data_mode_state() {
    let settings = StreamSettings::default();
    for (mode, source, old, new) in [
        (
            LegacyMode::Yaml,
            "body: |\n  first\n  second\nnext: true\n",
            "  second",
            "second: false\n  third",
        ),
        (
            LegacyMode::Turtle,
            "ex:s {\n  [ ex:t\n    \"first\n    second\"\n  ]\n}\n",
            "second\"",
            "second\" .\n    <new>",
        ),
        (
            LegacyMode::Ebnf,
            "rule: (* first\n  second\n  last *) name;\n",
            "second",
            "second *)\n  [a-z] (* third",
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
        for start in line_starts(&updated) {
            assert_eq!(
                replay.indentation(start, Default::default()),
                fresh.indentation(start, Default::default()),
                "{mode:?} at {start}",
            );
        }
    }
}

#[test]
fn copies_turtle_context_without_sharing_changes() {
    let mode = LegacyMode::Turtle;
    let settings = StreamSettings::default();
    let mut tokenizer = Tokenizer::new(
        &mode,
        "ex:s { ex:p",
        settings.tab_size,
        settings.indent_unit,
    );
    for token in tokenizer.by_ref() {
        token.unwrap();
    }
    let original = tokenizer.state();
    let mut copy = mode.copy_state(original);
    let mut stream = StringStream::new("}", settings.tab_size, settings.indent_unit);
    mode.token(&mut stream, &mut copy);
    assert_eq!(mode.indent(&copy, "next", settings.indent_unit), Some(0));
    assert_eq!(mode.indent(original, "next", settings.indent_unit), Some(7));
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(at, _)| at + 1))
}
