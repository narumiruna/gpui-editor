// Node reference: @codemirror/lang-sql 6.10.0, @lezer/lr 1.4.13,
// @lezer/highlight 1.2.5, @codemirror/language 6.13.1, and
// @codemirror/commands 6.11.2 (`selectParentSyntax`).
// Tree and highlight offsets were converted from UTF-16 to UTF-8 bytes.
use std::{fmt::Write, ops::Range};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, SelectionRange, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        sql::{SqlDialect, SqlDialectSpec, SqlDocument, SqlLanguage, SqlNode, SqlNodeKind},
    },
};

struct Fixture<'a> {
    dialect: &'a str,
    source: &'a str,
    recovered: bool,
    nodes: &'a str,
    highlights: &'a str,
}

const SYNTAX: &[Fixture<'_>] = &[
    Fixture {
        dialect: "StandardSQL",
        source: r###"-- note
SELECT schema.t.col, (x + [2]), {true}, null;
/* tail */ SELECT "c", ?.k, B'01', X'dA', .25e-2 FROM t"###,
        recovered: false,
        nodes: "0:Script:0-109 1:LineComment:0-7 1:Statement:8-53 2:Keyword:8-14 2:CompositeIdentifier:15-27 3:Identifier:15-21 \
            3:.:21-22 3:Identifier:22-23 3:.:23-24 3:Identifier:24-27 2:Punctuation:27-28 2:Parens:29-38 3:(:29-30 \
            3:Identifier:30-31 3:Operator:32-33 3:Brackets:34-37 4:[:34-35 4:Number:35-36 4:]:36-37 3:):37-38 2:Punctuation:38-39 \
            2:Braces:40-46 3:{:40-41 3:Bool:41-45 3:}:45-46 2:Punctuation:46-47 2:Null:48-52 2:;:52-53 1:BlockComment:54-64 \
            1:Statement:65-109 2:Keyword:65-71 2:QuotedIdentifier:72-75 2:Punctuation:75-76 2:CompositeIdentifier:77-80 \
            3:SpecialVar:77-78 3:.:78-79 3:Identifier:79-80 2:Punctuation:80-81 2:Bits:82-87 2:Punctuation:87-88 2:Number:89-94 \
            2:Punctuation:94-95 2:Number:96-102 2:Keyword:103-107 2:Identifier:108-109",
        highlights: "lineComment:0-7 keyword:8-14 name:15-21 name:22-23 name:24-27 punctuation:27-28 paren:29-30 name:30-31 operator:32-33 \
            squareBracket:34-35 number:35-36 squareBracket:36-37 paren:37-38 punctuation:38-39 brace:40-41 bool:41-45 brace:45-46 \
            punctuation:46-47 null:48-52 blockComment:54-64 keyword:65-71 string.special:72-75 punctuation:75-76 name.special:77-78 \
            name:79-80 punctuation:80-81 number:82-87 punctuation:87-88 number:89-94 punctuation:94-95 number:96-102 keyword:103-107 \
            name:108-109",
    },
    Fixture {
        dialect: "StandardSQL",
        source: r###"SELECT a /*one*/ . /*two*/ SELECT, .schema.name, .x, a.;
-- done"###,
        recovered: true,
        nodes: "0:Script:0-64 1:Statement:0-56 2:Keyword:0-6 2:CompositeIdentifier:7-27 3:Identifier:7-8 3:BlockComment:9-16 3:.:17-18 \
            3:BlockComment:19-26 3:⚠:27-27 2:Keyword:27-33 2:Punctuation:33-34 2:CompositeIdentifier:35-47 3:.:35-36 \
            3:Identifier:36-42 3:.:42-43 3:Identifier:43-47 2:Punctuation:47-48 2:CompositeIdentifier:49-51 3:.:49-50 \
            3:Identifier:50-51 3:⚠:51-51 2:Punctuation:51-52 2:CompositeIdentifier:53-55 3:Identifier:53-54 3:.:54-55 3:⚠:55-55 \
            2:;:55-56 1:LineComment:57-64",
        highlights: "keyword:0-6 name:7-8 blockComment:9-16 blockComment:19-26 keyword:27-33 punctuation:33-34 name:36-42 name:43-47 \
            punctuation:47-48 name:50-51 punctuation:51-52 name:53-54 lineComment:57-64",
    },
    Fixture {
        dialect: "PostgreSQL",
        source: r###"SELECT $tag$💖 ; $tag$, $$x$$, E'a\'b', n's', _utf8'z', a::jsonb FROM t ILIKE 'x'"###,
        recovered: false,
        nodes: "0:Script:0-83 1:Statement:0-83 2:Keyword:0-6 2:String:7-24 2:Punctuation:24-25 2:String:26-31 2:Punctuation:31-32 \
            2:String:33-40 2:Punctuation:40-41 2:String:42-46 2:Punctuation:46-47 2:String:48-56 2:Punctuation:56-57 \
            2:Identifier:58-59 2:Punctuation:59-60 2:Punctuation:60-61 2:Type:61-66 2:Keyword:67-71 2:Identifier:72-73 \
            2:Keyword:74-79 2:String:80-83",
        highlights: "keyword:0-6 string:7-24 punctuation:24-25 string:26-31 punctuation:31-32 string:33-40 punctuation:40-41 string:42-46 \
            punctuation:46-47 string:48-56 punctuation:56-57 name:58-59 punctuation:59-60 punctuation:60-61 typeName:61-66 \
            keyword:67-71 name:72-73 keyword:74-79 string:80-83",
    },
    Fixture {
        dialect: "MySQL",
        source: r###"select `x`, "s", n'a', 0b010, 0B101, @@foo, group_concat, charset, --no
# hi
select -- yes
longblob"###,
        recovered: false,
        nodes: "0:Script:0-99 1:Statement:0-99 2:Keyword:0-6 2:QuotedIdentifier:7-10 2:Punctuation:10-11 2:String:12-15 \
            2:Punctuation:15-16 2:String:17-21 2:Punctuation:21-22 2:Bits:23-28 2:Punctuation:28-29 2:Number:30-31 \
            2:Identifier:31-35 2:Punctuation:35-36 2:SpecialVar:37-42 2:Punctuation:42-43 2:Keyword:44-56 2:Punctuation:56-57 \
            2:Builtin:58-65 2:Punctuation:65-66 2:Operator:67-69 2:Keyword:69-71 2:LineComment:72-76 2:Keyword:77-83 \
            2:LineComment:84-90 2:Type:91-99",
        highlights: "keyword:0-6 string.special:7-10 punctuation:10-11 string:12-15 punctuation:15-16 string:17-21 punctuation:21-22 \
            number:23-28 punctuation:28-29 number:30-31 name:31-35 punctuation:35-36 name.special:37-42 punctuation:42-43 \
            keyword:44-56 punctuation:56-57 name.standard:58-65 punctuation:65-66 operator:67-69 keyword:69-71 lineComment:72-76 \
            keyword:77-83 lineComment:84-90 typeName:91-99",
    },
    Fixture {
        dialect: "MariaSQL",
        source: r###"select always, virtual, group_concat, groupby_concat, print # hi"###,
        recovered: false,
        nodes: "0:Script:0-64 1:Statement:0-59 2:Keyword:0-6 2:Keyword:7-13 2:Punctuation:13-14 2:Keyword:15-22 2:Punctuation:22-23 \
            2:Identifier:24-36 2:Punctuation:36-37 2:Keyword:38-52 2:Punctuation:52-53 2:Builtin:54-59 1:LineComment:60-64",
        highlights: "keyword:0-6 keyword:7-13 punctuation:13-14 keyword:15-22 punctuation:22-23 name:24-36 punctuation:36-37 keyword:38-52 \
            punctuation:52-53 name.standard:54-59 lineComment:60-64",
    },
    Fixture {
        dialect: "MSSQL",
        source: r###"select top 2 [a b], "q", @@version, cast, dateadd, nvarchar, null, ["###,
        recovered: false,
        nodes: "0:Script:0-68 1:Statement:0-68 2:Keyword:0-6 2:Keyword:7-10 2:Number:11-12 2:QuotedIdentifier:13-18 2:Punctuation:18-19 \
            2:QuotedIdentifier:20-23 2:Punctuation:23-24 2:SpecialVar:25-34 2:Punctuation:34-35 2:Builtin:36-40 2:Punctuation:40-41 \
            2:Builtin:42-49 2:Punctuation:49-50 2:Type:51-59 2:Punctuation:59-60 2:Keyword:61-65 2:Punctuation:65-66 \
            2:QuotedIdentifier:67-68",
        highlights: "keyword:0-6 keyword:7-10 number:11-12 string.special:13-18 punctuation:18-19 string.special:20-23 punctuation:23-24 \
            name.special:25-34 punctuation:34-35 name.standard:36-40 punctuation:40-41 name.standard:42-49 punctuation:49-50 \
            typeName:51-59 punctuation:59-60 keyword:61-65 punctuation:65-66 string.special:67-68",
    },
    Fixture {
        dialect: "SQLite",
        source: r###"pragma `x`, @x, :y, ?123, $foo, [z], true, blob, explain"###,
        recovered: false,
        nodes: "0:Script:0-56 1:Statement:0-56 2:Keyword:0-6 2:QuotedIdentifier:7-10 2:Punctuation:10-11 2:SpecialVar:12-14 \
            2:Punctuation:14-15 2:SpecialVar:16-18 2:Punctuation:18-19 2:SpecialVar:20-24 2:Punctuation:24-25 2:SpecialVar:26-30 \
            2:Punctuation:30-31 2:Brackets:32-35 3:[:32-33 3:Identifier:33-34 3:]:34-35 2:Punctuation:35-36 2:Bool:37-41 \
            2:Punctuation:41-42 2:Type:43-47 2:Punctuation:47-48 2:Builtin:49-56",
        highlights: "keyword:0-6 string.special:7-10 punctuation:10-11 name.special:12-14 punctuation:14-15 name.special:16-18 \
            punctuation:18-19 name.special:20-24 punctuation:24-25 name.special:26-30 punctuation:30-31 squareBracket:32-33 \
            name:33-34 squareBracket:34-35 punctuation:35-36 bool:37-41 punctuation:41-42 typeName:43-47 punctuation:47-48 \
            name.standard:49-56",
    },
    Fixture {
        dialect: "Cassandra",
        source: r###"consistency local_quorum select frozen, nan, NaN, Infinity // end
/* a /* b */ c */ Type"###,
        recovered: false,
        nodes: "0:Script:0-88 1:Statement:0-88 2:Keyword:0-11 2:Keyword:12-24 2:Keyword:25-31 2:Type:32-38 2:Punctuation:38-39 \
            2:Keyword:40-43 2:Punctuation:43-44 2:Keyword:45-48 2:Punctuation:48-49 2:Keyword:50-58 2:LineComment:59-65 \
            2:BlockComment:66-83 2:Keyword:84-88",
        highlights: "keyword:0-11 keyword:12-24 keyword:25-31 typeName:32-38 punctuation:38-39 keyword:40-43 punctuation:43-44 keyword:45-48 \
            punctuation:48-49 keyword:50-58 lineComment:59-65 blockComment:66-83 keyword:84-88",
    },
    Fixture {
        dialect: "PLSQL",
        source: r###"select q'[a''b]', Q'!x!', "s", n'n', varchar2, rownum, spool"###,
        recovered: false,
        nodes: "0:Script:0-60 1:Statement:0-60 2:Keyword:0-6 2:String:7-16 2:Punctuation:16-17 2:String:18-24 2:Punctuation:24-25 \
            2:String:26-29 2:Punctuation:29-30 2:String:31-35 2:Punctuation:35-36 2:Type:37-45 2:Punctuation:45-46 2:Keyword:47-53 \
            2:Punctuation:53-54 2:Builtin:55-60",
        highlights: "keyword:0-6 string:7-16 punctuation:16-17 string:18-24 punctuation:24-25 string:26-29 punctuation:29-30 string:31-35 \
            punctuation:35-36 typeName:37-45 punctuation:45-46 keyword:47-53 punctuation:53-54 name.standard:55-60",
    },
    Fixture {
        dialect: "StandardSQL",
        source: r###"select café, 🦀, _name, a., .z, (x
  FROM t;
select ] 1;"###,
        recovered: true,
        nodes: "0:Script:0-59 1:Statement:0-47 2:Keyword:0-6 2:Identifier:7-10 2:⚠:10-12 2:Punctuation:12-13 2:⚠:14-18 \
            2:Punctuation:18-19 2:⚠:20-21 2:Identifier:21-25 2:Punctuation:25-26 2:CompositeIdentifier:27-29 3:Identifier:27-28 \
            3:.:28-29 3:⚠:29-29 2:Punctuation:29-30 2:CompositeIdentifier:31-33 3:.:31-32 3:Identifier:32-33 3:⚠:33-33 \
            2:Punctuation:33-34 2:Parens:35-46 3:(:35-36 3:Identifier:36-37 3:Keyword:40-44 3:Identifier:45-46 3:⚠:46-46 2:;:46-47 \
            1:Statement:48-59 2:Keyword:48-54 2:⚠:55-56 3:]:55-56 2:Number:57-58 2:;:58-59",
        highlights: "keyword:0-6 name:7-10 punctuation:12-13 punctuation:18-19 name:21-25 punctuation:25-26 name:27-28 punctuation:29-30 \
            name:32-33 punctuation:33-34 paren:35-36 name:36-37 keyword:40-44 name:45-46 keyword:48-54 squareBracket:55-56 \
            number:57-58",
    },
    Fixture {
        dialect: "StandardSQL",
        source: r###"SELECT (a,
 b), -- inner
  [c,
  d
 ],
 {
  e
 }
