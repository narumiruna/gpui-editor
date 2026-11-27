//! Oracle: unchanged ESM `@codemirror/legacy-modes/mode/sql` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs
//! match the SHA-512 SRI values in `docs/codemirror-upstream.json`.
//! The scratch oracle called `indent` before each line and recorded every
//! raw token. Ranges are UTF-8 bytes except in the `utf16` contrast case.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::{LanguageData, StreamLanguage, StreamSettings},
    legacy_modes::{
        LegacyMode, LegacySql, LegacySqlConfig, LegacySqlHookResult, LegacySqlSupport, sql,
    },
    stream_parser::{StreamParser, Tokenizer},
};
use serde_json::{Value, json};

const MODULE: &str = "@codemirror/legacy-modes/mode/sql";
const MODES: [(&str, LegacyMode); 13] = [
    ("standardSQL", LegacyMode::StandardSql),
    ("msSQL", LegacyMode::MsSql),
    ("mySQL", LegacyMode::MySql),
    ("mariaDB", LegacyMode::MariaDb),
    ("sqlite", LegacyMode::Sqlite),
    ("cassandra", LegacyMode::Cassandra),
    ("plSQL", LegacyMode::PlSql),
    ("hive", LegacyMode::Hive),
    ("pgSQL", LegacyMode::PgSql),
    ("gql", LegacyMode::Gql),
    ("gpSQL", LegacyMode::GpSql),
    ("sparkSQL", LegacyMode::SparkSql),
    ("esper", LegacyMode::Esper),
];

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/legacy_sql_modes.json")).unwrap()
}

fn line_starts(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(pos, _)| pos + 1))
}

fn settings(case: &Value) -> StreamSettings {
    StreamSettings {
        indent_unit: case["unit"].as_u64().unwrap().try_into().unwrap(),
        tab_size: NonZeroUsize::new(case["tabSize"].as_u64().unwrap().try_into().unwrap()).unwrap(),
    }
}

fn configured() -> LegacySql {
    use LegacySqlHookResult::{Continue, Style, Unstyled};

    sql(LegacySqlConfig {
        client: "run widget priority".into(),
        atoms: "maybe shared".into(),
        builtin: "widget priority".into(),
        keywords: String::from("signal priority shared").into(),
        date_sql: "stamp".into(),
        operator_chars: "~%!".into(),
        brackets: "()[]".into(),
        punctuation: ";,:".into(),
        support: LegacySqlSupport {
            odbc_dot_table: true,
            decimalless_float: true,
            zeroless_float: true,
            binary_number: true,
            hex_number: true,
            double_quote: true,
            n_char_cast: true,
            charset_cast: true,
            escape_constant: true,
            comment_hash: true,
            comment_slash_slash: true,
            comment_space_required: true,
        },
        backslash_string_escapes: false,
        hooks: vec![
            ('!', |stream| {
                stream.eat('@');
                Unstyled
            }),
            ('~', |stream| {
                stream.eat(':');
                Continue
            }),
            ('%', |stream| {
                if stream.eat_while(|ch| ch.is_ascii_uppercase()) {
                    Style("meta")
                } else {
                    Continue
                }
            }),
            ('[', |stream| {
                if stream.eat('=').is_some() {
                    Style("comment")
                } else {
                    Continue
                }
            }),
        ],
    })
}

#[test]
fn registers_the_thirteen_concrete_exports_by_exact_name() {
    let oracle = oracle();
    let exports = oracle["exports"].as_array().unwrap();
    assert_eq!(exports.len(), MODES.len() + 1);
    for export in exports {
        let name = export.as_str().unwrap();
        let mode = LegacyMode::from_export(MODULE, name);
        if name == "sql" {
            assert_eq!(mode, None, "the factory is not a concrete export");
        } else {
            let expected = MODES.iter().find(|(export, _)| *export == name).unwrap().1;
            assert_eq!(mode, Some(expected), "{name}");
        }
    }
    for name in ["StandardSQL", "mysql", "MariaDB", "postgresql", "CQL"] {
        assert_eq!(LegacyMode::from_export(MODULE, name), None, "{name}");
    }
    for module in [
        "@codemirror/lang-sql",
        "@codemirror/legacy-modes/mode/sql.js",
    ] {
        assert_eq!(LegacyMode::from_export(module, "standardSQL"), None);
    }
}

fn assert_metadata(parser: &impl StreamParser, data: &LanguageData, expected: &Value) {
    assert_eq!(json!(parser.name()), expected["name"]);
    assert_eq!(json!(parser.merge_tokens()), expected["mergeTokens"]);
    let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
    assert_eq!(json!(comments.line), expected["commentTokens"]["line"]);
    let (open, close) = comments.block.unwrap();
    assert_eq!(
        json!({"open": open, "close": close}),
        expected["commentTokens"]["block"]
    );
    let brackets = data.get::<&[&str]>("closeBrackets").unwrap();
    assert_eq!(json!(brackets), expected["closeBrackets"]["brackets"]);
}

