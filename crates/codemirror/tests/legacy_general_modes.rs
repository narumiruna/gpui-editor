//! Oracle: unchanged `@codemirror/legacy-modes` 6.5.5 and `StringStream`
//! from `@codemirror/streamparser` 6.0.0, run outside the repository after
//! verifying both SHA-512 digests against `docs/codemirror-upstream.json`.
//! The fixture records raw styles and UTF-8 byte ranges. Same-style surrogate
//! halves form one scalar token. Indentation uses four-column tabs and an
//! indent unit of two, with final-state probes at units two and three.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_general_modes.json");
const EXPORTS: [(&str, &str, LegacyMode); 5] = [
    ("haxe", "haxe", LegacyMode::Haxe),
    ("haxe", "hxml", LegacyMode::Hxml),
    ("spreadsheet", "spreadsheet", LegacyMode::Spreadsheet),
    ("ttcn", "ttcn", LegacyMode::Ttcn),
    ("ttcn-cfg", "ttcnCfg", LegacyMode::TtcnCfg),
];

#[test]
fn resolves_published_exports_by_exact_id() {
    for (path, export, mode) in EXPORTS {
        let module = format!("{PREFIX}{path}");
        assert_eq!(LegacyMode::from_export(&module, export), Some(mode));
        assert_eq!(
            LegacyMode::from_export(&module, &export.to_uppercase()),
            None
        );
        assert_eq!(
            LegacyMode::from_export(&format!("{module}.js"), export),
            None
        );
        for (other_path, other_export, _) in EXPORTS {
            if other_path != path {
                assert_eq!(LegacyMode::from_export(&module, other_export), None);
            }
        }
    }
}

#[test]
fn matches_pinned_tokens_and_indentation() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (path, export, mode) in EXPORTS {
        let id = format!("{path}#{export}");
        for (label, case) in oracle["modes"][&id]["cases"].as_object().unwrap() {
            let source = case["source"].as_str().unwrap();
            let mut expected = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|line| line.as_array().unwrap());
            let mut scan = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
            for token in scan.by_ref() {
                let token = token.unwrap_or_else(|err| panic!("{id}/{label}: {err}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                assert_eq!(
                    Some(&json!([token.range.start, token.range.end, token.style])),
                    expected.next(),
                    "{id}/{label}: {:?}",
                    token.text,
                );
            }
            assert!(expected.next().is_none(), "{id}/{label}: missing tokens");

            let last = oracle["after"]
                .as_array()
                .unwrap()
                .iter()
                .map(|after| {
                    [2, 3].map(|unit| mode.indent(scan.state(), after.as_str().unwrap(), unit))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(last),
                case["after"],
                "{id}/{label}: final-state indentation"
            );

            let doc = mode.language().parse(Text::new(source), settings).unwrap();
            let indents = line_starts(source)
                .map(|position| doc.indentation(position, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{id}/{label}: line indentation"
            );
        }
    }
}

#[test]
fn matches_pinned_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    for (path, export, mode) in EXPORTS {
        let id = format!("{path}#{export}");
        let expected = &oracle["modes"][&id]["data"];
        assert_eq!(json!(mode.name()), expected["name"], "{id}");
        assert_eq!(json!(mode.language().name()), expected["name"], "{id}");
        assert_eq!(json!(mode.merge_tokens()), expected["merge"], "{id}");
        let doc = mode
            .language()
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let data = doc.language_data_at(0).unwrap();
        let comments = data
            .get::<CommentTokens<'static>>("commentTokens")
            .map(|value| {
                let mut comments = json!({"line": value.line});
                if let Some((open, close)) = value.block {
                    comments["block"] = json!({"open": open, "close": close});
                }
                comments
            });
        assert_eq!(json!(comments), expected["comments"], "{id}");
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            expected["autocomplete"],
            "{id}"
        );
        let triggers = data
            .get::<fn(&str) -> bool>("indentOnInput")
            .map(|predicate| {
                oracle["triggers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| predicate(input.as_str().unwrap()))
                    .collect::<Vec<_>>()
            });
        assert_eq!(json!(triggers), expected["triggers"], "{id}");
    }
}

#[test]
fn replays_edits_from_saved_multiline_state() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Haxe,
            "class C {\n function f(arg:Int) {\n  var n = arg;\n  return n;\n }\n}\n",
            "var n = arg;",
            "var other = arg;\n  var n = other;",
        ),
        (
            LegacyMode::Hxml,
            "-cmd 'open\n  next'\n--flag\n",
            "next'",
            "more\n  close'",
        ),
        (
            LegacyMode::Spreadsheet,
            "=SUM([item\n next], B2)\n",
            "next]",
            "more\n next]",
        ),
        (
            LegacyMode::Ttcn,
            "module M { /* open\n close */ function f() {}\n}\n",
            "close */",
            "more\n close */",
        ),
        (
            LegacyMode::TtcnCfg,
            "LogFile := 'open\n close'H\nFileMask := LOG_ALL\n",
            "close'",
            "more\n close'",
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
        for position in line_starts(&updated) {
            assert_eq!(
                replay.indentation(position, Default::default()),
                fresh.indentation(position, Default::default()),
                "{mode:?} at {position}"
            );
        }
    }
}

#[test]
fn spreadsheet_empty_class_retries_at_the_same_cursor() {
    let mode = LegacyMode::Spreadsheet;
    let settings = StreamSettings::default();
    let mut state = mode.start_state(settings.indent_unit);
    let mut stream = StringStream::new("[]", settings.tab_size, settings.indent_unit);
    assert_eq!(
        mode.token(&mut stream, &mut state).as_deref(),
        Some("bracket")
    );
    stream.start_token();
    assert_eq!(
        mode.token(&mut stream, &mut state).as_deref(),
        Some("operator")
    );
    assert_eq!(stream.start(), stream.pos());
    assert_eq!(
        mode.token(&mut stream, &mut state).as_deref(),
        Some("bracket")
    );
    assert!(stream.eol());
}

#[test]
fn spreadsheet_strings_advance_through_unicode_line_separators() {
    let settings = StreamSettings::default();
    let mode = LegacyMode::Spreadsheet;
    for source in ["\"\u{2028}\"", "'\u{2029}'"] {
        let mut tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
        let token = tokens.next().unwrap().unwrap();
        assert_eq!(token.text, source);
        assert_eq!(token.style.as_deref(), Some("string"));
        assert!(tokens.next().is_none());
    }
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(position, _)| position + 1))
}
