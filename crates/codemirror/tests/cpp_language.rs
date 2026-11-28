//! Fixtures from the SRI-verified @codemirror/lang-cpp 6.0.3 and @lezer/cpp 1.1.6.
//! The external oracle used @codemirror/state 6.7.6, @codemirror/language 6.13.1,
//! @lezer/lr 1.4.13, @lezer/common 1.5.7, and @lezer/highlight 1.2.5.
//! `syntaxTree`, `highlightTree`, `getIndentation`, and `foldable` produced
//! the expected data; positions map from normalized UTF-16 to original UTF-8.
//! For edit cycles, fresh and edited results agreed for both `cppLanguage`
//! and `cpp()`. See `fixtures/cpp_edit_oracle.mjs` to regenerate the data.
//! No JavaScript is required to run the Rust tests.

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditError, Text,
    commands::CommentTokens,
    language::{
        HighlightTag, IndentOptions, StreamSettings, TagModifier,
        cpp::{
            BLOCK_COMMENT, CppDocument, CppNode, CppNodeKind, LINE_COMMENT, NAME, STRING_PREFIXES,
            indent_on_input,
        },
        lezer::ReuseStats,
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, LoadInvocation, NativeLanguage,
        NativeSyntax,
    },
};
use serde_json::{Value, json};

fn parse(source: &str) -> CppDocument {
    CppDocument::parse(Text::new(source))
}

fn settings() -> StreamSettings {
    StreamSettings {
        indent_unit: 4,
        ..StreamSettings::default()
    }
}

fn tree(node: CppNode<'_>) -> String {
    let name = node.kind().name();
    let name = if node.kind() == CppNodeKind::Error
        || name.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
    {
        name.to_owned()
    } else {
        format!("{name:?}")
    };
    if node.children().len() == 0 {
        name
    } else {
        format!(
            "{name}({})",
            node.children().map(tree).collect::<Vec<_>>().join(",")
        )
    }
}