FROM t;

/* heading
 inner
 */
SELECT (
"###,
        recovered: true,
        nodes: "0:Script:0-89 1:Statement:0-56 2:Keyword:0-6 2:Parens:7-14 3:(:7-8 3:Identifier:8-9 3:Punctuation:9-10 \
            3:Identifier:12-13 3:):13-14 2:Punctuation:14-15 2:LineComment:16-24 2:Brackets:27-37 3:[:27-28 3:Identifier:28-29 \
            3:Punctuation:29-30 3:Identifier:33-34 3:]:36-37 2:Punctuation:37-38 2:Braces:40-48 3:{:40-41 3:Identifier:44-45 \
            3:}:47-48 2:Keyword:49-53 2:Identifier:54-55 2:;:55-56 1:BlockComment:58-79 1:Statement:80-89 2:Keyword:80-86 \
            2:Parens:87-89 3:(:87-88 3:⚠:89-89",
        highlights: "keyword:0-6 paren:7-8 name:8-9 punctuation:9-10 name:12-13 paren:13-14 punctuation:14-15 lineComment:16-24 \
            squareBracket:27-28 name:28-29 punctuation:29-30 name:33-34 squareBracket:36-37 punctuation:37-38 brace:40-41 name:44-45 \
            brace:47-48 keyword:49-53 name:54-55 blockComment:58-79 keyword:80-86 paren:87-88",
    },
    Fixture {
        dialect: "StandardSQL",
        source: r###"SELECT schema.t.col, (x + 2) FROM t;
SELECT 3;"###,
        recovered: false,
        nodes: "0:Script:0-46 1:Statement:0-36 2:Keyword:0-6 2:CompositeIdentifier:7-19 3:Identifier:7-13 3:.:13-14 3:Identifier:14-15 \
            3:.:15-16 3:Identifier:16-19 2:Punctuation:19-20 2:Parens:21-28 3:(:21-22 3:Identifier:22-23 3:Operator:24-25 \
            3:Number:26-27 3:):27-28 2:Keyword:29-33 2:Identifier:34-35 2:;:35-36 1:Statement:37-46 2:Keyword:37-43 2:Number:44-45 \
            2:;:45-46",
        highlights: "keyword:0-6 name:7-13 name:14-15 name:16-19 punctuation:19-20 paren:21-22 name:22-23 operator:24-25 number:26-27 \
            paren:27-28 keyword:29-33 name:34-35 keyword:37-43 number:44-45",
    },
    Fixture {
        dialect: "StandardSQL",
        source: r###"select 'abc''a', 1e+, "x""y", B'012', B"12", X'1g', 0xZ;"###,
        recovered: false,
        nodes: "0:Script:0-56 1:Statement:0-56 2:Keyword:0-6 2:String:7-12 2:String:12-15 2:Punctuation:15-16 2:Number:17-20 \
            2:Punctuation:20-21 2:QuotedIdentifier:22-25 2:QuotedIdentifier:25-28 2:Punctuation:28-29 2:Bits:30-34 2:Number:34-35 \
            2:String:35-47 2:Number:47-48 2:Identifier:48-49 2:String:49-56",
        highlights: "keyword:0-6 string:7-12 string:12-15 punctuation:15-16 number:17-20 punctuation:20-21 string.special:22-25 \
            string.special:25-28 punctuation:28-29 number:30-34 number:34-35 string:35-47 number:47-48 name:48-49 string:49-56",
    },
    Fixture {
        dialect: "PostgreSQL",
        source: r###"select $1, _foo, _a'b', a::int, d.$$x$$, _u8'foo'"###,
        recovered: true,
        nodes: "0:Script:0-49 1:Statement:0-49 2:Keyword:0-6 2:⚠:7-8 2:Number:8-9 2:Punctuation:9-10 2:⚠:11-12 2:Identifier:12-15 \
            2:Punctuation:15-16 2:⚠:17-18 2:Identifier:18-19 2:String:19-22 2:Punctuation:22-23 2:Identifier:24-25 \
            2:Punctuation:25-26 2:Punctuation:26-27 2:Type:27-30 2:Punctuation:30-31 2:CompositeIdentifier:32-34 3:Identifier:32-33 \
            3:.:33-34 3:⚠:34-34 2:String:34-39 2:Punctuation:39-40 2:String:41-49",
        highlights: "keyword:0-6 number:8-9 punctuation:9-10 name:12-15 punctuation:15-16 name:18-19 string:19-22 punctuation:22-23 \
            name:24-25 punctuation:25-26 punctuation:26-27 typeName:27-30 punctuation:30-31 name:32-33 string:34-39 \
            punctuation:39-40 string:41-49",
    },
    Fixture {
        dialect: "Custom",
        source: r###"select foo BAR baz mytype t true false null unknown # a
// b
--c
-- d
$foo 'x\'y' 0b01 B'1z' q'<x>'"###,
        recovered: false,
        nodes: "0:Script:0-99 1:Statement:0-99 2:Identifier:0-6 2:Builtin:7-10 2:Identifier:11-14 2:Type:15-18 2:Type:19-25 \
            2:Builtin:26-27 2:Bool:28-32 2:Bool:33-38 2:Type:39-43 2:Null:44-51 2:LineComment:52-55 2:LineComment:56-60 \
            2:Operator:61-63 2:Identifier:63-64 2:LineComment:65-69 2:SpecialVar:70-74 2:String:75-81 2:Bits:82-86 2:Bytes:87-92 \
            2:String:93-99",
        highlights: "name:0-6 name.standard:7-10 name:11-14 typeName:15-18 typeName:19-25 name.standard:26-27 bool:28-32 bool:33-38 \
            typeName:39-43 null:44-51 lineComment:52-55 lineComment:56-60 operator:61-63 name:63-64 lineComment:65-69 \
            name.special:70-74 string:75-81 number:82-86 string:87-92 string:93-99",
    },
];

