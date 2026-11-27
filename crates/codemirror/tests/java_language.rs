//! Exact fixtures from the SRI-verified `@codemirror/lang-java` 6.0.2 and
//! `@lezer/java` 1.2.1 tarballs. The oracle ran locally outside the repository
//! with `@codemirror/language` 6.13.1 and `@lezer/lr` 1.4.13. It converted
//! UTF-16 offsets to UTF-8 bytes. For CRLF, it used the raw parse tree and
//! mapped the editor's normalized line positions back to the original bytes.

use std::{num::NonZeroUsize, ops::Range};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditorState, Text, Transaction,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        java::{self, JavaDocument, class_content_parser, parser},
        lezer::{ParseError, ReuseStats, SyntaxNode, TreeFragment},
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
    },
};

fn row(name: &str, range: Range<usize>, depth: usize) -> String {
    format!("{depth}:{name}:{}..{}", range.start, range.end)
}

fn span(range: Range<usize>, tags: &str) -> String {
    format!("{}..{}:{tags}", range.start, range.end)
}

fn compare(actual: impl Iterator<Item = String>, expected: &str) {
    assert_eq!(
        actual.collect::<Vec<_>>(),
        expected.split_whitespace().collect::<Vec<_>>()
    );
}

fn assert_document(source: &str, recovered: bool, tree: &str, highlights: &str, lines: &str) {
    let doc = JavaDocument::parse(Text::new(source));
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(doc.is_recovered(), recovered, "{:?}", doc.parse_error());
    compare(
        doc.nodes().map(|node| {
            let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
            row(node.name(), node.range(), depth)
        }),
        tree,
    );
    compare(
        doc.highlights(0..source.len()).unwrap().map(|highlight| {
            let [tag] = highlight.tags else {
                panic!("one tag per Java span")
            };
            let suffix = match tag.modifiers() {
                [] => "",
                [TagModifier::Definition] => ".definition",
                [TagModifier::Function] => ".function",
                [TagModifier::Standard] => ".standard",
                modifiers => panic!("unexpected modifiers: {modifiers:?}"),
            };
            span(highlight.range, &format!("{}{suffix}", tag.name()))
        }),
        highlights,
    );
    compare(
        (1..=doc.text().line_count()).map(|number| {
            let line = doc.text().line(number).unwrap();
            let (start, end) = (line.range.start, line.range.end);
            let indent = doc
                .indentation(start, StreamSettings::default(), IndentOptions::default())
                .map_or_else(|| "-".to_owned(), |col| col.to_string());
            let fold = doc.foldable(start, end).map_or_else(
                || "-".to_owned(),
                |range| format!("{}..{}", range.start, range.end),
            );
            format!("{start}..{end}:{indent}:{fold}")
        }),
        lines,
    );
}

