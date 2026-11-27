//! JS oracle: `@codemirror/lang-go` 6.0.1 and `@lezer/go` 1.0.1 (SRI-verified
//! npm tarballs), with `@codemirror/state` 6.7.6, `@codemirror/language` 6.13.1,
//! `@codemirror/autocomplete` 6.20.4, `@lezer/lr` 1.4.13, `@lezer/common` 1.5.7,
//! and `@lezer/highlight` 1.2.5. The fixtures use `syntaxTree`, `highlightTree`,
//! `getIndentation`, and `foldable`. The oracle converted UTF-16 offsets to
//! original UTF-8 bytes. Rust tests do not require JS.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        go::{
            BLOCK_COMMENT, CLOSE_BRACKETS, GoDocument, GoNode, GoNodeKind, LINE_COMMENT, NAME,
            indent_on_input,
        },
    },
};
use serde_json::Value;

fn settings() -> StreamSettings {
    StreamSettings {
        indent_unit: 4,
        ..StreamSettings::default()
    }
}

fn tree(node: GoNode<'_>) -> String {
    let name = node.kind().name();
    let name = if node.kind() == GoNodeKind::Error || name.as_bytes()[0].is_ascii_alphabetic() {
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

fn parse(source: &str) -> GoDocument {
    GoDocument::parse(Text::new(source))
}

fn check_oracle(fixture: &str) {
    let expected: Value = serde_json::from_str(fixture).unwrap();
    let source = expected["doc"].as_str().unwrap();
    let doc = parse(source);
    assert_eq!(tree(doc.root()), expected["tree"].as_str().unwrap());
    assert_eq!(
        doc.is_recovered(),
        expected["tree"].as_str().unwrap().contains('⚠')
    );
    let ranges = doc
        .nodes()
        .map(|node| {
            let range = node.range();
            if let Some(parent) = node.parent() {
                assert!(parent.range().start <= range.start && range.end <= parent.range().end);
            }
            assert!(source.get(range.clone()).is_some());
            format!("{}:{}", range.start, range.end)
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(ranges, expected["nodes"].as_str().unwrap());
    check_editor_queries(&doc, &expected);
}

fn check_editor_queries(doc: &GoDocument, expected: &Value) {
    let highlights = doc
        .highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let styles = span
                .tags
                .iter()
                .map(|tag| {
                    let mut style = tag.name().to_owned();
                    for modifier in tag.modifiers() {
                        style.push_str(match modifier {
                            TagModifier::Definition => ".definition",
                            TagModifier::Function => ".function",
                            _ => panic!("unexpected tag modifier: {modifier:?}"),
                        });
                    }
                    style
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("{}:{}:{styles}", span.range.start, span.range.end)
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(highlights, expected["highlights"].as_str().unwrap());
    assert_eq!(
        doc.text().line_count(),
        expected["indent"].as_array().unwrap().len()
    );
    for i in 1..=doc.text().line_count() {
        let line = doc.text().line(i).unwrap();
        assert_eq!(
            doc.indentation(line.range.start, settings(), IndentOptions::default()),
            expected["indent"][i - 1].as_u64().map(|n| n as usize),
            "indentation on line {i}: {:?}",
            line.text
        );
        assert_eq!(
            doc.foldable(line.range.start, line.range.end)
                .map(|fold| format!("{}:{}", fold.start, fold.end))
                .as_deref(),
            expected["fold"][i - 1].as_str(),
            "fold on line {i}"
        );
    }
}

#[test]
fn exposes_upstream_language_data() {
    assert_eq!(NAME, "go");
    assert_eq!(LINE_COMMENT, "//");
    assert_eq!(BLOCK_COMMENT, ("/*", "*/"));
    assert_eq!(CLOSE_BRACKETS, ['(', '[', '{', '\'', '"', '`']);
    for line in ["case", "\tdefault", "}", "\u{feff}}"] {
        assert!(indent_on_input(line));
    }
    for line in [
        "case ",
        "case:",
        "default_label",
        "case x:",
        "{}",
        "}\n",
        "\u{85}}",
        "// }",
    ] {
        assert!(!indent_on_input(line));
    }
}

#[test]
fn maps_crlf_and_clips_highlights_in_original_bytes() {
    let lf = "package π\n/* 😀\n comment */\nfunc f() {\n return\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    let normalized = parse(lf);
    let original = parse(&crlf);
    assert!(!original.is_recovered());
    assert_eq!(tree(original.root()), tree(normalized.root()));
    let remap = |at: usize| at + lf.as_bytes()[..at].iter().filter(|&&b| b == b'\n').count();
    for (before, after) in normalized.nodes().zip(original.nodes()) {
        assert_eq!(
            remap(before.range().start)..remap(before.range().end),
            after.range()
        );
    }
    let at = crlf.find('😀').unwrap();
    let spans = original.highlights(at..at + 4).unwrap().collect::<Vec<_>>();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, at..at + 4);
    assert_eq!(spans[0].tags[0].name(), "blockComment");
    assert!(original.highlights(at + 1..at + 4).is_none());
    assert!(original.highlights(at..at).unwrap().next().is_none());
    assert!(
        original
            .indentation(at + 1, settings(), IndentOptions::default())
            .is_none()
    );
    let line = original.text().line(2).unwrap();
    let fold = original.foldable(line.range.start, line.range.end).unwrap();
    assert_eq!(&crlf[fold], " 😀\r\n comment ");
}

#[test]
fn matches_oracle_simulated_break_and_indentation_override() {
    let source = "func f() {\n\tmycall(1,\n\t\tword,\n\t)\n}\n";
    let doc = parse(source);
    let at = source.find("1,").unwrap();
    let line = doc.text().line_at(at).unwrap().range.start;
    let override_indent = |start| (start == line).then_some(10);
    let indent = |options| doc.indentation(at, settings(), options);
    assert_eq!(indent(IndentOptions::default()), Some(11));
    assert_eq!(
        indent(IndentOptions {
            override_indentation: Some(&override_indent),
            ..IndentOptions::default()
        }),
        Some(17)
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

    let doc = parse("func f() {}");
    let indent = |options| doc.indentation(10, settings(), options);
    assert_eq!(indent(IndentOptions::default()), Some(9));
    assert_eq!(
        indent(IndentOptions {
            simulate_break: Some(10),
            ..IndentOptions::default()
        }),
        Some(0)
    );
    assert_eq!(
        indent(IndentOptions {
            simulate_break: Some(10),
            simulate_double_break: true,
            ..IndentOptions::default()
        }),
        Some(4)
    );
}

#[test]
fn matches_oracle_declarations() {
    check_oracle(
        r###"{"name":"declarations","doc":"package main\nimport (\n\t. \"fmt\"\n\tlib \"example.org/lib\"\n)\ntype Box[T ~int | ~float64] struct {\n\tValue T `json:\"value\"`\n}\ntype Closer interface {\n\tClose() error\n}\nfunc (b *Box[T]) Get() T { return b.Value }\nfunc f(v int) {\n\tprintln(1,\n\t\tv,\n\t)\n}\n","tree":"SourceFile(PackageClause(package,DefName),ImportDecl(import,SpecList(\"(\",ImportSpec(\".\",String),ImportSpec(DefName,String),\")\")),TypeDecl(type,TypeSpec(DefName,TypeParams(\"[\",TypeParam(TypeName,TypeElem(UnderlyingType(\"~\",TypeName),LogicOp,UnderlyingType(\"~\",TypeName))),\"]\"),StructType(struct,StructBody(\"{\",FieldDecl(FieldName,TypeName,String),\"}\")))),TypeDecl(type,TypeSpec(DefName,InterfaceType(interface,InterfaceBody(\"{\",MethodElem(FieldName,Parameters(\"(\",\")\"),TypeName),\"}\")))),MethodDecl(func,Parameters(\"(\",Parameter(DefName,PointerType(\"*\",ParameterizedType(TypeName,TypeArgs(\"[\",TypeName,\"]\")))),\")\"),FieldName,Parameters(\"(\",\")\"),TypeName,Block(\"{\",ReturnStatement(return,SelectorExpr(VariableName,\".\",FieldName)),\"}\")),FunctionDecl(func,DefName,Parameters(\"(\",Parameter(DefName,TypeName),\")\"),Block(\"{\",ExprStatement(CallExpr(VariableName,Arguments(\"(\",Number,\",\",VariableName,\",\",\")\"))),\"}\")))","highlights":"0:7:moduleKeyword 8:12:variableName.definition 13:19:moduleKeyword 20:21:paren 23:24:derefOperator 25:30:string 32:35:variableName.definition 36:53:string 54:55:paren 56:60:definitionKeyword 61:64:typeName.definition 64:65:squareBracket 65:66:typeName 67:68:modifier 68:71:typeName 72:73:logicOperator 74:75:modifier 75:82:typeName 82:83:squareBracket 84:90:definitionKeyword 91:92:brace 94:99:propertyName 100:101:typeName 102:116:string 117:118:brace 119:123:definitionKeyword 124:130:typeName.definition 131:140:definitionKeyword 141:142:brace 144:149:propertyName 149:150:paren 150:151:paren 152:157:typeName 158:159:brace 160:164:definitionKeyword 165:166:paren 166:167:variableName.definition 168:169:modifier 169:172:typeName 172:173:squareBracket 173:174:typeName 174:175:squareBracket 175:176:paren 177:180:propertyName 180:181:paren 181:182:paren 183:184:typeName 185:186:brace 187:193:controlKeyword 194:195:variableName 195:196:derefOperator 196:201:propertyName 202:203:brace 204:208:definitionKeyword 209:210:variableName.definition.function 210:211:paren 211:212:variableName.definition 213:216:typeName 216:217:paren 218:219:brace 221:228:variableName.function 228:229:paren 229:230:number 230:231:separator 234:235:variableName 235:236:separator 238:239:paren 240:241:brace","nodes":"0:242 0:12 0:7 8:12 13:55 13:19 20:55 20:21 23:30 23:24 25:30 32:53 32:35 36:53 54:55 56:118 56:60 61:118 61:64 64:83 64:65 65:82 65:66 67:82 67:71 67:68 68:71 72:73 74:82 74:75 75:82 82:83 84:118 84:90 91:118 91:92 94:116 94:99 100:101 102:116 117:118 119:159 119:123 124:159 124:130 131:159 131:140 141:159 141:142 144:157 144:149 149:151 149:150 150:151 152:157 158:159 160:203 160:164 165:176 165:166 166:175 166:167 168:175 168:169 169:175 169:172 172:175 172:173 173:174 174:175 175:176 177:180 180:182 180:181 181:182 183:184 185:203 185:186 187:201 187:193 194:201 194:195 195:196 196:201 202:203 204:241 204:208 209:210 210:217 210:211 211:216 211:212 213:216 216:217 218:241 218:219 221:239 221:239 221:228 228:239 228:229 229:230 230:231 234:235 235:236 238:239 240:241","indent":[0,0,4,4,0,0,4,0,0,4,0,0,0,4,12,11,0,0],"fold":[null,"21:54",null,null,null,null,null,null,null,null,null,null,"219:240",null,null,null,null,null]}"###,
    );
}

#[test]
fn matches_oracle_selection() {
    check_oracle(
        r###"{"name":"selection","doc":"package p\nfunc f(ch chan int, v any) {\n\tif v == nil {\n\t\treturn // note\n\t} else if v != nil {\n\t\tg := new(int)\n\t\t*g++\n\t}\n\tswitch a := v.(type) {\n\tcase int, string:\n\t\tprintln(a)\n\tdefault:\n\t\treturn\n\t}\n\tselect {\n\tcase ch <- 1:\n\t\treturn\n\tcase y := <-ch:\n\t\tprintln(y)\n\t}\n}\n","tree":"SourceFile(PackageClause(package,DefName),FunctionDecl(func,DefName,Parameters(\"(\",Parameter(DefName,ChannelType(chan,TypeName)),\",\",Parameter(DefName,TypeName),\")\"),Block(\"{\",IfStatement(if,BinaryExp(VariableName,CompareOp,Nil),Block(\"{\",ReturnStatement(return),LineComment,\"}\"),else,IfStatement(if,BinaryExp(VariableName,CompareOp,Nil),Block(\"{\",VarDecl(DefName,\":=\",CallExpr(VariableName,Arguments(\"(\",VariableName,\")\"))),IncDecStatement(UnaryExp(DerefOp,VariableName),IncDecOp),\"}\"))),TypeSwitchStatement(switch,DefName,\":=\",VariableName,\".\",\"(\",type,\")\",SwitchBlock(\"{\",Case(case,TypeName,\",\",TypeName,\":\"),ExprStatement(CallExpr(VariableName,Arguments(\"(\",VariableName,\")\"))),Case(default,\":\"),ReturnStatement(return),\"}\")),SelectStatement(select,SelectBlock(\"{\",Case(case,SendStatement(VariableName,\"<-\",Number),\":\"),ReturnStatement(return),Case(case,ReceiveStatement(DefName,\":=\",UnaryExp(\"<-\",VariableName)),\":\"),ExprStatement(CallExpr(VariableName,Arguments(\"(\",VariableName,\")\"))),\"}\")),\"}\")))","highlights":"0:7:moduleKeyword 8:9:variableName.definition 10:14:definitionKeyword 15:16:variableName.definition.function 16:17:paren 17:19:variableName.definition 20:24:definitionKeyword 25:28:typeName 28:29:separator 30:31:variableName.definition 32:35:typeName 35:36:paren 37:38:brace 40:42:controlKeyword 43:44:variableName 45:47:compareOperator 48:51:null 52:53:brace 56:62:controlKeyword 63:70:lineComment 72:73:brace 74:78:controlKeyword 79:81:controlKeyword 82:83:variableName 84:86:compareOperator 87:90:null 91:92:brace 95:96:variableName.definition 97:99:definitionOperator 100:103:variableName.function 103:104:paren 104:107:variableName 107:108:paren 111:112:derefOperator 112:113:variableName 113:115:updateOperator 117:118:brace 120:126:controlKeyword 127:128:variableName.definition 129:131:definitionOperator 132:133:variableName 133:134:derefOperator 134:135:paren 135:139:definitionKeyword 139:140:paren 141:142:brace 144:148:controlKeyword 149:152:typeName 152:153:separator 154:160:typeName 160:161:punctuation 164:171:variableName.function 171:172:paren 172:173:variableName 173:174:paren 183:184:punctuation 187:193:controlKeyword 195:196:brace 198:204:controlKeyword 205:206:brace 208:212:controlKeyword 213:215:variableName 216:218:operator 219:220:number 220:221:punctuation 224:230:controlKeyword 232:236:controlKeyword 237:238:variableName.definition 239:241:definitionOperator 242:244:operator 244:246:variableName 246:247:punctuation 250:257:variableName.function 257:258:paren 258:259:variableName 259:260:paren 262:263:brace 264:265:brace","nodes":"0:266 0:9 0:7 8:9 10:265 10:14 15:16 16:36 16:17 17:28 17:19 20:28 20:24 25:28 28:29 30:35 30:31 32:35 35:36 37:265 37:38 40:118 40:42 43:51 43:44 45:47 48:51 52:73 52:53 56:62 56:62 63:70 72:73 74:78 79:118 79:81 82:90 82:83 84:86 87:90 91:118 91:92 95:108 95:96 97:99 100:108 100:103 103:108 103:104 104:107 107:108 111:115 111:113 111:112 112:113 113:115 117:118 120:196 120:126 127:128 129:131 132:133 133:134 134:135 135:139 139:140 141:196 141:142 144:161 144:148 149:152 152:153 154:160 160:161 164:174 164:174 164:171 171:174 171:172 172:173 173:174 176:184 176:183 183:184 187:193 187:193 195:196 198:263 198:204 205:263 205:206 208:221 208:212 213:220 213:215 216:218 219:220 220:221 224:230 224:230 232:247 232:236 237:246 237:238 239:241 242:246 242:244 244:246 246:247 250:260 250:260 250:257 257:260 257:258 258:259 259:260 262:263 264:265","indent":[0,0,4,8,4,8,8,4,4,4,8,4,8,4,4,4,8,4,8,4,0,0],"fold":[null,"38:264","53:72",null,"92:117",null,null,null,"142:195",null,null,null,null,null,"206:262",null,null,null,null,null,null,null]}"###,
    );
}

#[test]
fn matches_oracle_literals() {
    check_oracle(
        r###"{"name":"literals","doc":"package π\n/* astral 😀\n\t second line\n*/\nvar 𐐀 = []any{\n\t`raw 😀\n string`, \"escape\\n\", '界',\n\t0x_1.fp2, 0o711, 0b100, .5e-1i, true, false, nil,\n}\nfunc f() {\n\tx := make([]int, 1)\n\tx[0] += int(𐐀[0].(int))\n\tA[T](x) // expand\n}\n","tree":"SourceFile(PackageClause(package,DefName),BlockComment,VarDecl(var,VarSpec(DefName,\"=\",TypedLiteral(SliceType(\"[\",\"]\",TypeName),LiteralValue(\"{\",Element(String),\",\",Element(String),\",\",Element(Rune),\",\",Element(Number),\",\",Element(Number),\",\",Element(Number),\",\",Element(Number),\",\",Element(Bool),\",\",Element(Bool),\",\",Element(Nil),\",\",\"}\")))),FunctionDecl(func,DefName,Parameters(\"(\",\")\"),Block(\"{\",VarDecl(DefName,\":=\",CallExpr(make,Arguments(\"(\",SliceType(\"[\",\"]\",TypeName),\",\",Number,\")\"))),Assignment(IndexExpr(VariableName,\"[\",Number,\"]\"),UpdateOp,CallExpr(VariableName,Arguments(\"(\",TypeAssertion(IndexExpr(VariableName,\"[\",Number,\"]\"),\".\",\"(\",TypeName,\")\"),\")\"))),ExprStatement(CallExpr(ParameterizedExpr(VariableName,TypeArgs(\"[\",TypeName,\"]\")),Arguments(\"(\",VariableName,\")\"))),LineComment,\"}\")))","highlights":"0:7:moduleKeyword 8:10:variableName.definition 11:42:blockComment 43:46:definitionKeyword 47:51:variableName.definition 52:53:definitionOperator 54:55:squareBracket 55:56:squareBracket 56:59:typeName 59:60:brace 62:80:string 80:81:separator 82:92:string 92:93:separator 94:99:character 99:100:separator 102:110:number 110:111:separator 112:117:number 117:118:separator 119:124:number 124:125:separator 126:132:number 132:133:separator 134:138:bool 138:139:separator 140:145:bool 145:146:separator 147:150:null 150:151:separator 152:153:brace 154:158:definitionKeyword 159:160:variableName.definition.function 160:161:paren 161:162:paren 163:164:brace 166:167:variableName.definition 168:170:definitionOperator 175:176:paren 176:177:squareBracket 177:178:squareBracket 178:181:typeName 181:182:separator 183:184:number 184:185:paren 187:188:variableName 188:189:squareBracket 189:190:number 190:191:squareBracket 192:194:updateOperator 195:198:variableName.function 198:199:paren 199:203:variableName 203:204:squareBracket 204:205:number 205:206:squareBracket 206:207:derefOperator 207:208:paren 208:211:typeName 211:212:paren 212:213:paren 215:216:variableName 216:217:squareBracket 217:218:typeName 218:219:squareBracket 219:220:paren 220:221:variableName 221:222:paren 223:232:lineComment 233:234:brace","nodes":"0:235 0:10 0:7 8:10 11:42 43:153 43:46 47:153 47:51 52:53 54:153 54:59 54:55 55:56 56:59 59:153 59:60 62:80 62:80 80:81 82:92 82:92 92:93 94:99 94:99 99:100 102:110 102:110 110:111 112:117 112:117 117:118 119:124 119:124 124:125 126:132 126:132 132:133 134:138 134:138 138:139 140:145 140:145 145:146 147:150 147:150 150:151 152:153 154:234 154:158 159:160 160:162 160:161 161:162 163:234 163:164 166:185 166:167 168:170 171:185 171:175 175:185 175:176 176:181 176:177 177:178 178:181 181:182 183:184 184:185 187:213 187:191 187:188 188:189 189:190 190:191 192:194 195:213 195:198 198:213 198:199 199:212 199:206 199:203 203:204 204:205 205:206 206:207 207:208 208:211 211:212 212:213 215:222 215:222 215:219 215:216 216:219 216:217 217:218 218:219 219:222 219:220 220:221 221:222 223:232 233:234","indent":[0,0,null,null,0,4,4,4,0,0,4,4,4,0,0],"fold":[null,"13:40",null,null,"60:152",null,null,null,null,"164:233",null,null,null,null,null]}"###,
    );
}

#[test]
fn matches_oracle_continuation() {
    check_oracle(
        r###"{"name":"continuation","doc":"func f() {\nlabel:\n\tif x &&\n\t\ty {\n\t\tz :=\n\t\t\tcall(1,\n\t\t\t\tx)\n\t\tgoto label\n\t}\n\t{ x = (1 +\n\t\t2) }\n}\n","tree":"SourceFile(FunctionDecl(func,DefName,Parameters(\"(\",\")\"),Block(\"{\",LabeledStatement(LabelName,\":\",IfStatement(if,BinaryExp(VariableName,LogicOp,VariableName),Block(\"{\",VarDecl(DefName,\":=\",CallExpr(VariableName,Arguments(\"(\",Number,\",\",VariableName,\")\"))),GotoStatement(goto,LabelName),\"}\"))),Block(\"{\",Assignment(VariableName,\"=\",ParenthesizedExpr(\"(\",BinaryExp(Number,ArithOp,Number),\")\")),\"}\"),\"}\")))","highlights":"0:4:definitionKeyword 5:6:variableName.definition.function 6:7:paren 7:8:paren 9:10:brace 11:16:labelName 16:17:punctuation 19:21:controlKeyword 22:23:variableName 24:26:logicOperator 29:30:variableName 31:32:brace 35:36:variableName.definition 37:39:definitionOperator 43:47:variableName.function 47:48:paren 48:49:number 49:50:separator 55:56:variableName 56:57:paren 60:64:controlKeyword 65:70:labelName 72:73:brace 75:76:brace 77:78:variableName 79:80:definitionOperator 81:82:paren 82:83:number 84:85:arithmeticOperator 88:89:number 89:90:paren 91:92:brace 93:94:brace","nodes":"0:95 0:94 0:4 5:6 6:8 6:7 7:8 9:94 9:10 11:73 11:16 16:17 19:73 19:21 22:30 22:23 24:26 29:30 31:73 31:32 35:57 35:36 37:39 43:57 43:47 47:57 47:48 48:49 49:50 55:56 56:57 60:70 60:64 65:70 72:73 75:92 75:76 77:90 77:78 79:80 81:90 81:82 82:89 82:83 84:85 88:89 89:90 91:92 93:94","indent":[0,4,0,8,8,12,17,8,4,4,11,0,0],"fold":["10:93",null,null,"32:72",null,null,null,null,null,"76:91",null,null,null]}"###,
    );
}

#[test]
fn matches_oracle_comments() {
    check_oracle(
        r###"{"name":"comments","doc":"// start\npackage p // semicolon before a line comment\n/* before */ func f(a /* inline */ int) { /* opening\n\t comment */\n\treturn\n}\nvar x = 3; /* tail */","tree":"SourceFile(LineComment,PackageClause(package,DefName),LineComment,BlockComment,FunctionDecl(func,DefName,Parameters(\"(\",Parameter(DefName,BlockComment,TypeName),\")\"),Block(\"{\",BlockComment,ReturnStatement(return),\"}\")),VarDecl(var,VarSpec(DefName,\"=\",Number)),\";\",BlockComment)","highlights":"0:8:lineComment 9:16:moduleKeyword 17:18:variableName.definition 19:53:lineComment 54:66:blockComment 67:71:definitionKeyword 72:73:variableName.definition.function 73:74:paren 74:75:variableName.definition 76:88:blockComment 89:92:typeName 92:93:paren 94:95:brace 96:119:blockComment 121:127:controlKeyword 128:129:brace 130:133:definitionKeyword 134:135:variableName.definition 136:137:definitionOperator 138:139:number 139:140:separator 141:151:blockComment","nodes":"0:151 0:8 9:18 9:16 17:18 19:53 54:66 67:129 67:71 72:73 73:93 73:74 74:92 74:75 76:88 89:92 92:93 94:129 94:95 96:119 121:127 121:127 128:129 130:139 130:133 134:139 134:135 136:137 138:139 139:140 141:151","indent":[0,0,0,null,4,0,0],"fold":[null,null,"95:128",null,null,null,null]}"###,
    );
}

#[test]
fn matches_oracle_queries_after_a_missing_closer() {
    let doc = parse("package p\nfunc f() {\n\tprintln(1)\n");
    assert!(doc.is_recovered());
    let missing = doc
        .nodes()
        .find(|node| node.kind() == GoNodeKind::Error)
        .unwrap();
    assert_eq!(missing.range(), 33..33);
    assert_eq!(missing.parent().unwrap().kind(), GoNodeKind::Block);
    assert_eq!(
        doc.root().children().next_back().unwrap().kind(),
        GoNodeKind::Error
    );
    check_editor_queries(&doc, &serde_json::from_str(r#"{
        "highlights":"0:7:moduleKeyword 8:9:variableName.definition 10:14:definitionKeyword 15:16:variableName.definition.function 16:17:paren 17:18:paren 19:20:brace 22:29:variableName.function 29:30:paren 30:31:number 31:32:paren",
        "indent":[0,0,4,4], "fold":[null,"20:33",null,null]
    }"#).unwrap());
}

#[test]
fn tracks_semicolon_context_after_extending_tokens_and_line_comments() {
    let doc = parse("var x = nil\nvar y = 1 // note\nvar z = 2\n");
    assert!(!doc.is_recovered());
    assert_eq!(
        tree(doc.root()),
        "SourceFile(VarDecl(var,VarSpec(DefName,\"=\",VariableName)),VarDecl(var,VarSpec(DefName,\"=\",Number)),LineComment,VarDecl(var,VarSpec(DefName,\"=\",Number)))"
    );
}

#[test]
fn preserves_a_boolean_when_upstream_needs_a_final_semicolon() {
    let doc = parse("var x = true\n");
    assert!(doc.is_recovered());
    assert_eq!(
        tree(doc.root()),
        r#"SourceFile(VarDecl(var,VarSpec(DefName,"=",Bool)),⚠)"#
    );
    let span = doc.highlights(8..12).unwrap().next().unwrap();
    assert_eq!(span.range, 8..12);
    assert_eq!(span.tags[0].name(), "bool");
    assert_eq!(doc.root().children().next_back().unwrap().range(), 13..13);
}

#[test]
fn keeps_ranges_valid_in_truncated_utf8_documents() {
    let source = "package π\r\nfunc a[T any]() {\r\n  x := `𐐀`\r\n  /* 😀 */\r\n}\r\n";
    for end in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
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
fn keeps_a_malformed_declaration_inside_its_block() {
    let doc = parse("package p\nfunc f() {\n\tvar = ;\n}\n");
    assert!(doc.is_recovered());
    assert_eq!(doc.root().range(), 0..32);
    assert_eq!(
        tree(doc.root()),
        r#"SourceFile(PackageClause(package,DefName),FunctionDecl(func,DefName,Parameters("(",")"),Block("{",VarDecl(var,⚠("=")),";","}")))"#,
    );
    let error = doc
        .nodes()
        .find(|node| node.kind() == GoNodeKind::Error)
        .unwrap();
    assert_eq!(error.range(), 26..27);
    assert_eq!(error.parent().unwrap().kind(), GoNodeKind::VarDecl);
}

#[test]
fn reuses_only_strict_checkpoints_after_repairing_eof() {
    let mut source = (0..20)
        .map(|n| format!("var prefix{n} = {n}\r\n"))
        .collect::<Vec<_>>()
        .concat();
    source.push_str("func f(param int) {\r\n\tvar café = 2\r\n\treturn café\r\n}\r\n");
    let from = source.len() - "}\r\n".len();
    let remove = ChangeSet::single(Change::new(from..source.len(), "")).unwrap();
    let before = parse(&source);
    assert!(!before.is_recovered());
    let broken = before.reparse(&remove).unwrap();
    assert!(broken.is_recovered());
    assert_eq!(broken.reuse_stats(), Default::default());
    assert_eq!(broken.text().as_str(), &source[..from]);

    let restore = ChangeSet::single(Change::new(from..from, "}\r\n")).unwrap();
    let repaired = broken.reparse(&restore).unwrap();
    assert!(!repaired.is_recovered());
    assert_eq!(repaired.text(), before.text());
    assert_eq!(tree(repaired.root()), tree(before.root()));
    let reuse = repaired.reuse_stats();
    assert!((64..from).contains(&reuse.resumed_at));
    assert!(reuse.nodes_reused > 0);
    assert!(source.is_char_boundary(reuse.resumed_at));
    assert_ne!(
        &source.as_bytes()[reuse.resumed_at - 1..reuse.resumed_at + 1],
        b"\r\n"
    );
}

#[test]
fn matches_oracle_unclosed_comment() {
    check_oracle(
        r###"{"name":"unclosed_comment","doc":"package p\n/* missing 😀\n next line\n","tree":"SourceFile(PackageClause(package,DefName),BlockComment(⚠))","highlights":"0:7:moduleKeyword 8:9:variableName.definition 10:37:blockComment","nodes":"0:37 0:9 0:7 8:9 10:37 37:37","indent":[0,0,null,null],"fold":[null,"12:35",null,null]}"###,
    );
}