fn append_node(out: &mut String, node: SqlNode<'_>, depth: usize) {
    let range = node.range();
    write!(
        out,
        " {depth}:{}:{}-{}",
        node.kind().name(),
        range.start,
        range.end
    )
    .unwrap();
    for child in node.children() {
        assert_eq!(child.parent().unwrap().range(), range);
        append_node(out, child, depth + 1);
    }
}

#[test]
fn syntax_and_highlights_match_published_dialects() {
    let custom = SqlLanguage::define(SqlDialectSpec {
        keywords: "foo BAR baz",
        types: "baz mytype null",
        builtin: "foo t",
        backslash_escapes: true,
        hash_comments: true,
        slash_comments: true,
        space_after_dashes: true,
        unquoted_bit_literals: true,
        treat_bits_as_bytes: true,
        plsql_quoting_mechanism: true,
        special_var: "$",
        ..SqlDialectSpec::DEFAULT
    });
    for (index, fixture) in SYNTAX.iter().enumerate() {
        let text = Text::new(fixture.source);
        let language =
            SqlDialect::from_export(fixture.dialect).map_or(&custom, |dialect| dialect.language());
        let document = language.parse(text);
        let mut actual = String::new();
        append_node(&mut actual, document.root(), 0);
        assert_eq!(
            actual.trim(),
            fixture.nodes,
            "node fixture {index}: {} / {:?}",
            fixture.dialect,
            document.parse_error()
        );
        assert_eq!(
            document.is_recovered(),
            fixture.recovered,
            "recovery fixture {index}: {} / {:?}",
            fixture.dialect,
            document.parse_error()
        );
        let ranges = document
            .highlights(0..fixture.source.len())
            .unwrap()
            .map(|span| {
                let [tag] = span.tags else {
                    panic!("one tag per SQL token")
                };
                let mod_name = match tag.modifiers() {
                    [] => "",
                    [TagModifier::Special] => ".special",
                    [TagModifier::Standard] => ".standard",
                    mods => panic!("unexpected {mods:?}"),
                };
                format!(
                    "{}{mod_name}:{}-{}",
                    tag.name(),
                    span.range.start,
                    span.range.end
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            ranges, fixture.highlights,
            "highlight fixture {index}: {}",
            fixture.dialect
        );
    }
}

#[test]
fn keyword_metadata_matches_upstream_order_labels_and_types() {
    let cases = [
        ("StandardSQL", 295, 0xfd7abcb3df16cded_u64),
        ("PostgreSQL", 831, 0x57971090bd7ef7cd_u64),
        ("MySQL", 559, 0x8bebab0e2e926c9_u64),
        ("MariaSQL", 566, 0xe91f96c4117a3a38_u64),
        ("MSSQL", 729, 0x3e1d73838d0d4bcc_u64),
        ("SQLite", 385, 0x55828fa957317dde_u64),
        ("Cassandra", 127, 0xaddd2deb137fbad2_u64),
        ("PLSQL", 503, 0xd1180289b521b7dd_u64),
    ];
    for (name, count, expected) in cases {
        let words = SqlDialect::from_export(name).unwrap().language().words();
        assert_eq!(words.len(), count, "{name}");
        let bytes = words.iter().flat_map(|word| {
            word.label
                .bytes()
                .chain([0])
                .chain(word.completion_type().bytes())
                .chain([b'\n'])
        });
        let actual = bytes.fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(actual, expected, "{name}");
    }
    assert_eq!(SqlDialect::from_export("postgresql"), None);
    let custom = SqlLanguage::define(SqlDialectSpec {
        types: "only_type",
        case_insensitive_identifiers: true,
        ..SqlDialectSpec::DEFAULT
    });
    assert!(custom.case_insensitive_identifiers());
    assert!(!custom.words().iter().any(|word| word.label == "only_type"));
    assert_eq!(custom.words(), SqlDialect::StandardSql.language().words());
}

#[test]
fn statement_and_comment_folds_and_indent_match_reference() {
    type LineFixture = (Range<usize>, usize, Option<Range<usize>>);
    let cases: &[(usize, &[LineFixture])] = &[
        (
            3,
            &[
                (0..71, 0, Some(71..99)),
                (72..76, 2, None),
                (77..90, 2, None),
                (91..99, 2, None),
            ],
        ),
        (7, &[(0..65, 0, Some(65..88)), (66..88, 2, None)]),
        (
            10,
            &[
                (0..10, 0, Some(10..56)),
                (11..24, 2, None),
                (25..30, 2, None),
                (31..34, 2, None),
                (35..38, 2, None),
                (39..41, 2, None),
                (42..45, 2, None),
                (46..48, 2, None),
                (49..56, 2, None),
                (57..57, 0, None),
                (58..68, 0, Some(60..77)),
                (69..75, 0, None),
                (76..79, 0, None),
                (80..88, 0, Some(88..89)),
                (89..89, 2, None),
            ],
        ),
    ];
    for (index, lines) in cases {
        let fixture = &SYNTAX[*index];
        let doc = SqlDocument::parse_with_dialect(
            Text::new(fixture.source),
            SqlDialect::from_export(fixture.dialect).unwrap(),
        );
        for (line, indent, fold) in *lines {
            assert_eq!(
                doc.indentation(
                    line.start,
                    StreamSettings::default(),
                    IndentOptions::default()
                ),
                Some(*indent),
                "fixture {index}, line {line:?}"
            );
            assert_eq!(
                doc.foldable(line.start, line.end),
                *fold,
                "fixture {index}, line {line:?}"
            );
        }
    }
}

#[test]
fn parent_selection_matches_the_pinned_command() {
    let doc = SqlDocument::parse(Text::new(SYNTAX[11].source));
    const FIRST_STATEMENT: Range<usize> = 0..36;
    let cases: &[(Range<usize>, &[Range<usize>])] = &[
        (16..16, &[16..19, 7..19, 0..36]),
        (17..17, &[16..19, 7..19, 0..36]),
        (16..19, &[7..19, 0..36]),
        (7..7, &[7..13, 7..19, 0..36]),
        (8..8, &[7..13, 7..19, 0..36]),
        (7..13, &[7..19, 0..36]),
        (21..21, &[21..22, 21..28, 0..36]),
        (22..22, &[22..23, 21..28, 0..36]),
        (21..23, &[21..28, 0..36]),
        (26..26, &[26..27, 21..28, 0..36]),
        (27..27, &[27..28, 21..28, 0..36]),
        (26..28, &[21..28, 0..36]),
        (33..33, &[29..33, 0..36]),
        (34..34, &[34..35, 0..36]),
        (33..36, &[FIRST_STATEMENT]),
        (0..0, &[0..6, FIRST_STATEMENT]),
        (1..1, &[0..6, FIRST_STATEMENT]),
        (0..6, &[FIRST_STATEMENT]),
    ];
    for (initial, expansions) in cases {
        let mut range = initial.clone();
        for expected in *expansions {
            range = doc.parent_selection(range).unwrap();
            assert_eq!(range, *expected, "start {initial:?}");
        }
        assert_eq!(doc.parent_selection(range), None);
    }
    let selection = EditorSelection::new(
        vec![SelectionRange::new(17, 17), SelectionRange::new(22, 22)],
        1,
    )
    .unwrap();
    let expanded = doc.select_parent_syntax(&selection).unwrap();
    assert_eq!(
        expanded.ranges(),
        &[SelectionRange::new(16, 19), SelectionRange::new(22, 23)]
    );
    assert_eq!(expanded.main_index(), 1);
}

#[test]
fn byte_queries_validate_and_clip_without_normalizing_text() {
    let source = "-- 🦀\r\nSELECT 'é'\r\n  FROM t;";
    let doc = SqlDocument::parse(Text::new(source));
    assert!(!doc.is_recovered(), "{:?}", doc.parse_error());
    assert_eq!(doc.root().range(), 0..source.len());
    assert_eq!(doc.text().as_str(), source);
    assert!(doc.highlights(4..7).is_none());
    assert!(doc.parent_selection(4..4).is_none());
    assert!(
        doc.select_parent_syntax(&EditorSelection::single(4, 4))
            .is_none()
    );
    assert!(
        doc.indentation(4, StreamSettings::default(), IndentOptions::default())
            .is_none()
    );
    assert_eq!(doc.highlights(3..7).unwrap().next().unwrap().range, 3..7);
    assert_eq!(doc.highlights(7..7).unwrap().count(), 0);
    let statement = doc
        .nodes()
        .find(|node| node.kind() == SqlNodeKind::Statement)
        .unwrap();
    assert_eq!(statement.range().start, 9);
    assert_eq!(doc.foldable(9, 20), Some(20..source.len()));
}

#[test]
fn fold_cutoffs_use_utf16_units_and_keep_scalar_boundaries() {
    // The published statement fold starts 100 UTF-16 units in. The first
    // source puts that offset inside the crab glyph, so use its byte start.
    for (source, expected) in [
        (format!("SELECT '{}🦀y'\nFROM t;", "x".repeat(91)), 99),
        (format!("SELECT '{}🦀yyyy'\nFROM t;", "x".repeat(89)), 102),
    ] {
        let line_end = source.find('\n').unwrap();
        let end = source.len();
        let doc = SqlDocument::parse(Text::new(source));
        assert_eq!(doc.foldable(0, line_end), Some(expected..end));
    }
    let doc = SqlDocument::parse(Text::new("/* foo\n fin🦀"));
    assert_eq!(doc.foldable(0, 6), Some(2..11));
    let doc = SqlDocument::parse(Text::new("/* foo\n fini🦀z"));
    assert_eq!(doc.foldable(0, 6), Some(2..12));
}

#[test]
fn reparsing_reuses_the_strict_dialect_scanner_prefix() {
    let dialect = SqlDialect::PostgreSql;
    let language = dialect.language();
    let prefix = "select $q$🦀$q$, e'word' from app.items;\r\n".repeat(192);
    let mut document = language.parse(Text::new(format!("{prefix}select '字';")));
    assert!(!document.is_recovered());
    for (before, after) in [("字", "長文"), ("長文", "🧪")] {
        let start = document.text().as_str().rfind(before).unwrap();
        let change = ChangeSet::single(Change::new(start..start + before.len(), after)).unwrap();
        document = language.reparse(&document, &change).unwrap();
        let fresh = language.parse(document.text().clone());
        assert!(!document.is_recovered());
        let reused = document.reuse_stats();
        assert!(reused.resumed_at > prefix.len() / 2);
        assert!(reused.resumed_at <= start);
        assert!(reused.nodes_reused > 1000);
        let mut actual = String::new();
        let mut expected = String::new();
        append_node(&mut actual, document.root(), 0);
        append_node(&mut expected, fresh.root(), 0);
        assert_eq!(actual, expected);
        let range = 0..fresh.text().len();
        assert!(
            document
                .highlights(range.clone())
                .unwrap()
                .eq(fresh.highlights(range).unwrap())
        );
        assert!(document.text().as_str().starts_with(&prefix));
    }
    let other_parser = SqlLanguage::define(dialect.spec());
    let reparsed = other_parser
        .reparse(&document, &ChangeSet::default())
        .unwrap();
    assert_eq!(reparsed.reuse_stats().resumed_at, 0);
    assert_eq!(reparsed.text(), document.text());
}

#[test]
fn incomplete_input_keeps_ordered_ranges_and_later_statements() {
    for fixture in SYNTAX {
        let Some(dialect) = SqlDialect::from_export(fixture.dialect) else {
            continue;
        };
        for (end, _) in fixture.source.char_indices() {
            let doc = SqlDocument::parse_with_dialect(Text::new(&fixture.source[..end]), dialect);
            for node in doc.nodes() {
                let range = node.range();
                assert!(doc.text().slice(range.clone()).is_some());
                let mut last_end = range.start;
                for child in node.children() {
                    let child = child.range();
                    assert!(last_end <= child.start && child.end <= range.end);
                    last_end = child.end;
                }
            }
        }
    }
    let source = format!("SELECT {}🦀;\nSELECT 2;", "(".repeat(2_000));
    let doc = SqlDocument::parse(Text::new(source));
    assert!(doc.is_recovered());
    assert_eq!(doc.root().children().count(), 2);
    let last = doc.root().children().next_back().unwrap();
    assert_eq!(doc.text().slice(last.range()), Some("SELECT 2;"));
}