#[test]
fn declaration() {
    assert_document(
        "package p;\nimport java.util.*;\n@Anno public class Box<T> {\n  /* 😀\n   * doc */\n  int[] data = {\n0x10, 2\n};\n  void run() {\n    var f = (x) -> x + 1;\n    this.use(f, null, true, 'a', .5f);\n  }\n}\n",
        false,
        "
        0:Program:0..195 1:PackageDeclaration:0..10 2:package:0..7 2:Identifier:8..9 2:;:9..10
        1:ImportDeclaration:11..30 2:import:11..17 2:ScopedIdentifier:18..27 3:Identifier:18..22
        3:.:22..23 3:Identifier:23..27 2:.:27..28 2:Asterisk:28..29 2:;:29..30
        1:ClassDeclaration:31..194 2:Modifiers:31..43 3:MarkerAnnotation:31..36 4:Identifier:32..36
        3:public:37..43 2:class:44..49 2:Definition:50..53 2:TypeParameters:53..56
        3:TypeParameter:54..55 4:Definition:54..55 2:ClassBody:57..194 3:{:57..58 3:BlockComment:61..80
        3:FieldDeclaration:83..108 4:ArrayType:83..88 5:PrimitiveType:83..86 5:Dimension:86..88
        6:[:86..87 6:]:87..88 4:VariableDeclarator:89..107 5:Definition:89..93 5:AssignOp:94..95
        5:ArrayInitializer:96..107 6:{:96..97 6:IntegerLiteral:98..102 6:,:102..103
        6:IntegerLiteral:104..105 6:}:106..107 4:;:107..108 3:MethodDeclaration:111..192 4:void:111..115
        4:Definition:116..119 4:FormalParameters:119..121 5:(:119..120 5:):120..121 4:Block:122..192
        5:{:122..123 5:LocalVariableDeclaration:128..149 6:var:128..131 6:VariableDeclarator:132..148
        7:Definition:132..133 7:AssignOp:134..135 7:LambdaExpression:136..148
        8:InferredParameters:136..139 9:(:136..137 9:Definition:137..138 9:):138..139
        8:BinaryExpression:143..148 9:Identifier:143..144 9:ArithOp:145..146 9:IntegerLiteral:147..148
        6:;:148..149 5:ExpressionStatement:154..188 6:MethodInvocation:154..187 7:this:154..158
        7:.:158..159 7:MethodName:159..162 8:Identifier:159..162 7:ArgumentList:162..187 8:(:162..163
        8:Identifier:163..164 8:,:164..165 8:null:166..170 8:,:170..171 8:BooleanLiteral:172..176
        8:,:176..177 8:CharacterLiteral:178..181 8:,:181..182 8:FloatingPointLiteral:183..186
        8:):186..187 6:;:187..188 5:}:191..192 3:}:193..194
        ",
        "
        0..7:moduleKeyword 8..9:variableName 9..10:separator 11..17:moduleKeyword 18..22:variableName
        22..23:derefOperator 23..27:variableName 27..28:derefOperator 28..29:punctuation
        29..30:separator 32..36:variableName 37..43:modifier 44..49:definitionKeyword
        50..53:variableName.definition 54..55:variableName.definition 57..58:brace 61..80:blockComment
        83..86:typeName.standard 86..87:squareBracket 87..88:squareBracket
        89..93:variableName.definition 94..95:definitionOperator 96..97:brace 98..102:integer
        102..103:separator 104..105:integer 106..107:brace 107..108:separator 111..115:keyword
        116..119:variableName.definition 119..120:paren 120..121:paren 122..123:brace
        128..131:definitionKeyword 132..133:variableName.definition 134..135:definitionOperator
        136..137:paren 137..138:variableName.definition 138..139:paren 143..144:variableName
        145..146:arithmeticOperator 147..148:integer 148..149:separator 154..158:self
        158..159:derefOperator 159..162:variableName.function 162..163:paren 163..164:variableName
        164..165:separator 166..170:null 170..171:separator 172..176:bool 176..177:separator
        178..181:character 181..182:separator 183..186:float 186..187:paren 187..188:separator
        191..192:brace 193..194:brace
        ",
        "
        0..10:0:- 11..30:0:- 31..58:0:58..193 59..68:2:63..78 69..80:-:- 81..97:2:97..106 98..105:4:-
        106..108:2:- 109..123:2:123..191 124..149:4:- 150..188:4:- 189..192:2:- 193..194:0:-
        195..195:0:-
        ",
    );
}