#[test]
fn matches_language_data_and_merge_preference() {
    let oracle = oracle();
    for (name, mode) in MODES {
        let language = mode.language();
        let doc = language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        let expected = &oracle["metadata"][name];
        assert_eq!(json!(language.name()), expected["name"]);
        assert_metadata(&mode, doc.language_data_at(0).unwrap(), expected);
    }
    for (name, mode) in [
        ("sql", sql(LegacySqlConfig::default())),
        ("configured", configured()),
    ] {
        let language = mode.language();
        let doc = language
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        assert_metadata(
            &mode,
            doc.language_data_at(0).unwrap(),
            &oracle["metadata"][name],
        );
    }
}

fn assert_case<M: StreamParser>(mode: &M, language: &StreamLanguage<M>, case: &Value) {
    let export = case["export"].as_str().unwrap();
    let title = case["title"].as_str().unwrap();
    let source = case["source"].as_str().unwrap();
    let settings = settings(case);
    let mut expected = case["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|line| line.as_array().unwrap());
    for token in Tokenizer::new(mode, source, settings.tab_size, settings.indent_unit) {
        let token = token.unwrap();
        let actual = json!([token.range.start, token.range.end, token.style]);
        assert_eq!(token.text, &source[token.range.clone()]);
        assert_eq!(
            Some(&actual),
            expected.next(),
            "{export}/{title}: {:?} {:?}",
            token.range,
            token.text
        );
    }
    assert_eq!(expected.next(), None, "{export}/{title}: missing tokens");

    let doc = language.parse(Text::new(source), settings).unwrap();
    let indents = line_starts(source)
        .map(|pos| doc.indentation(pos, Default::default()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(json!(indents), case["indents"], "{export}/{title}: indent");
}

#[test]
fn matches_raw_tokens_and_indentation_from_the_pinned_npm_modes() {
    for case in oracle()["cases"].as_array().unwrap() {
        let export = case["export"].as_str().unwrap();
        if !matches!(export, "sql" | "configured") {
            let mode = LegacyMode::from_export(MODULE, export).unwrap();
            assert_case(&mode, mode.language(), case);
        }
    }
}

#[test]
fn matches_the_pinned_factory_with_default_and_custom_options() {
    for case in oracle()["cases"].as_array().unwrap() {
        let mode = match case["export"].as_str().unwrap() {
            "sql" => sql(LegacySqlConfig::default()),
            "configured" => configured(),
            _ => continue,
        };
        assert_case(&mode, &mode.language(), case);
    }
}

#[test]
fn uses_utf8_boundaries_and_scalar_columns_instead_of_utf16() {
    let oracle = oracle();
    let case = &oracle["utf16"];
    let source = case["source"].as_str().unwrap();
    let settings = settings(case);
    let mode = LegacyMode::StandardSql;
    let mut tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit);
    let first = tokens.next().unwrap().unwrap();
    assert_eq!(first.range, 0.."🦀join".len());
    assert_eq!(first.text, "🦀join");
    assert_eq!(first.style, None);
    assert_eq!(case["tokens"][0][0], json!([0, 1, null]));
    assert_eq!(case["tokens"][0][1], json!([1, 6, null]));

    let doc = mode.language().parse(Text::new(source), settings).unwrap();
    let indents = line_starts(source)
        .map(|pos| doc.indentation(pos, Default::default()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(json!(indents), json!([null, 7, 6, null]));
    assert_eq!(case["indents"], json!([null, 8, 7, null]));
}

#[test]
fn replays_edits_from_open_literal_comment_and_indent_contexts() {
    let settings = StreamSettings::default();
    for (mode, source, old, new) in [
        (
            LegacyMode::PgSql,
            "SELECT (E'open\nold'\n)\nSELECT",
            "old'",
            "🦀\\'new\ndone'",
        ),
        (
            LegacyMode::MySql,
            "(\n/* outer /* nested\nold */\n*/ y\n)\n",
            "old */",
            "new\nclose */",
        ),
        (
            LegacyMode::Sqlite,
            "( first\n[\nold\n]\n)\n",
            "old",
            "@猫\n:named",
        ),
    ] {
        let language = mode.language();
        let mut replay = language.parse(Text::new(source), settings).unwrap();
        let start = source.find(old).unwrap();
        let edit = ChangeSet::single(Change::new(start..start + old.len(), new)).unwrap();
        replay.apply_changes(&edit).unwrap();
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
                replay.indentation(start, Default::default()).unwrap(),
                fresh.indentation(start, Default::default()).unwrap(),
                "{mode:?} at {start}"
            );
        }
    }
}