fn styles(tags: &[HighlightTag]) -> String {
    tags.iter()
        .map(|tag| {
            let mut style = tag.name().to_owned();
            for modifier in tag.modifiers() {
                style.push_str(match modifier {
                    TagModifier::Definition => ".definition",
                    TagModifier::Function => ".function",
                    TagModifier::Special => ".special",
                    TagModifier::Standard => ".standard",
                    _ => panic!("unexpected tag modifier: {modifier:?}"),
                });
            }
            style
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn check_fixture(fixture: &str) {
    let expected: Value = serde_json::from_str(fixture).unwrap();
    let source = expected["doc"].as_str().unwrap();
    let name = expected["name"].as_str().unwrap();
    let doc = parse(source);
    let expected_tree = expected["tree"].as_str().unwrap();
    assert_eq!(tree(doc.root()), expected_tree, "tree: {name}");
    assert_eq!(
        doc.is_recovered(),
        expected_tree.contains('⚠'),
        "recovery: {name}"
    );
    assert_eq!(doc.root().range(), 0..source.len());
    let ranges = doc
        .nodes()
        .map(|node| {
            let range = node.range();
            assert!(source.get(range.clone()).is_some());
            if let Some(parent) = node.parent() {
                assert!(parent.range().start <= range.start && range.end <= parent.range().end);
            }
            format!("{}:{}", range.start, range.end)
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        ranges,
        expected["nodes"].as_str().unwrap(),
        "ranges: {name}"
    );
    let highlights = doc
        .highlights(0..source.len())
        .unwrap()
        .map(|span| {
            format!(
                "{}:{}:{}",
                span.range.start,
                span.range.end,
                styles(span.tags)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        highlights,
        expected["highlights"].as_str().unwrap(),
        "highlights: {name}"
    );
    assert_eq!(
        doc.text().line_count(),
        expected["indent"].as_array().unwrap().len()
    );
    for i in 1..=doc.text().line_count() {
        let line = doc.text().line(i).unwrap();
        assert_eq!(
            doc.indentation(line.range.start, settings(), IndentOptions::default()),
            expected["indent"][i - 1].as_u64().map(|n| n as usize),
            "indentation: {name}, line {i}: {:?}",
            line.text
        );
        assert_eq!(
            doc.foldable(line.range.start, line.range.end)
                .map(|r| format!("{}:{}", r.start, r.end))
                .as_deref(),
            expected["fold"][i - 1].as_str(),
            "fold: {name}, line {i}"
        );
    }
}

#[test]
fn exposes_upstream_language_metadata() {
    assert_eq!(NAME, "cpp");
    assert_eq!(LINE_COMMENT, "//");
    assert_eq!(BLOCK_COMMENT, ("/*", "*/"));
    assert_eq!(
        STRING_PREFIXES,
        ["L", "u", "U", "u8", "LR", "UR", "uR", "u8R", "R"]
    );
    for line in ["case ", "\tcase ", "   default:", "{", "}", "\u{feff}}"] {
        assert!(indent_on_input(line), "{line:?}");
    }
    for line in [
        "case",
        "case  ",
        "case 0:",
        "default",
        "default: ",
        "defaulting:",
        "{}",
        "}\n",
        "\u{85}}",
        "// }",
    ] {
        assert!(!indent_on_input(line), "{line:?}");
    }
}

#[test]
fn c_and_cpp_catalog_entries_share_the_native_cpp_parser() {
    for (name, aliases, extensions) in [
        ("C", &["c"][..], &["c", "h", "ino"][..]),
        (
            "C++",
            &["cpp", "c++"][..],
            &["cpp", "c++", "cc", "cxx", "hpp", "h++", "hh", "hxx"][..],
        ),
    ] {
        let description = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
        assert_eq!(description.name, name);
        assert_eq!(description.aliases, aliases);
        assert_eq!(description.extensions, extensions);
        assert_eq!(description.source.module, "@codemirror/lang-cpp");
        assert_eq!(description.source.export, "cpp");
        assert_eq!(description.source.invocation, LoadInvocation::Call);
        for extension in extensions {
            let filename = format!("unit.{extension}");
            let by_filename = LanguageDescription::match_filename(LANGUAGES, &filename).unwrap();
            assert_eq!(by_filename.name, name);
            assert_eq!(by_filename.built_in(), Some(NativeLanguage::Cpp));
        }
        let support = description.native_support_load().load().unwrap();
        assert_eq!(support.language(), NativeLanguage::Cpp);
        assert_eq!(support.completion_source_capacity(), 0);
        let supported = support.parse(Text::new("int item;")).unwrap();
        let NativeSyntax::Cpp(doc) = supported.syntax() else {
            panic!("expected the C++ parser")
        };
        assert_eq!(doc.root().kind(), CppNodeKind::Program);
        assert!(!doc.is_recovered());
        let data = supported.data_at(0, Assoc::Before);
        assert_eq!(data.len(), 1);
        let comments = data[0].get::<CommentTokens>("commentTokens").unwrap();
        assert_eq!(comments.line, Some(LINE_COMMENT));
        assert_eq!(comments.block, Some(BLOCK_COMMENT));
        assert_eq!(
            data[0].get::<CloseBracketData>("closeBrackets"),
            Some(&CloseBracketData {
                string_prefixes: &STRING_PREFIXES,
                ..Default::default()
            })
        );
        assert!(data[0].get::<&str>("wordChars").is_none());
        let rule = data[0].get::<fn(&str) -> bool>("indentOnInput").unwrap();
        assert!(rule("\tcase "));
        assert!(!rule("case 1:"));
        assert!(supported.completions(0, true).is_empty());
    }
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn assert_edit_snapshot(row: &Value, doc: &CppDocument) {
    let id = row["id"].as_str().unwrap();
    let stage = row["stage"].as_str().unwrap();
    let source = row["source"].as_str().unwrap();
    assert_eq!(doc.text().as_str(), source, "{id} {stage}");
    assert_eq!(doc.is_recovered(), stage == "broken", "{id} {stage}");
    if doc.is_recovered() {
        assert_eq!(doc.reuse_stats(), ReuseStats::default(), "{id} {stage}");
    }
    let nodes: Vec<_> = doc
        .nodes()
        .map(|node| {
            let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
            let range = node.range();
            assert!(source.get(range.clone()).is_some());
            json!([depth, node.kind().name(), range.start, range.end])
        })
        .collect();
    assert_eq!(json!(nodes), row["tree"], "{id} {stage}: tree");
    let spans: Vec<_> = doc
        .highlights(0..source.len())
        .unwrap()
        .map(|span| json!([span.range.start, span.range.end, styles(span.tags)]))
        .collect();
    assert_eq!(json!(spans), row["spans"], "{id} {stage}: highlights");
    let lines = row["lines"].as_array().unwrap();
    assert_eq!(doc.syntax_lines().line_count(), lines.len(), "{id} {stage}");
    assert_eq!(doc.syntax_lines().len(), source.len(), "{id} {stage}");
    for (index, line) in lines.iter().enumerate() {
        let start = byte(&line[0]);
        let end = byte(&line[1]);
        let native = doc.syntax_lines().line(index + 1).unwrap();
        assert_eq!(native.range, start..end, "{id} {stage}: line bounds");
        assert_eq!(native.text, &source[start..end], "{id} {stage}: line text");
        let indent = doc.indentation(start, settings(), IndentOptions::default());
        assert_eq!(
            json!(indent),
            line[2],
            "{id} {stage}: indent at line {start}"
        );
        let fold = doc
            .foldable(start, end)
            .map(|range| [range.start, range.end]);
        assert_eq!(json!(fold), line[3], "{id} {stage}: fold at {start}");
    }
    for point in row["points"].as_array().unwrap() {
        let at = byte(&point[0]);
        for split in [false, true] {
            let options = IndentOptions {
                simulate_break: split.then_some(at),
                simulate_double_break: split,
                ..IndentOptions::default()
            };
            let indent = doc.indentation(at, settings(), options);
            assert_eq!(
                json!(indent),
                point[if split { 2 } else { 1 }],
                "{id} {stage}: indent at {at}, split={split}"
            );
        }
    }
}

fn check_edit_cycles(fixtures: &str, after_edit: impl Fn(&Value, &CppDocument)) {
    let mut previous = None::<CppDocument>;
    for line in fixtures.lines() {
        let mut row: Value = serde_json::from_str(line).unwrap();
        if let Some(pad) = row["pad"].as_u64() {
            let source = row["source"].as_str().unwrap();
            row["source"] = json!(source.replace("{padding}", &" ".repeat(pad as usize)));
        }
        let fresh = parse(row["source"].as_str().unwrap());
        assert_edit_snapshot(&row, &fresh);
        previous = if let Some(edit) = row["edit"].as_array() {
            let range = byte(&edit[0])..byte(&edit[1]);
            let change = ChangeSet::single(Change::new(range, edit[2].as_str().unwrap())).unwrap();
            let edited = previous.unwrap().reparse(&change).unwrap();
            assert_edit_snapshot(&row, &edited);
            after_edit(&row, &edited);
            Some(edited)
        } else {
            Some(fresh)
        };
    }
}

#[test]
fn pinned_syntax_services_match_before_during_and_after_common_edits() {
    check_edit_cycles(include_str!("fixtures/cpp_edit_oracle.jsonl"), |_, _| {});
}

#[test]
fn pinned_syntax_services_match_for_ambiguous_expressions_and_edits() {
    check_edit_cycles(
        include_str!("fixtures/cpp_ambiguity_oracle.jsonl"),
        |_, _| {},
    );
}

#[test]
fn pinned_recovery_at_and_above_the_strict_checkpoint_limit() {
    check_edit_cycles(
        include_str!("fixtures/cpp_large_recovery.jsonl"),
        |row, doc| {
            if row["stage"] != "repaired" {
                return;
            }
            let stats = doc.reuse_stats();
            if row["id"] == "eof-at-limit" {
                let at = stats.resumed_at;
                let edit_start = byte(&row["edit"][0]);
                let source = doc.text().as_str();
                assert!((64..=edit_start).contains(&at), "{stats:?}");
                assert!(stats.nodes_reused > 0, "{stats:?}");
                assert!(source.is_char_boundary(at));
                assert_ne!(&source.as_bytes()[at - 1..=at], b"\r\n");
            } else {
                assert_eq!(stats, ReuseStats::default());
            }
        },
    );
}

#[test]
fn malformed_documents_retain_only_validated_strict_checkpoints_for_repair() {
    let prefix =
        "int one = 1;\r\nint two = 2;\r\nint three = 3;\r\nint four = 4;\r\nint five = 5;\r\n";
    let malformed = parse(&format!("{prefix}int last = 1\r\n// 🧪\r\n"));
    assert!(malformed.is_recovered());
    assert_eq!(malformed.reuse_stats(), ReuseStats::default());
    let semicolon = prefix.len() + "int last = 1".len();
    let repair = ChangeSet::single(Change::new(semicolon..semicolon, ";")).unwrap();
    let fixed = malformed.reparse(&repair).unwrap();
    let stats = fixed.reuse_stats();
    assert!(!fixed.is_recovered());
    assert!((64..=semicolon).contains(&stats.resumed_at), "{stats:?}");
    assert!(stats.nodes_reused > 0, "{stats:?}");
    let source = fixed.text().as_str();
    assert!(source.is_char_boundary(stats.resumed_at));
    assert_ne!(
        source
            .as_bytes()
            .get(stats.resumed_at - 1..=stats.resumed_at),
        Some(&b"\r\n"[..])
    );
    let fresh = parse(source);
    assert!(
        fixed
            .nodes()
            .map(|n| (n.kind(), n.range()))
            .eq(fresh.nodes().map(|n| (n.kind(), n.range())))
    );
    assert!(
        fixed
            .highlights(0..source.len())
            .unwrap()
            .eq(fresh.highlights(0..source.len()).unwrap())
    );

    let astral = malformed.text().as_str().find('🧪').unwrap();
    let invalid = ChangeSet::single(Change::new(astral + 1..astral + 1, "x")).unwrap();
    assert_eq!(
        malformed.reparse(&invalid).unwrap_err(),
        EditError::InvalidRange
    );
    let replace =
        ChangeSet::single(Change::new(0..malformed.text().len(), "int replacement;\n")).unwrap();
    let replaced = malformed.reparse(&replace).unwrap();
    assert_eq!(replaced.reuse_stats(), ReuseStats::default());
    assert!(!replaced.is_recovered());
}

#[test]
fn large_malformed_document_does_not_keep_a_reuse_seed() {
    let prefix = "int value = 1;\r\n".repeat(1_200);
    let broken = parse(&format!("{prefix}int last = 1"));
    assert!(broken.is_recovered());
    assert_eq!(broken.reuse_stats(), ReuseStats::default());
    assert_eq!(
        broken
            .nodes()
            .filter(|node| node.kind() == CppNodeKind::Declaration)
            .count(),
        1_201
    );
    let end = broken.text().len();
    let repair = ChangeSet::single(Change::new(end..end, ";")).unwrap();
    let repaired = broken.reparse(&repair).unwrap();
    assert!(!repaired.is_recovered());
    assert_eq!(repaired.reuse_stats(), ReuseStats::default());
}

#[test]
fn matches_oracle_indent_options() {
    let source = "int f() {\n\treturn call(1,\n\t\tvalue);\n}\n";
    let doc = parse(source);
    let at = source.find("1,").unwrap();
    let line = doc.text().line_at(at).unwrap().range.start;
    let override_indent = |p| (p == line).then_some(10);
    let indent = |options| doc.indentation(at, settings(), options);
    assert_eq!(indent(IndentOptions::default()), Some(16));
    assert_eq!(
        indent(IndentOptions {
            override_indentation: Some(&override_indent),
            ..IndentOptions::default()
        }),
        Some(22)
    );
    for double in [false, true] {
        assert_eq!(
            indent(IndentOptions {
                simulate_break: Some(at),
                simulate_double_break: double,
                ..IndentOptions::default()
            }),
            Some(8)
        );
    }
}

#[test]
fn validates_byte_queries_and_clips_spans() {
    let source = "/* 😀\r\n * inside\r\n */\r\n";
    let doc = parse(source);
    let at = source.find('😀').unwrap();
    let spans = doc.highlights(at..at + 4).unwrap().collect::<Vec<_>>();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, at..at + 4);
    assert_eq!(spans[0].tags[0].name(), "blockComment");
    assert!(doc.highlights(at + 1..at + 4).is_none());
    assert!(doc.highlights(at..at).unwrap().next().is_none());
    assert!(
        doc.indentation(at + 1, settings(), IndentOptions::default())
            .is_none()
    );
    assert!(doc.foldable(0, at + 1).is_none());
}

#[test]
fn keeps_ranges_valid_in_truncated_utf8_documents() {
    let source = "int f() {\r\n  auto a = R\"m(😀)m\";\r\n  /* 😀 */\r\n  return 0;\r\n}\r\n";
    for end in source
        .char_indices()
        .map(|(at, _)| at)
        .chain([source.len()])
    {
        let prefix = &source[..end];
        let doc = parse(prefix);
        assert_eq!(doc.root().range(), 0..end);
        for node in doc.nodes() {
            assert!(prefix.get(node.range()).is_some());
            if let Some(range) = node.fold_range() {
                assert!(prefix.get(range).is_some());
            }
        }
    }
}

#[test]
fn matches_oracle_c_declarations() {
    check_fixture(
        r###"
{"name":"c_declarations","doc":"#include <stdio.h>\n#define SUM(a, b) ((a) + \\\n  (b))\n#ifndef ENABLED\nint values[] = {\n  1, 2,\n  3\n};\n#endif\nstruct Point {\n  int x;\n  int y;\n};\nenum Kind {\n  first,\n  second\n};\nint main(void) {\n  struct Point p = {.x = 1, .y = 2};\n  return SUM(p.x, p.y);\n}\n","tree":"Program(PreprocDirective(\"#include\",SystemLibString),PreprocDirective(\"#define\",Identifier,\"(\",Identifier,\",\",Identifier,\")\",PreprocArg),PreprocDirective(\"#ifndef\",Identifier),Declaration(PrimitiveType,InitDeclarator(ArrayDeclarator(Identifier,\"[\",\"]\"),InitializerList(\"{\",Number,\",\",Number,\",\",Number,\"}\"))),PreprocDirective(\"#endif\"),StructSpecifier(struct,TypeIdentifier,FieldDeclarationList(\"{\",FieldDeclaration(PrimitiveType,FieldIdentifier),FieldDeclaration(PrimitiveType,FieldIdentifier),\"}\")),EnumSpecifier(enum,TypeIdentifier,EnumeratorList(\"{\",Enumerator(Identifier),\",\",Enumerator(Identifier),\"}\")),FunctionDefinition(PrimitiveType,FunctionDeclarator(Identifier,ParameterList(\"(\",ParameterDeclaration(PrimitiveType),\")\")),CompoundStatement(\"{\",Declaration(StructSpecifier(struct,TypeIdentifier),InitDeclarator(Identifier,InitializerList(\"{\",InitializerPair(FieldDesignator(FieldIdentifier),Number),\",\",InitializerPair(FieldDesignator(FieldIdentifier),Number),\"}\"))),ReturnStatement(return,CallExpression(Identifier,ArgumentList(\"(\",FieldExpression(Identifier,FieldIdentifier),\",\",FieldExpression(Identifier,FieldIdentifier),\")\"))),\"}\")))","nodes":"0:257 0:19 0:8 9:18 19:53 19:26 27:30 30:31 31:32 32:33 34:35 35:36 36:52 53:69 53:60 61:68 69:100 69:72 73:99 73:81 73:79 79:80 80:81 84:99 84:85 88:89 89:90 91:92 92:93 96:97 98:99 101:108 101:107 108:142 108:114 115:120 121:142 121:122 125:131 125:128 129:130 134:140 134:137 138:139 141:142 144:175 144:148 149:153 154:175 154:155 158:163 158:163 163:164 167:173 167:173 174:175 177:256 177:180 181:191 181:185 185:191 185:186 186:190 186:190 190:191 192:256 192:193 196:230 196:208 196:202 203:208 209:229 209:210 213:229 213:214 214:220 214:216 215:216 219:220 220:221 222:228 222:224 223:224 227:228 228:229 233:254 233:239 240:253 240:243 243:253 243:244 244:247 244:245 246:247 247:248 249:252 249:250 251:252 252:253 255:256","highlights":"0:8:processingInstruction 9:18:string.special 19:26:processingInstruction 27:30:variableName 30:31:paren 31:32:variableName 32:33:separator 34:35:variableName 35:36:paren 36:52:meta 53:60:processingInstruction 61:68:variableName 69:72:typeName.standard 73:79:variableName 79:80:squareBracket 80:81:squareBracket 84:85:brace 88:89:number 89:90:separator 91:92:number 92:93:separator 96:97:number 98:99:brace 101:107:processingInstruction 108:114:definitionKeyword 115:120:typeName 121:122:brace 125:128:typeName.standard 129:130:propertyName 134:137:typeName.standard 138:139:propertyName 141:142:brace 144:148:definitionKeyword 149:153:typeName 154:155:brace 158:163:variableName 163:164:separator 167:173:variableName 174:175:brace 177:180:typeName.standard 181:185:variableName.definition.function 185:186:paren 186:190:typeName.standard 190:191:paren 192:193:brace 196:202:definitionKeyword 203:208:typeName 209:210:variableName 213:214:brace 215:216:propertyName 219:220:number 220:221:separator 223:224:propertyName 227:228:number 228:229:brace 233:239:controlKeyword 240:243:variableName.function 243:244:paren 244:245:variableName 246:247:propertyName 247:248:separator 249:250:variableName 251:252:propertyName 252:253:paren 255:256:brace","indent":[0,0,0,0,0,4,4,0,0,0,4,4,0,0,4,4,0,0,4,4,0,0],"fold":[null,null,null,null,"85:98",null,null,null,null,"122:141",null,null,null,"155:174",null,null,null,"193:255",null,null,null,null]}
"###,
    );
}

#[test]
fn matches_oracle_cpp_templates() {
    check_fixture(
        r###"
{"name":"cpp_templates","doc":"namespace library {\ntemplate<class T> struct Box {\n  T value;\n  T get() const { return value; }\n};\n}\nlibrary::Box<library::Box<int>> box;\nint g(int left, int right) {\n  return (left >> right) + (left > right) + box.value.get();\n}\n","tree":"Program(NamespaceDefinition(namespace,Identifier,CompoundStatement(\"{\",TemplateDeclaration(template,TemplateParameterList(\"<\",TypeParameterDeclaration(class,TypeIdentifier),\">\"),StructSpecifier(struct,TypeIdentifier,FieldDeclarationList(\"{\",FieldDeclaration(TypeIdentifier,FieldIdentifier),FunctionDefinition(TypeIdentifier,FunctionDeclarator(FieldIdentifier,ParameterList(\"(\",\")\"),const),CompoundStatement(\"{\",ReturnStatement(return,Identifier),\"}\")),\"}\"))),\"}\")),Declaration(TemplateType(ScopedTypeIdentifier(NamespaceIdentifier,\"::\",TypeIdentifier),TemplateArgumentList(\"<\",TypeDescriptor(TemplateType(ScopedTypeIdentifier(NamespaceIdentifier,\"::\",TypeIdentifier),TemplateArgumentList(\"<\",TypeDescriptor(PrimitiveType),\">\"))),\">\")),Identifier),FunctionDefinition(PrimitiveType,FunctionDeclarator(Identifier,ParameterList(\"(\",ParameterDeclaration(PrimitiveType,Identifier),\",\",ParameterDeclaration(PrimitiveType,Identifier),\")\")),CompoundStatement(\"{\",ReturnStatement(return,BinaryExpression(BinaryExpression(ParenthesizedExpression(\"(\",BinaryExpression(Identifier,BitOp,Identifier),\")\"),ArithOp,ParenthesizedExpression(\"(\",BinaryExpression(Identifier,CompareOp,Identifier),\")\")),ArithOp,CallExpression(FieldExpression(FieldExpression(Identifier,FieldIdentifier),FieldIdentifier),ArgumentList(\"(\",\")\")))),\"}\")))","nodes":"0:230 0:100 0:9 10:17 18:100 18:19 20:98 20:28 28:37 28:29 29:36 29:34 35:36 36:37 38:97 38:44 45:48 49:97 49:50 53:61 53:54 55:60 64:95 64:65 66:77 66:69 69:71 69:70 70:71 72:77 78:95 78:79 80:93 80:86 87:92 94:95 96:97 99:100 101:137 101:132 101:113 101:108 108:110 110:113 113:132 113:114 114:131 114:131 114:126 114:121 121:123 123:126 126:131 126:127 127:130 127:130 130:131 131:132 133:136 138:229 138:141 142:164 142:143 143:164 143:144 144:152 144:147 148:152 152:153 154:163 154:157 158:163 163:164 165:229 165:166 169:227 169:175 176:226 176:208 176:191 176:177 177:190 177:181 182:184 185:190 190:191 192:193 194:208 194:195 195:207 195:199 200:201 202:207 207:208 209:210 211:226 211:224 211:220 211:214 215:220 221:224 224:226 224:225 225:226 228:229","highlights":"0:9:definitionKeyword 10:17:variableName 18:19:brace 20:28:definitionKeyword 28:29:angleBracket 29:34:definitionKeyword 35:36:typeName 36:37:angleBracket 38:44:definitionKeyword 45:48:typeName 49:50:brace 53:54:typeName 55:60:propertyName 64:65:typeName 66:69:propertyName 69:70:paren 70:71:paren 72:77:modifier 78:79:brace 80:86:controlKeyword 87:92:variableName 94:95:brace 96:97:brace 99:100:brace 101:108:namespace 110:113:typeName 113:114:angleBracket 114:121:namespace 123:126:typeName 126:127:angleBracket 127:130:typeName.standard 130:131:angleBracket 131:132:angleBracket 133:136:variableName 138:141:typeName.standard 142:143:variableName.definition.function 143:144:paren 144:147:typeName.standard 148:152:variableName 152:153:separator 154:157:typeName.standard 158:163:variableName 163:164:paren 165:166:brace 169:175:controlKeyword 176:177:paren 177:181:variableName 182:184:bitwiseOperator 185:190:variableName 190:191:paren 192:193:arithmeticOperator 194:195:paren 195:199:variableName 200:201:compareOperator 202:207:variableName 207:208:paren 209:210:arithmeticOperator 211:214:variableName 215:220:propertyName 221:224:propertyName.function 224:225:paren 225:226:paren 228:229:brace","indent":[0,4,4,4,0,0,0,0,4,0,0],"fold":["19:99","50:96",null,null,null,null,null,"166:228",null,null,null]}
"###,
    );
}

#[test]
fn matches_oracle_control() {
    check_fixture(
        r###"
{"name":"control","doc":"int f(int x) {\nlabel:\n  if (x > 0)\n    x += 2;\n  else\n    x = 0;\n  switch (x) {\n    case 0:\n      break;\n    default:\n      return x;\n  }\n  try {\n    return compute(1,\n                   x);\n  } catch (...) {\n    throw;\n  }\n}\n","tree":"Program(FunctionDefinition(PrimitiveType,FunctionDeclarator(Identifier,ParameterList(\"(\",ParameterDeclaration(PrimitiveType,Identifier),\")\")),CompoundStatement(\"{\",LabeledStatement(StatementIdentifier,IfStatement(if,ConditionClause(\"(\",BinaryExpression(Identifier,CompareOp,Number),\")\"),ExpressionStatement(AssignmentExpression(Identifier,UpdateOp,Number)),else,ExpressionStatement(AssignmentExpression(Identifier,Number)))),SwitchStatement(switch,ConditionClause(\"(\",Identifier,\")\"),CompoundStatement(\"{\",CaseStatement(case,Number,BreakStatement(break)),CaseStatement(default,ReturnStatement(return,Identifier)),\"}\")),TryStatement(try,CompoundStatement(\"{\",ReturnStatement(return,CallExpression(Identifier,ArgumentList(\"(\",Number,\",\",Identifier,\")\"))),\"}\"),CatchClause(catch,ParameterList(\"(\",\")\"),CompoundStatement(\"{\",ThrowStatement(throw),\"}\"))),\"}\")))","nodes":"0:226 0:225 0:3 4:12 4:5 5:12 5:6 6:11 6:9 10:11 11:12 13:225 13:14 15:64 15:20 24:64 24:26 27:34 27:28 28:33 28:29 30:31 32:33 33:34 39:46 39:45 39:40 41:43 44:45 49:53 58:64 58:63 58:59 62:63 67:137 67:73 74:77 74:75 75:76 76:77 78:137 78:79 84:104 84:88 89:90 98:104 98:103 109:133 109:116 124:133 124:130 131:132 136:137 140:223 140:143 144:194 144:145 150:190 150:156 157:189 157:164 164:189 164:165 165:166 166:167 187:188 188:189 193:194 195:223 195:200 201:206 201:202 205:206 207:223 207:208 213:219 213:218 222:223 224:225","highlights":"0:3:typeName.standard 4:5:variableName.definition.function 5:6:paren 6:9:typeName.standard 10:11:variableName 11:12:paren 13:14:brace 15:20:labelName 24:26:controlKeyword 27:28:paren 28:29:variableName 30:31:compareOperator 32:33:number 33:34:paren 39:40:variableName 41:43:updateOperator 44:45:number 49:53:controlKeyword 58:59:variableName 62:63:number 67:73:controlKeyword 74:75:paren 75:76:variableName 76:77:paren 78:79:brace 84:88:controlKeyword 89:90:number 98:103:controlKeyword 109:116:controlKeyword 124:130:controlKeyword 131:132:variableName 136:137:brace 140:143:controlKeyword 144:145:brace 150:156:controlKeyword 157:164:variableName.function 164:165:paren 165:166:number 166:167:separator 187:188:variableName 188:189:paren 193:194:brace 195:200:controlKeyword 201:202:paren 205:206:paren 207:208:brace 213:218:controlKeyword 222:223:brace 224:225:brace","indent":[0,4,0,6,2,6,4,6,8,6,8,2,4,6,19,2,6,2,0,0],"fold":["14:224",null,null,null,null,null,"79:136",null,null,null,null,null,"145:193",null,null,"208:222",null,null,null,null]}
"###,
    );
}

#[test]
fn matches_oracle_strings_crlf() {
    check_fixture(
        r###"
{"name":"strings_crlf","doc":"/* 😀\r\n * inside\r\n */\r\nauto a = u8R\"delim(raw 😀)delim\";\r\nauto b = L\"escaped\\nwide\";\r\nauto c = 'x';\r\nauto d = 12_ms;\r\n#include \"local.h\"\r\n#pragma once\r\n","tree":"Program(BlockComment,Declaration(auto,InitDeclarator(Identifier,RawString)),Declaration(auto,InitDeclarator(Identifier,String(EscapeSequence))),Declaration(auto,InitDeclarator(Identifier,CharLiteral)),Declaration(auto,InitDeclarator(Identifier,UserDefinedLiteral(Number,Identifier))),PreprocDirective(\"#include\",String),PreprocDirective(PreprocDirectiveName,PreprocArg))","nodes":"0:156 0:23 25:60 25:29 30:59 30:31 34:59 62:88 62:66 67:87 67:68 71:87 80:82 90:103 90:94 95:102 95:96 99:102 105:120 105:109 110:119 110:111 114:119 114:116 116:119 122:142 122:130 131:140 142:156 142:149 149:154","highlights":"0:23:blockComment 25:29:definitionKeyword 30:31:variableName 34:59:string.special 62:66:definitionKeyword 67:68:variableName 71:80:string 80:82:escape 82:87:string 90:94:definitionKeyword 95:96:variableName 99:102:character 105:109:definitionKeyword 110:111:variableName 114:116:number 116:119:literal 122:130:processingInstruction 131:140:string 142:149:processingInstruction 149:154:meta","indent":[0,null,null,0,0,0,0,0,0,0],"fold":["2:21",null,null,null,null,null,null,null,null,null]}
"###,
    );
}

#[test]
fn matches_oracle_cpp20() {
    check_fixture(
        r###"
{"name":"cpp20","doc":"export module example:part;\nimport widget;\ntemplate <typename T> concept HasSize = requires(T value) { value.size(); };\nauto f(int x) {\n  auto lambda = [x](int y) { return x + y; };\n  co_return lambda(2);\n}\n","tree":"Program(ExportDeclaration(export,ModuleDeclaration(module,ModuleName(Identifier,PartitionName))),ImportDeclaration(import,ModuleName(Identifier)),TemplateDeclaration(template,TemplateParameterList(\"<\",TypeParameterDeclaration(typename,TypeIdentifier),\">\"),ConceptDefinition(concept,Identifier,RequiresExpression(requires,ParameterList(\"(\",ParameterDeclaration(TypeIdentifier,Identifier),\")\"),RequirementList(\"{\",SimpleRequirement(CallExpression(FieldExpression(Identifier,FieldIdentifier),ArgumentList(\"(\",\")\"))),\"}\")))),FunctionDefinition(auto,FunctionDeclarator(Identifier,ParameterList(\"(\",ParameterDeclaration(PrimitiveType,Identifier),\")\")),CompoundStatement(\"{\",Declaration(auto,InitDeclarator(Identifier,LambdaExpression(LambdaCaptureSpecifier(\"[\",Identifier,\"]\"),AbstractFunctionDeclarator(ParameterList(\"(\",ParameterDeclaration(PrimitiveType,Identifier),\")\")),CompoundStatement(\"{\",ReturnStatement(return,BinaryExpression(Identifier,ArithOp,Identifier)),\"}\")))),CoReturnStatement(co_return,CallExpression(Identifier,ArgumentList(\"(\",Number,\")\"))),\"}\")))","nodes":"0:207 0:27 0:6 7:27 7:13 14:26 14:21 22:26 28:42 28:34 35:41 35:41 43:119 43:51 52:64 52:53 53:63 53:61 62:63 63:64 65:119 65:72 73:80 83:118 83:91 91:100 91:92 92:99 92:93 94:99 99:100 101:118 101:102 103:116 103:115 103:113 103:108 109:113 113:115 113:114 114:115 117:118 120:206 120:124 125:133 125:126 126:133 126:127 127:132 127:130 131:132 132:133 134:206 134:135 138:181 138:142 143:180 143:149 152:180 152:155 152:153 153:154 154:155 155:162 155:162 155:156 156:161 156:159 160:161 161:162 163:180 163:164 165:178 165:171 172:177 172:173 174:175 176:177 179:180 184:204 184:193 194:203 194:200 200:203 200:201 201:202 202:203 205:206","highlights":"0:6:definitionKeyword 7:13:definitionKeyword 14:21:namespace 22:26:labelName 28:34:definitionKeyword 35:41:namespace 43:51:definitionKeyword 52:53:angleBracket 53:61:definitionKeyword 62:63:typeName 63:64:angleBracket 65:72:definitionKeyword 73:80:variableName 83:91:definitionKeyword 91:92:paren 92:93:typeName 94:99:variableName 99:100:paren 101:102:brace 103:108:variableName 109:113:propertyName.function 113:114:paren 114:115:paren 117:118:brace 120:124:definitionKeyword 125:126:variableName.definition.function 126:127:paren 127:130:typeName.standard 131:132:variableName 132:133:paren 134:135:brace 138:142:definitionKeyword 143:149:variableName 152:153:squareBracket 153:154:variableName 154:155:squareBracket 155:156:paren 156:159:typeName.standard 160:161:variableName 161:162:paren 163:164:brace 165:171:controlKeyword 172:173:variableName 174:175:arithmeticOperator 176:177:variableName 179:180:brace 184:193:controlKeyword 194:200:variableName.function 200:201:paren 201:202:number 202:203:paren 205:206:brace","indent":[0,0,0,0,4,4,0,0],"fold":[null,null,null,"135:205",null,null,null,null]}
"###,
    );
}

#[test]
fn matches_oracle_missing_block() {
    check_fixture(
        r###"
{"name":"missing_block","doc":"int f() {\n  return 1;\n","tree":"Program(FunctionDefinition(PrimitiveType,FunctionDeclarator(Identifier,ParameterList(\"(\",\")\")),CompoundStatement(\"{\",ReturnStatement(return,Number),⚠)))","nodes":"0:22 0:22 0:3 4:7 4:5 5:7 5:6 6:7 8:22 8:9 12:21 12:18 19:20 22:22","highlights":"0:3:typeName.standard 4:5:variableName.definition.function 5:6:paren 6:7:paren 8:9:brace 12:18:controlKeyword 19:20:number","indent":[0,4,4],"fold":["9:22",null,null]}
"###,
    );
}

#[test]
fn matches_oracle_missing_comment() {
    check_fixture(
        r###"
{"name":"missing_comment","doc":"int a;\n/* note\n  unfinished text","tree":"Program(Declaration(PrimitiveType,Identifier),BlockComment(⚠))","nodes":"0:32 0:6 0:3 4:5 7:32 32:32","highlights":"0:3:typeName.standard 4:5:variableName 7:32:blockComment","indent":[0,0,null],"fold":[null,"9:30",null]}
"###,
    );
}

#[test]
fn matches_oracle_missing_semicolon() {
    check_fixture(
        r###"
{"name":"missing_semicolon","doc":"int a = 1","tree":"Program(Declaration(PrimitiveType,InitDeclarator(Identifier,Number),⚠))","nodes":"0:9 0:9 0:3 4:9 4:5 8:9 9:9","highlights":"0:3:typeName.standard 4:5:variableName 8:9:number","indent":[0],"fold":[null]}
"###,
    );
}

#[test]
fn matches_oracle_missing_directive_end() {
    check_fixture(
        r###"
{"name":"missing_directive_end","doc":"#include <foo>","tree":"Program(PreprocDirective(\"#include\",SystemLibString,⚠))","nodes":"0:14 0:14 0:8 9:14 14:14","highlights":"0:8:processingInstruction 9:14:string.special","indent":[0],"fold":[null]}
"###,
    );
}

#[test]
fn matches_oracle_malformed() {
    check_fixture(
        r###"
{"name":"malformed","doc":"int a;\n@\nint b;\n","tree":"Program(Declaration(PrimitiveType,Identifier),⚠,Declaration(PrimitiveType,Identifier))","nodes":"0:16 0:6 0:3 4:5 7:8 9:15 9:12 13:14","highlights":"0:3:typeName.standard 4:5:variableName 9:12:typeName.standard 13:14:variableName","indent":[0,0,0,0],"fold":[null,null,null,null]}
"###,
    );
}

#[test]
fn matches_oracle_missing_call() {
    check_fixture(
        r###"
{"name":"missing_call","doc":"foo(a, b","tree":"Program(ExpressionStatement(CallExpression(Identifier,ArgumentList(\"(\",Identifier,\",\",Identifier,⚠)),⚠))","nodes":"0:8 0:8 0:8 0:3 3:8 3:4 4:5 5:6 7:8 8:8 8:8","highlights":"0:3:variableName.function 3:4:paren 4:5:variableName 5:6:separator 7:8:variableName","indent":[0],"fold":[null]}
"###,
    );
}

#[test]
fn matches_oracle_external_tokens() {
    check_fixture(
        r###"
{"name":"external_tokens","doc":"REGISTER_WIDGET(Widget)\nstruct __declspec(align) Record { int num; };\nauto a = R\"m(x )nope\" y)m\";\nauto b = LR\"(wide)\";\nauto c = UR\"(wide)\";\nauto d = uR\"(wide)\";\nauto e = u8R\"🔑(emoji)🔑\";\n","tree":"Program(Macro(MacroName,ArgumentList(\"(\",Identifier,\")\")),StructSpecifier(struct,MsDeclspecModifier(__declspec,\"(\",Identifier,\")\"),TypeIdentifier,FieldDeclarationList(\"{\",FieldDeclaration(PrimitiveType,FieldIdentifier),\"}\")),Declaration(auto,InitDeclarator(Identifier,RawString)),Declaration(auto,InitDeclarator(Identifier,RawString)),Declaration(auto,InitDeclarator(Identifier,RawString)),Declaration(auto,InitDeclarator(Identifier,RawString)),Declaration(auto,InitDeclarator(Identifier,RawString)))","nodes":"0:192 0:23 0:15 15:23 15:16 16:22 22:23 24:68 24:30 31:48 31:41 41:42 42:47 47:48 49:55 56:68 56:57 58:66 58:61 62:65 67:68 70:97 70:74 75:96 75:76 79:96 98:118 98:102 103:117 103:104 107:117 119:139 119:123 124:138 124:125 128:138 140:160 140:144 145:159 145:146 149:159 161:191 161:165 166:190 166:167 170:190","highlights":"0:15:name.special 15:16:paren 16:22:variableName 22:23:paren 24:30:definitionKeyword 31:41:definitionKeyword 41:42:paren 42:47:variableName 47:48:paren 49:55:typeName 56:57:brace 58:61:typeName.standard 62:65:propertyName 67:68:brace 70:74:definitionKeyword 75:76:variableName 79:96:string.special 98:102:definitionKeyword 103:104:variableName 107:117:string.special 119:123:definitionKeyword 124:125:variableName 128:138:string.special 140:144:definitionKeyword 145:146:variableName 149:159:string.special 161:165:definitionKeyword 166:167:variableName 170:190:string.special","indent":[0,0,0,0,0,0,0,0],"fold":[null,null,null,null,null,null,null,null]}
"###,
    );
}