#[test]
fn control() {
    assert_document(
        "\tclass Demo {\n\tvoid f(int x) {\n\tlabel:\n\tif (x > 0)\n work(x,\n2);\n\telse\n switch (x) {\ncase 1:\n  x++;\n  break label;\ndefault:\n  throw new Error();\n}\n\ttry\n{\nwork();\n} catch (Exception e) {\nagain();\n}\n}\n}\n",
        false,
        "
        0:Program:0..200 1:ClassDeclaration:1..199 2:class:1..6 2:Definition:7..11 2:ClassBody:12..199
        3:{:12..13 3:MethodDeclaration:15..197 4:void:15..19 4:Definition:20..21
        4:FormalParameters:21..28 5:(:21..22 5:FormalParameter:22..27 6:PrimitiveType:22..25
        6:Definition:26..27 5:):27..28 4:Block:29..197 5:{:29..30 5:LabeledStatement:32..145
        6:Label:32..37 6:::37..38 6:IfStatement:40..145 7:if:40..42 7:ParenthesizedExpression:43..50
        8:(:43..44 8:BinaryExpression:44..49 9:Identifier:44..45 9:CompareOp:46..47
        9:IntegerLiteral:48..49 8:):49..50 7:ExpressionStatement:52..63 8:MethodInvocation:52..62
        9:MethodName:52..56 10:Identifier:52..56 9:ArgumentList:56..62 10:(:56..57 10:Identifier:57..58
        10:,:58..59 10:IntegerLiteral:60..61 10:):61..62 8:;:62..63 7:else:65..69
        7:SwitchStatement:71..145 8:switch:71..77 8:ParenthesizedExpression:78..81 9:(:78..79
        9:Identifier:79..80 9:):80..81 8:SwitchBlock:82..145 9:{:82..83 9:SwitchLabel:84..90
        10:case:84..88 10:IntegerLiteral:89..90 9:::90..91 9:ExpressionStatement:94..98
        10:UpdateExpression:94..97 11:Identifier:94..95 11:UpdateOp:95..97 10:;:97..98
        9:BreakStatement:101..113 10:break:101..106 10:Label:107..112 10:;:112..113
        9:SwitchLabel:114..121 10:default:114..121 9:::121..122 9:ThrowStatement:125..143
        10:throw:125..130 10:ObjectCreationExpression:131..142 11:new:131..134 11:TypeName:135..140
        11:ArgumentList:140..142 12:(:140..141 12:):141..142 10:;:142..143 9:}:144..145
        5:TryStatement:147..195 6:try:147..150 6:Block:151..162 7:{:151..152
        7:ExpressionStatement:153..160 8:MethodInvocation:153..159 9:MethodName:153..157
        10:Identifier:153..157 9:ArgumentList:157..159 10:(:157..158 10:):158..159 8:;:159..160
        7:}:161..162 6:CatchClause:163..195 7:catch:163..168 7:CatchFormalParameter:169..182
        8:(:169..170 8:CatchType:170..179 9:TypeName:170..179 8:Definition:180..181 8:):181..182
        7:Block:183..195 8:{:183..184 8:ExpressionStatement:185..193 9:MethodInvocation:185..192
        10:MethodName:185..190 11:Identifier:185..190 10:ArgumentList:190..192 11:(:190..191
        11:):191..192 9:;:192..193 8:}:194..195 5:}:196..197 3:}:198..199
        ",
        "
        1..6:definitionKeyword 7..11:variableName.definition 12..13:brace 15..19:keyword
        20..21:variableName.definition 21..22:paren 22..25:typeName.standard
        26..27:variableName.definition 27..28:paren 29..30:brace 32..37:labelName 40..42:controlKeyword
        43..44:paren 44..45:variableName 46..47:compareOperator 48..49:integer 49..50:paren
        52..56:variableName.function 56..57:paren 57..58:variableName 58..59:separator 60..61:integer
        61..62:paren 62..63:separator 65..69:controlKeyword 71..77:controlKeyword 78..79:paren
        79..80:variableName 80..81:paren 82..83:brace 84..88:controlKeyword 89..90:integer
        94..95:variableName 95..97:updateOperator 97..98:separator 101..106:controlKeyword
        107..112:labelName 112..113:separator 114..121:controlKeyword 125..130:controlKeyword
        131..134:keyword 135..140:typeName 140..141:paren 141..142:paren 142..143:separator
        144..145:brace 147..150:controlKeyword 151..152:brace 153..157:variableName.function
        157..158:paren 158..159:paren 159..160:separator 161..162:brace 163..168:controlKeyword
        169..170:paren 170..179:typeName 180..181:variableName.definition 181..182:paren 183..184:brace
        185..190:variableName.function 190..191:paren 191..192:paren 192..193:separator 194..195:brace
        196..197:brace 198..199:brace
        ",
        "
        0..13:0:13..198 14..30:6:30..196 31..38:6:- 39..50:4:- 51..59:6:- 60..63:6:- 64..69:4:-
        70..83:6:83..144 84..91:3:- 92..98:5:- 99..113:5:- 114..122:3:- 123..143:5:- 144..145:1:-
        146..150:6:- 151..152:6:152..161 153..160:2:- 161..184:0:184..194 185..193:2:- 194..195:0:-
        196..197:4:- 198..199:4:- 200..200:0:-
        ",
    );
}

#[test]
fn modern() {
    assert_document(
        "open module demo.core {\nrequires transitive java.base;\nexports demo to friend;\n}\nsealed interface Shape permits Dot {}\nrecord Dot(int x) implements Shape {\n int compare(Object o) {\n  return switch (o) {\n   default -> o instanceof Dot d ? d.x() : 0;\n  };\n }\n}\n",
        false,
        "
        0:Program:0..259 1:ModuleDeclaration:0..80 2:open:0..4 2:module:5..11 2:ScopedIdentifier:12..21
        3:Identifier:12..16 3:.:16..17 3:Identifier:17..21 2:ModuleBody:22..80 3:{:22..23
        3:ModuleDirective:24..54 4:requires:24..32 4:transitive:33..43 4:ScopedIdentifier:44..53
        5:Identifier:44..48 5:.:48..49 5:Identifier:49..53 4:;:53..54 3:ModuleDirective:55..78
        4:exports:55..62 4:Identifier:63..67 4:to:68..70 4:Identifier:71..77 4:;:77..78 3:}:79..80
        1:InterfaceDeclaration:81..118 2:Modifiers:81..87 3:sealed:81..87 2:interface:88..97
        2:Definition:98..103 2:Permits:104..115 3:permits:104..111 3:InterfaceTypeList:112..115
        4:TypeName:112..115 2:InterfaceBody:116..118 3:{:116..117 3:}:117..118
        1:RecordDeclaration:119..258 2:record:119..125 2:Definition:126..129 2:RecordHeader:129..136
        3:(:129..130 3:RecordComponent:130..135 4:PrimitiveType:130..133 4:Definition:134..135
        3:):135..136 2:SuperInterfaces:137..153 3:implements:137..147 3:InterfaceTypeList:148..153
        4:TypeName:148..153 2:RecordBody:154..258 3:{:154..155 3:MethodDeclaration:157..256
        4:PrimitiveType:157..160 4:Definition:161..168 4:FormalParameters:168..178 5:(:168..169
        5:FormalParameter:169..177 6:TypeName:169..175 6:Definition:176..177 5:):177..178
        4:Block:179..256 5:{:179..180 5:ReturnStatement:183..253 6:return:183..189
        6:SwitchExpression:190..252 7:switch:190..196 7:ParenthesizedExpression:197..200 8:(:197..198
        8:Identifier:198..199 8:):199..200 7:SwitchBlock:201..252 8:{:201..202 8:SwitchRule:206..248
        9:SwitchLabel:206..213 10:default:206..213 9:ExpressionStatement:217..248
        10:TernaryExpression:217..247 11:InstanceofExpression:217..235 12:Identifier:217..218
        12:instanceof:219..229 12:TypePattern:230..235 13:TypeName:230..233 13:Definition:234..235
        11:LogicOp:236..237 11:MethodInvocation:238..243 12:Identifier:238..239 12:.:239..240
        12:MethodName:240..241 13:Identifier:240..241 12:ArgumentList:241..243 13:(:241..242
        13:):242..243 11:::244..245 11:IntegerLiteral:246..247 10:;:247..248 8:}:251..252 6:;:252..253
        5:}:255..256 3:}:257..258
        ",
        "
        0..4:keyword 5..11:moduleKeyword 12..16:variableName 16..17:derefOperator 17..21:variableName
        22..23:brace 24..32:modifier 33..43:modifier 44..48:variableName 48..49:derefOperator
        49..53:variableName 53..54:separator 55..62:modifier 63..67:variableName 68..70:keyword
        71..77:variableName 77..78:separator 79..80:brace 88..97:definitionKeyword
        98..103:variableName.definition 112..115:typeName 116..117:brace 117..118:brace
        126..129:variableName.definition 129..130:paren 130..133:typeName.standard
        134..135:variableName.definition 135..136:paren 137..147:definitionKeyword 148..153:typeName
        154..155:brace 157..160:typeName.standard 161..168:variableName.definition 168..169:paren
        169..175:typeName 176..177:variableName.definition 177..178:paren 179..180:brace
        183..189:controlKeyword 190..196:controlKeyword 197..198:paren 198..199:variableName
        199..200:paren 201..202:brace 206..213:controlKeyword 217..218:variableName
        219..229:operatorKeyword 230..233:typeName 234..235:variableName.definition
        236..237:logicOperator 238..239:variableName 239..240:derefOperator
        240..241:variableName.function 241..242:paren 242..243:paren 246..247:integer 247..248:separator
        251..252:brace 252..253:separator 255..256:brace 257..258:brace
        ",
        "
        0..23:0:23..79 24..54:2:- 55..78:2:- 79..80:0:- 81..118:0:- 119..155:0:- 156..180:2:180..255
        181..202:3:202..251 203..248:4:- 249..253:2:- 254..256:1:- 257..258:0:- 259..259:0:-
        ",
    );
}

#[test]
fn text_block_crlf() {
    assert_document(
        "// 😀\r\nclass Café {\r\n  String s = \"\"\"\r\n    😀 text\r\n    \"\"\";\r\n}\r\n",
        false,
        "
        0:Program:0..70 1:LineComment:0..8 1:ClassDeclaration:9..68 2:class:9..14 2:Definition:15..20
        2:ClassBody:21..68 3:{:21..22 3:FieldDeclaration:26..65 4:TypeName:26..32
        4:VariableDeclarator:33..64 5:Definition:33..34 5:AssignOp:35..36 5:TextBlock:37..64 4:;:64..65
        3:}:67..68
        ",
        "
        0..8:lineComment 9..14:definitionKeyword 15..20:variableName.definition 21..22:brace
        26..32:typeName 33..34:variableName.definition 35..36:definitionOperator 37..64:string
        64..65:separator 67..68:brace
        ",
        "
        0..7:0:- 9..22:0:22..67 24..40:2:- 42..55:2:- 57..65:2:- 67..68:0:- 70..70:0:-
        ",
    );
}

#[test]
fn folds() {
    assert_document(
        "@Meta({\n1, 2\n})\ninterface I {\n int[] nums = {\n0, 1\n};\n}\nenum E {\n A, B;\n E() {\nwork();\n }\n}\n",
        false,
        "
        0:Program:0..92 1:InterfaceDeclaration:0..55 2:Modifiers:0..15 3:Annotation:0..15
        4:Identifier:1..5 4:AnnotationArgumentList:5..15 5:(:5..6 5:ElementValueArrayInitializer:6..14
        6:{:6..7 6:IntegerLiteral:8..9 6:,:9..10 6:IntegerLiteral:11..12 6:}:13..14 5:):14..15
        2:interface:16..25 2:Definition:26..27 2:InterfaceBody:28..55 3:{:28..29
        3:ConstantDeclaration:31..53 4:ArrayType:31..36 5:PrimitiveType:31..34 5:Dimension:34..36
        6:[:34..35 6:]:35..36 4:VariableDeclarator:37..52 5:Definition:37..41 5:AssignOp:42..43
        5:ArrayInitializer:44..52 6:{:44..45 6:IntegerLiteral:46..47 6:,:47..48 6:IntegerLiteral:49..50
        6:}:51..52 4:;:52..53 3:}:54..55 1:EnumDeclaration:56..91 2:enum:56..60 2:Definition:61..62
        2:EnumBody:63..91 3:{:63..64 3:EnumConstant:66..67 4:Definition:66..67 3:,:67..68
        3:EnumConstant:69..70 4:Definition:69..70 3:EnumBodyDeclarations:70..89 4:;:70..71
        4:ConstructorDeclaration:73..89 5:Definition:73..74 5:FormalParameters:74..76 6:(:74..75
        6:):75..76 5:ConstructorBody:77..89 6:{:77..78 6:ExpressionStatement:79..86
        7:MethodInvocation:79..85 8:MethodName:79..83 9:Identifier:79..83 8:ArgumentList:83..85
        9:(:83..84 9:):84..85 7:;:85..86 6:}:88..89 3:}:90..91
        ",
        "
        1..5:variableName 5..6:paren 6..7:brace 8..9:integer 9..10:separator 11..12:integer 13..14:brace
        14..15:paren 16..25:definitionKeyword 26..27:variableName.definition 28..29:brace
        31..34:typeName.standard 34..35:squareBracket 35..36:squareBracket
        37..41:variableName.definition 42..43:definitionOperator 44..45:brace 46..47:integer
        47..48:separator 49..50:integer 51..52:brace 52..53:separator 54..55:brace
        56..60:definitionKeyword 61..62:variableName.definition 63..64:brace
        66..67:variableName.definition 67..68:separator 69..70:variableName.definition 70..71:separator
        73..74:variableName.definition 74..75:paren 75..76:paren 77..78:brace
        79..83:variableName.function 83..84:paren 84..85:paren 85..86:separator 88..89:brace
        90..91:brace
        ",
        "
        0..7:0:7..13 8..12:2:- 13..15:0:- 16..29:2:29..54 30..45:2:45..51 46..50:3:- 51..53:1:-
        54..55:0:- 56..64:0:64..90 65..71:2:- 72..78:2:78..88 79..86:3:- 87..89:1:- 90..91:0:-
        92..92:0:-
        ",
    );
}

#[test]
fn eof() {
    assert_document(
        "class Open {\n  void f() {\n    work(1,\n         2",
        true,
        "
        0:Program:0..48 1:ClassDeclaration:0..48 2:class:0..5 2:Definition:6..10 2:ClassBody:11..48
        3:{:11..12 3:MethodDeclaration:15..48 4:void:15..19 4:Definition:20..21
        4:FormalParameters:21..23 5:(:21..22 5:):22..23 4:Block:24..48 5:{:24..25
        5:ExpressionStatement:30..48 6:MethodInvocation:30..48 7:MethodName:30..34 8:Identifier:30..34
        7:ArgumentList:34..48 8:(:34..35 8:IntegerLiteral:35..36 8:,:36..37 8:IntegerLiteral:47..48
        8:⚠:48..48 6:⚠:48..48 5:⚠:48..48 3:⚠:48..48
        ",
        "
        0..5:definitionKeyword 6..10:variableName.definition 11..12:brace 15..19:keyword
        20..21:variableName.definition 21..22:paren 22..23:paren 24..25:brace
        30..34:variableName.function 34..35:paren 35..36:integer 36..37:separator 47..48:integer
        ",
        "
        0..12:0:12..48 13..25:2:25..48 26..37:4:- 38..48:9:-
        ",
    );
}

#[test]
fn eof_after_comment() {
    assert_document(
        "class Open {\n  // 😀",
        true,
        "
        0:Program:0..22 1:ClassDeclaration:0..22 2:class:0..5 2:Definition:6..10 2:ClassBody:11..22
        3:{:11..12 3:LineComment:15..22 3:⚠:22..22
        ",
        "
        0..5:definitionKeyword 6..10:variableName.definition 11..12:brace 15..22:lineComment
        ",
        "
        0..12:0:12..22 13..22:2:-
        ",
    );
}

#[test]
fn invalid() {
    assert_document(
        "class Good {}\n#",
        true,
        "
        0:Program:0..15 1:ClassDeclaration:0..13 2:class:0..5 2:Definition:6..10 2:ClassBody:11..13
        3:{:11..12 3:}:12..13 1:⚠:14..15
        ",
        "
        0..5:definitionKeyword 6..10:variableName.definition 11..12:brace 12..13:brace
        ",
        "
        0..13:0:- 14..15:0:-
        ",
    );
}

#[test]
fn empty() {
    assert_document(
        "  \n\n",
        false,
        "
        0:Program:0..4
        ",
        "",
        "
        0..2:0:- 3..3:0:- 4..4:0:-
        ",
    );
}

#[test]
fn class_content() {
    let source = "public int field;\nprivate Example() {}\nstatic void call() {}\n";
    assert!(std::ptr::eq(parser(), parser()));
    let tree = class_content_parser().parse_glr(source).unwrap();
    fn walk(node: SyntaxNode<'_>, depth: usize, rows: &mut Vec<String>) {
        rows.push(row(node.name(), node.range(), depth));
        for child in node.children() {
            walk(child, depth + 1, rows);
        }
    }
    let mut rows = Vec::new();
    walk(tree.root(), 0, &mut rows);
    compare(rows.into_iter(),
        "
        0:ClassContent:0..61 1:FieldDeclaration:0..17 2:Modifiers:0..6 3:public:0..6
        2:PrimitiveType:7..10 2:VariableDeclarator:11..16 3:Definition:11..16 2:;:16..17
        1:ConstructorDeclaration:18..38 2:Modifiers:18..25 3:private:18..25 2:Definition:26..33
        2:FormalParameters:33..35 3:(:33..34 3:):34..35 2:ConstructorBody:36..38 3:{:36..37 3:}:37..38
        1:MethodDeclaration:39..60 2:Modifiers:39..45 3:static:39..45 2:void:46..50 2:Definition:51..55
        2:FormalParameters:55..57 3:(:55..56 3:):56..57 2:Block:58..60 3:{:58..59 3:}:59..60
        ",
    );
}

#[test]
fn metadata_and_indentation_options() {
    assert_eq!(java::NAME, "java");
    assert_eq!(java::LINE_COMMENT, "//");
    assert_eq!(java::BLOCK_COMMENT, ("/*", "*/"));
    for prefix in ["case ", " \tdefault:", "\u{feff}}", "  {", "}"] {
        assert!(java::indent_on_input(prefix), "{prefix:?}");
    }
    for prefix in ["case", "case x:", "default: ", "}\n", "\u{85}}", ""] {
        assert!(!java::indent_on_input(prefix), "{prefix:?}");
    }

    let source = "class T {\n\tvoid f() {\n\t\tcall( // ignored\n      123,\n      other);\n\t\tcall( first, second);\n\t}\n}";
    let doc = JavaDocument::parse(Text::new(source));
    let settings = StreamSettings {
        tab_size: NonZeroUsize::new(8).unwrap(),
        indent_unit: 4,
    };
    assert_eq!(
        doc.indentation(58, settings, IndentOptions::default()),
        Some(20)
    );
    assert_eq!(
        doc.indentation(
            58,
            settings,
            IndentOptions {
                override_indentation: Some(
                    &|from| (Some(from) == source.find("\t\tcall")).then_some(7)
                ),
                ..IndentOptions::default()
            }
        ),
        Some(11)
    );
    for (point, double, expected) in [(81, false, 22), (87, false, 20), (87, true, 22)] {
        assert_eq!(
            doc.indentation(
                point,
                settings,
                IndentOptions {
                    simulate_break: Some(point),
                    simulate_double_break: double,
                    ..IndentOptions::default()
                }
            ),
            Some(expected)
        );
    }
}

#[test]
fn catalog_java_support_matches_standalone_language_and_installs_only_java_data() {
    let entry = LanguageDescription::match_filename(LANGUAGES, "Example.java").unwrap();
    let support = *entry.native_support_load().load().unwrap();
    assert_eq!(support, NativeLanguage::Java.support());
    assert_eq!(support.completion_source_capacity(), 0);
    let mut state = EditorState::with_extensions(
        "class A {\r\n  // 😀\r  void f() {\n    run();\r\n  }\r}\n",
        support.tracked_language_data_extension(),
    )
    .unwrap();
    let mut installed = support.parse(state.text().clone()).unwrap();
    let check =
        |state: &EditorState,
         installed: &gpui_editor_codemirror::language_data::SupportedDocument| {
            let standalone = NativeLanguage::Java.parse(state.text().clone()).unwrap();
            let (NativeSyntax::Java(direct), NativeSyntax::Java(with_support)) =
                (&standalone, installed.syntax())
            else {
                panic!("Java parser in both language forms");
            };
            assert_eq!(with_support.root().name(), "Program");
            assert!(
                with_support
                    .nodes()
                    .map(|n| (n.term(), n.range()))
                    .eq(direct.nodes().map(|n| (n.term(), n.range())))
            );
            assert_eq!(
                installed.highlights(0..state.text().len()).unwrap(),
                standalone
                    .highlights(0..state.text().len())
                    .unwrap()
                    .collect::<Vec<_>>()
            );
            for at in [
                0,
                state.text().as_str().find("void").unwrap(),
                state.text().len(),
            ] {
                for side in [Assoc::Before, Assoc::After] {
                    assert_eq!(installed.language_at(at, side), Some(NativeLanguage::Java));
                    let comments =
                        state.language_data_at::<CommentTokens>("commentTokens", at, side);
                    let [comments] = comments.as_slice() else {
                        panic!("Java comment markers");
                    };
                    assert_eq!(
                        (comments.line, comments.block),
                        (Some("//"), Some(("/*", "*/")))
                    );
                    let tests =
                        state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
                    let [test] = tests.as_slice() else {
                        panic!("Java indentation trigger");
                    };
                    assert!(test(" \tcase "));
                    assert!(!test("default: "));
                    assert!(
                        state
                            .language_data_at::<CloseBracketData>("closeBrackets", at, side)
                            .is_empty()
                    );
                    assert!(
                        state
                            .language_data_at::<&str>("wordChars", at, side)
                            .is_empty()
                    );
                    assert!(installed.completions(at, true).is_empty());
                }
            }
        };
    check(&state, &installed);
    let edit = ChangeSet::single(Change::new(0..0, "/* note */\n")).unwrap();
    state.update(Transaction::new(edit.clone())).unwrap();
    installed.apply_changes(&edit).unwrap();
    check(&state, &installed);
}

#[test]
fn byte_boundaries_and_recovery_limits() {
    let source = "/* 😀 */ class A {}";
    let doc = JavaDocument::parse(Text::new(source));
    let mut children = doc.root().children();
    assert_eq!(children.len(), 2);
    assert_eq!(children.next().unwrap().fold_range(), Some(2..8));
    assert_eq!(children.next_back().unwrap().name(), "ClassDeclaration");
    assert_eq!(doc.highlights(3..7).unwrap().next().unwrap().range, 3..7);
    assert_eq!(doc.highlights(3..3).unwrap().count(), 0);
    assert!(doc.highlights(4..7).is_none());
    assert!(doc.foldable(4, 7).is_none());
    assert!(
        doc.indentation(4, StreamSettings::default(), IndentOptions::default())
            .is_none()
    );
    assert!(
        doc.indentation(
            source.len() + 1,
            StreamSettings::default(),
            IndentOptions::default()
        )
        .is_none()
    );

    // Size, including skipped whitespace, limits the extra recovery work.
    let mut source = String::from("class A { void f() { x += ; } }");
    source.extend(std::iter::repeat_n(' ', 16 * 1024 - source.len()));
    let doc = JavaDocument::parse(Text::new(&source));
    let diagnostic = doc.parse_error();
    assert!(matches!(
        diagnostic,
        Some(ParseError::UnexpectedToken { offset: 26, .. })
    ));
    let marker = doc.nodes().find(|node| node.term() == 0).unwrap();
    assert_eq!(marker.range(), 26..26);
    assert_eq!(marker.parent().unwrap().name(), "AssignmentExpression");
    source.push(' ');
    let capped = JavaDocument::parse(Text::new(&source));
    assert_eq!(capped.parse_error(), diagnostic);
    let error = capped.root().children().next().unwrap();
    assert_eq!((error.name(), error.range()), ("⚠", 0..source.len()));
    assert_eq!(capped.highlights(0..source.len()).unwrap().count(), 0);
    assert_eq!(capped.reuse_stats(), ReuseStats::default());
}

#[test]
fn recovered_program_can_seed_only_strict_prefix_reuse() {
    let prefix = "// 🦀 raw CRLF\r\nclass Seed { int café = 42; }\r\n".repeat(8);
    let source = format!("{prefix}class Tail {{\r\n  int data = 7;\r\n}}\r\n");
    let at = source.rfind('7').unwrap();
    let valid = JavaDocument::parse(Text::new(&source));
    let remove = ChangeSet::single(Change::new(at..at + 1, "")).unwrap();
    let insert = ChangeSet::single(Change::new(at..at, "7")).unwrap();
    let broken = valid.reparse(&remove).unwrap();
    let fresh = JavaDocument::parse(broken.text().clone());
    assert_eq!(
        broken.parse_error(),
        Some(parser().parse(broken.text().as_str()).unwrap_err())
    );
    assert_eq!(broken.parse_error(), fresh.parse_error());
    for seed in [broken, fresh] {
        assert_eq!(seed.reuse_stats(), ReuseStats::default());
        assert!(seed.nodes().any(|n| n.term() == 0));
        let repaired = seed.reparse(&insert).unwrap();
        let stats = repaired.reuse_stats();
        assert!((1..at).contains(&stats.resumed_at), "{stats:?}");
        assert!(source.is_char_boundary(stats.resumed_at));
        assert!(stats.nodes_reused > 0);
        assert_eq!(repaired.text().as_str(), source);
        assert_eq!(repaired.parse_error(), None);
        assert!(
            repaired
                .nodes()
                .map(|n| (n.term(), n.range()))
                .eq(valid.nodes().map(|n| (n.term(), n.range())))
        );
        assert!(
            repaired
                .highlights(0..source.len())
                .unwrap()
                .eq(valid.highlights(0..source.len()).unwrap())
        );
    }
}

#[test]
fn class_content_repair_reuses_only_same_root_checkpoints() {
    let prefix = "// 🦀\r\nint café;\r\n".repeat(12);
    let malformed = format!("{prefix}int value = ;\r\n");
    let fixed = format!("{prefix}int value = 1;\r\n");
    let at = malformed.rfind(';').unwrap();
    let class = class_content_parser();
    assert!(class.parse(&malformed).is_err());
    let recovered = class.parse_incremental_recovering(&malformed, &[]).unwrap();
    assert_eq!(recovered.reuse_stats(), ReuseStats::default());
    assert_eq!(recovered.reused_suffix_actions(), 0);
    let fragment = TreeFragment {
        tree: &recovered,
        old_range: 0..at,
        new_range: 0..at,
    };
    let repaired = class
        .parse_incremental(&fixed, Some(fragment.clone()))
        .unwrap();
    assert!(repaired.reuse_stats().nodes_reused > 0);
    assert_eq!(repaired.root().range(), 0..fixed.len());
    assert_eq!(repaired.root().name(), "ClassContent");

    let program = parser().parse_incremental(&fixed, Some(fragment)).unwrap();
    assert_eq!(program.root().name(), "Program");
    assert_eq!(program.reuse_stats(), ReuseStats::default());
}
