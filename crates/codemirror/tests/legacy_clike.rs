//! Oracle: the unchanged `mode/clike.js` ESM exports from `@codemirror/legacy-modes`
//! 6.5.5, with `StringStream` from `@codemirror/streamparser` 6.0.0. Archives were
//! verified against `docs/codemirror-upstream.json`. The oracle called `token`
//! until the cursor advanced and `indent` before each line with unit 2 and
//! tab size 4. UTF-16 ranges were converted to UTF-8 without counting CRLF
//! as a token. Only Rust runs these tests.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, Tokenizer},
};
use serde_json::{Value, json};

const MODULE: &str = "@codemirror/legacy-modes/mode/clike";

fn mode(export: &str) -> LegacyMode {
    LegacyMode::from_export(MODULE, export).unwrap_or_else(|| panic!("missing {export}"))
}

#[test]
fn concrete_exports_match_the_pinned_stream_oracle() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in oracle["cases"].as_array().unwrap() {
        let export = case["export"].as_str().unwrap();
        let mode = mode(export);
        let source = case["source"].as_str().unwrap();
        assert_eq!(json!(mode.name()), case["name"]);
        assert!(mode.merge_tokens());
        let mut expected = case["tokens"].as_array().unwrap().iter();
        for token in Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit) {
            let token = token.unwrap();
            assert_eq!(token.text, &source[token.range.clone()]);
            assert_eq!(
                &json!([token.range.start, token.range.end, token.style]),
                expected.next().expect("extra token"),
                "{export} at bytes {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert!(expected.next().is_none(), "{export}: missing tokens");

        let lang = mode.language();
        let doc = lang.parse(Text::new(source), settings).unwrap();
        assert_eq!(json!(lang.name()), case["name"]);
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
        let indents = starts
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{export} indentation");

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
        let block = comments
            .block
            .map(|(open, close)| json!({"open": open, "close": close}));
        assert_eq!(
            json!({"line": comments.line, "block": block}),
            case["commentTokens"]
        );
        assert_eq!(
            json!(data.get::<&[&str]>("closeBrackets")),
            case["brackets"],
            "{export} closeBrackets"
        );
        let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        let triggers = oracle["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| trigger(input.as_str().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(json!(triggers), case["trigger"], "{export} indentOnInput");
        let words = data.get::<&[&str]>("autocomplete").unwrap();
        // Compare all entries in upstream order without repeating the word lists.
        let hash = serde_json::to_vec(words)
            .unwrap()
            .into_iter()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        assert_eq!(
            json!(format!("{hash:016x}")),
            case["completions"],
            "{export} autocomplete"
        );
    }
    assert!(LegacyMode::from_export(MODULE, "clike").is_none());
    assert!(LegacyMode::from_export(MODULE, "nesc").is_none());
    assert!(LegacyMode::from_export(MODULE, "objectiveCPP").is_none());
}

#[test]
fn replay_preserves_multiline_scanners_and_context() {
    let settings = StreamSettings::default();
    for (export, source, before, after) in [
        (
            "cpp",
            "auto raw = R\"tag(a\nb\n)tag\";\nint next;",
            "b",
            "more",
        ),
        (
            "ceylon",
            "value s = \"hi ``\nname\n`` there\";\n",
            "name",
            "item.name",
        ),
        (
            "dart",
            "void main() {\nfinal t = '''hello\nworld $name\n''';\n}\n",
            "world",
            "new",
        ),
    ] {
        let language = mode(export).language();
        let mut incremental = language.parse(Text::new(source), settings).unwrap();
        let from = source.find(before).unwrap();
        incremental
            .apply_changes(
                &ChangeSet::single(Change::new(from..from + before.len(), after)).unwrap(),
            )
            .unwrap();
        let updated = source.replacen(before, after, 1);
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            incremental
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{export}"
        );
        for position in updated.match_indices('\n').map(|(i, _)| i + 1) {
            assert_eq!(
                incremental
                    .indentation(position, Default::default())
                    .unwrap(),
                fresh.indentation(position, Default::default()).unwrap(),
                "{export} at {position}"
            );
        }
    }
}

#[test]
fn malformed_ceylon_number_prefixes_advance() {
    // The JS number regex also allows an empty match, which can stall.
    let mode = mode("ceylon");
    let settings = StreamSettings::default();
    let source = "# $word";
    let tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit)
        .map(|token| token.unwrap().text)
        .collect::<Vec<_>>();
    assert_eq!(tokens, ["#", " ", "$word"]);
}

// Byte ranges and styles, including unstyled tokens and unmerged adjacent styles.
const ORACLE: &str = r###"
{
  "inputs": ["{","{}","}","\tcase 3:","case :","default:","case zip","casey:","} "," default:\n","case a\rb:","\ufeff}"],
  "cases": [
    {
      "export": "c",
      "name": "c",
      "source": "#define VALUE \\\n  0x2Au /* units */\n#define NAME \\\n  // an empty continuation\ntypedef int item_t;\nitem_t *\nmeasure(item_t *p) {\n\tswitch (*p) {\n\tcase 0:\n\t\treturn __LINE__;\n\tdefault:\n\t\treturn _X;\n\t}\n}\n",
      "indents": [0,null,0,null,0,0,null,2,6,8,6,8,4,0,null],
      "tokens": [
        [0,15,"meta"],[16,18,null],[18,24,"meta"],[24,35,"comment"],[36,50,"meta"],[51,53,null],[53,77,"comment"],
        [78,85,"keyword"],[85,86,null],[86,89,"type"],[89,90,null],[90,96,"type"],[96,97,null],[98,104,"type"],
        [104,105,null],[105,106,"type"],[107,114,"def"],[114,115,null],[115,121,"type"],[121,122,null],[122,123,"type"],
        [123,124,"variable"],[124,125,null],[125,126,null],[126,127,null],[128,129,null],[129,135,"keyword"],[135,136,null],
        [136,137,null],[137,138,"operator"],[138,139,"variable"],[139,140,null],[140,141,null],[141,142,null],[143,144,null],
        [144,148,"keyword"],[148,149,null],[149,150,"number"],[150,151,null],[152,154,null],[154,160,"keyword"],[160,161,null],
        [161,169,"builtin"],[169,170,null],[171,172,null],[172,179,"keyword"],[179,180,null],[181,183,null],[183,189,"keyword"],
        [189,190,null],[190,192,"builtin"],[192,193,null],[194,195,null],[195,196,null],[197,198,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "daf3163d1ad27d8b"
    },
    {
      "export": "cpp",
      "name": "cpp",
      "source": "template <typename T>\nconcept Frozen = true;\nnamespace lib {\nBox::Box() {}\nBox::~Box() {}\nconst char *s = u8R\"tag[!](one\ntwo)tag[!]\";\nauto t = L\"text\";\nlong n = 1'234ULL;\n}\n",
      "indents": [0,0,0,2,2,2,null,2,2,0,0],
      "tokens": [
        [0,8,"keyword"],[8,9,null],[9,10,"operator"],[10,18,"keyword"],[18,19,null],[19,20,"variable"],[20,21,"operator"],
        [22,29,"keyword"],[29,30,null],[30,36,"variable"],[36,37,null],[37,38,"operator"],[38,39,null],[39,43,"atom"],
        [43,44,null],[45,54,"keyword"],[54,55,null],[55,58,"def"],[58,59,null],[59,60,null],[61,69,"variable"],
        [69,70,null],[70,71,null],[71,72,null],[72,73,null],[73,74,null],[75,84,"def"],[84,85,null],
        [85,86,null],[86,87,null],[87,88,null],[88,89,null],[90,95,"keyword"],[95,96,null],[96,100,"type"],
        [100,101,null],[101,102,"type"],[102,103,"variable"],[103,104,null],[104,105,"operator"],[105,106,null],[106,120,"string"],
        [121,132,"string"],[132,133,null],[134,138,"keyword"],[138,139,null],[139,140,"variable"],[140,141,null],[141,142,"operator"],
        [142,143,null],[143,144,"string"],[144,150,"string"],[150,151,null],[152,156,"type"],[156,157,null],[157,158,"variable"],
        [158,159,null],[159,160,"operator"],[160,161,null],[161,169,"number"],[169,170,null],[171,172,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "7976c94af7cac4b8"
    },
    {
      "export": "java",
      "name": "java",
      "source": "@interface Note {}\n@Note class Sample {\n  String s = \"\"\"\n  text \\\"\"\"\n  next\"\"\";\n  int x = 0b1100_0101 + 12_345L;\n  var n = .5e+2f;\n}\n",
      "indents": [0,0,2,null,null,2,2,0,0],
      "tokens": [
        [0,10,"keyword"],[10,11,null],[11,15,"def"],[15,16,null],[16,17,null],[17,18,null],[19,24,"meta"],
        [24,25,null],[25,30,"keyword"],[30,31,null],[31,37,"def"],[37,38,null],[38,39,null],[40,42,null],
        [42,48,"type"],[48,49,null],[49,50,"variable"],[50,51,null],[51,52,"operator"],[52,53,null],[53,56,"string"],
        [57,59,null],[59,68,"string"],[69,71,null],[71,78,"string"],[78,79,null],[80,82,null],[82,85,"type"],
        [85,86,null],[86,87,"variable"],[87,88,null],[88,89,"operator"],[89,90,null],[90,101,"number"],[101,102,null],
        [102,103,"operator"],[103,104,null],[104,111,"number"],[111,112,null],[113,115,null],[115,118,"type"],[118,119,null],
        [119,120,"variable"],[120,121,null],[121,122,"operator"],[122,123,null],[123,129,"number"],[129,130,null],[131,132,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "1b3a4b403759804a"
    },
    {
      "export": "csharp",
      "name": "csharp",
      "source": "#region Name\r\nnamespace Example {\r\n  record Entry(int Id);\r\n  @select var raw = @\"a \"\"quote\"\"\r\n  and 🦊 text\";\r\n  if (true)\r\n    raw = null;\r\n}\r\n",
      "indents": [0,null,2,2,null,2,4,0,null],
      "tokens": [
        [0,7,"variable"],[7,8,null],[8,12,"variable"],[14,23,"keyword"],[23,24,null],[24,31,"def"],[31,32,null],
        [32,33,null],[35,37,null],[37,43,"keyword"],[43,44,null],[44,49,"def"],[49,50,null],[50,53,"type"],
        [53,54,null],[54,56,"variable"],[56,57,null],[57,58,null],[60,62,null],[62,69,"meta"],[69,70,null],
        [70,73,"keyword"],[73,74,null],[74,77,"def"],[77,78,null],[78,79,"operator"],[79,80,null],[80,93,"string"],
        [95,97,null],[97,111,"string"],[111,112,null],[114,116,null],[116,118,"keyword"],[118,119,null],[119,120,null],
        [120,124,"atom"],[124,125,null],[127,131,null],[131,134,"variable"],[134,135,null],[135,136,"operator"],[136,137,null],
        [137,141,"atom"],[141,142,null],[144,145,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "e990de632a2e099b"
    },
    {
      "export": "scala",
      "name": "scala",
      "source": "@main object Greeting {\n  /* outer\n     /* inner\n     */ middle\n     */\n  /* both /* same */ tail */ val a = 'symbol\n  val c = '\\u00f1'\n  val f = { x =>\n    println(x)\n  }\n  val s = \"\"\"hi\n  there\"\"\"\n}\n",
      "indents": [0,2,null,null,null,2,2,2,4,2,2,null,0,0],
      "tokens": [
        [0,5,"meta"],[5,6,null],[6,12,"keyword"],[12,13,null],[13,21,"def"],[21,22,null],[22,23,null],
        [24,26,null],[26,34,"comment"],[35,40,null],[40,48,"comment"],[49,54,null],[54,63,"comment"],[64,69,null],
        [69,71,"comment"],[72,74,null],[74,100,"comment"],[100,101,null],[101,104,"keyword"],[104,105,null],[105,106,"def"],
        [106,107,null],[107,108,"operator"],[108,109,null],[109,116,"atom"],[117,119,null],[119,122,"keyword"],[122,123,null],
        [123,124,"def"],[124,125,null],[125,126,"operator"],[126,127,null],[127,135,"character"],[136,138,null],[138,141,"keyword"],
        [141,142,null],[142,143,"def"],[143,144,null],[144,145,"operator"],[145,146,null],[146,147,null],[147,148,null],
        [148,149,"variable"],[149,150,null],[150,152,"operator"],[153,157,null],[157,164,"keyword"],[164,165,null],[165,166,"variable"],
        [166,167,null],[168,170,null],[170,171,null],[172,174,null],[174,177,"keyword"],[177,178,null],[178,179,"def"],
        [179,180,null],[180,181,"operator"],[181,182,null],[182,187,"string"],[188,190,null],[190,198,"string"],[199,200,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": ["(","[","{","'","\"","\"\"\""],
      "trigger": [true,false,true,false,false,false,false,false,false,false,false,true],
      "completions": "3db08769ba252a99"
    },
    {
      "export": "kotlin",
      "name": "kotlin",
      "source": "@file:JvmName(\"App\")\nimport foo.*\nval sum = 1 +\n    2\nval chain = source\n    .method()\nfun run(x: Int): Unit { use(\n    0xAB_CuL, .75f)\n    .again()\n  val s = \"a ${f(\"x\")}\"\n  val block = \"\"\"top\n  $x\"\"\"\n  /* first\n  /* next\n  */ inner\n  */\n  val that = thing\n    .foo()\n}\n",
      "indents": [0,0,0,2,0,2,0,2,4,2,2,null,2,null,null,null,2,4,0,0],
      "tokens": [
        [0,5,"meta"],[5,6,null],[6,13,"variable"],[13,14,null],[14,19,"string"],[19,20,null],[21,27,"keyword"],
        [27,28,null],[28,31,"variable"],[31,32,null],[32,33,"variable"],[34,37,"keyword"],[37,38,null],[38,41,"def"],
        [41,42,null],[42,43,"operator"],[43,44,null],[44,45,"number"],[45,46,null],[46,47,"operator"],[48,52,null],
        [52,53,"number"],[54,57,"keyword"],[57,58,null],[58,63,"def"],[63,64,null],[64,65,"operator"],[65,66,null],
        [66,72,"variable"],[73,77,null],[77,78,null],[78,84,"variable"],[84,85,null],[85,86,null],[87,90,"keyword"],
        [90,91,null],[91,94,"def"],[94,95,null],[95,96,"variable"],[96,97,null],[97,98,null],[98,101,"type"],
        [101,102,null],[102,103,null],[103,104,null],[104,108,"type"],[108,109,null],[109,110,null],[110,111,null],
        [111,114,"variable"],[114,115,null],[116,120,null],[120,128,"number"],[128,129,null],[129,130,null],[130,134,"number"],
        [134,135,null],[136,140,null],[140,141,null],[141,146,"variable"],[146,147,null],[147,148,null],[149,151,null],
        [151,154,"keyword"],[154,155,null],[155,156,"def"],[156,157,null],[157,158,"operator"],[158,159,null],[159,172,"string"],
        [173,175,null],[175,178,"keyword"],[178,179,null],[179,184,"def"],[184,185,null],[185,186,"operator"],[186,187,null],
        [187,193,"string"],[194,196,null],[196,201,"string"],[202,204,null],[204,212,"comment"],[213,215,null],[215,222,"comment"],
        [223,225,null],[225,233,"comment"],[234,236,null],[236,238,"comment"],[239,241,null],[241,244,"keyword"],[244,245,null],
        [245,249,"def"],[249,250,null],[250,251,"operator"],[251,252,null],[252,257,"variable"],[258,262,null],[262,263,null],
        [263,266,"variable"],[266,267,null],[267,268,null],[269,270,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": ["(","[","{","'","\"","\"\"\""],
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "bd1636ed775b7973"
    },
    {
      "export": "shader",
      "name": "shader",
      "source": "#version 120\nuniform sampler2D tex;\nvoid main() {\n  vec3 v = normalize(vec3(0.5, 0.0, 1.0));\n  gl_FragColor = texture2D(tex, gl_TexCoord[0].xy);\n}\n",
      "indents": [0,0,0,2,2,0,0],
      "tokens": [
        [0,12,"meta"],[13,20,"keyword"],[20,21,null],[21,30,"keyword"],[30,31,null],[31,34,"variable"],[34,35,null],
        [36,40,"type"],[40,41,null],[41,45,"variable"],[45,46,null],[46,47,null],[47,48,null],[48,49,null],
        [50,52,null],[52,56,"type"],[56,57,null],[57,58,"variable"],[58,59,null],[59,60,"operator"],[60,61,null],
        [61,70,"builtin"],[70,71,null],[71,75,"type"],[75,76,null],[76,79,"number"],[79,80,null],[80,81,null],
        [81,84,"number"],[84,85,null],[85,86,null],[86,89,"number"],[89,90,null],[90,91,null],[91,92,null],
        [93,95,null],[95,107,"atom"],[107,108,null],[108,109,"operator"],[109,110,null],[110,119,"builtin"],[119,120,null],
        [120,123,"variable"],[123,124,null],[124,125,null],[125,136,"atom"],[136,137,null],[137,138,"number"],[138,139,null],
        [139,140,null],[140,142,"variable"],[142,143,null],[143,144,null],[145,146,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,false,true,false,false,false,false,false,false,false,false,true],
      "completions": "88c35d1c1da6df24"
    },
    {
      "export": "nesC",
      "name": "nesc",
      "source": "#include \"Timer.h\"\nmodule Blink { uses interface Timer; }\nimplementation {\n  task void work() { call Timer.start(); }\n  async event void Timer.fired() { post work(); }\n  nx_struct msg { uint16_t value; };\n}\n",
      "indents": [0,0,0,2,2,2,0,0],
      "tokens": [
        [0,18,"meta"],[19,25,"keyword"],[25,26,null],[26,31,"variable"],[31,32,null],[32,33,null],[33,34,null],
        [34,38,"keyword"],[38,39,null],[39,48,"keyword"],[48,49,null],[49,54,"variable"],[54,55,null],[55,56,null],
        [56,57,null],[58,72,"keyword"],[72,73,null],[73,74,null],[75,77,null],[77,81,"keyword"],[81,82,null],
        [82,86,"type"],[86,87,null],[87,91,"variable"],[91,92,null],[92,93,null],[93,94,null],[94,95,null],
        [95,96,null],[96,100,"keyword"],[100,101,null],[101,106,"variable"],[106,107,null],[107,112,"variable"],[112,113,null],
        [113,114,null],[114,115,null],[115,116,null],[116,117,null],[118,120,null],[120,125,"keyword"],[125,126,null],
        [126,131,"keyword"],[131,132,null],[132,136,"type"],[136,137,null],[137,142,"variable"],[142,143,null],[143,148,"variable"],
        [148,149,null],[149,150,null],[150,151,null],[151,152,null],[152,153,null],[153,157,"keyword"],[157,158,null],
        [158,162,"variable"],[162,163,null],[163,164,null],[164,165,null],[165,166,null],[166,167,null],[168,170,null],
        [170,179,"keyword"],[179,180,null],[180,183,"variable"],[183,184,null],[184,185,null],[185,186,null],[186,194,"type"],
        [194,195,null],[195,200,"variable"],[200,201,null],[201,202,null],[202,203,null],[203,204,null],[205,206,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "cc31d3decfa6bcf1"
    },
    {
      "export": "objectiveC",
      "name": "objectivec",
      "source": "#import <Foundation/Foundation.h>\nNS_ASSUME_NONNULL_BEGIN\n@interface Widget : NSObject\n@property (nonatomic, readonly) BOOL ready;\n@end\n@implementation Widget\n- (instancetype)init {\n  @autoreleasepool {\n    id value = @\"héllo\";\n    return nil;\n  }\n}\n@end\n",
      "indents": [0,0,2,null,0,0,0,2,4,4,2,0,0,0],
      "tokens": [
        [0,33,"meta"],[34,57,"builtin"],[58,68,"keyword"],[68,69,null],[69,75,"def"],[75,76,null],[76,77,null],
        [77,78,null],[78,86,"variable"],[87,96,"keyword"],[96,97,null],[97,98,null],[98,107,"keyword"],[107,108,null],
        [108,109,null],[109,117,"keyword"],[117,118,null],[118,119,null],[119,123,"type"],[123,124,null],[124,129,"variable"],
        [129,130,null],[131,135,"keyword"],[136,151,"keyword"],[151,152,null],[152,158,"def"],[159,160,"operator"],[160,161,null],
        [161,162,null],[162,174,"type"],[174,175,null],[175,179,"variable"],[179,180,null],[180,181,null],[182,184,null],
        [184,200,"keyword"],[200,201,null],[201,202,null],[203,207,null],[207,209,"type"],[209,210,null],[210,215,"variable"],
        [215,216,null],[216,217,"operator"],[217,218,null],[218,219,"variable"],[219,227,"string"],[227,228,null],[229,233,null],
        [233,239,"keyword"],[239,240,null],[240,243,"atom"],[243,244,null],[245,247,null],[247,248,null],[249,250,null],
        [251,255,"keyword"]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "77a8a5e59f660ae0"
    },
    {
      "export": "objectiveCpp",
      "name": "objectivecpp",
      "source": "@interface Widget : NSObject\n@end\nnamespace app {\nWidget::Widget() {}\nNSString *text = @\"hi\";\nauto s = uR\"d(line\nmore)d\";\nvector_t *value = nullptr;\n}\n",
      "indents": [0,null,null,2,2,2,null,2,0,null],
      "tokens": [
        [0,10,"keyword"],[10,11,null],[11,17,"def"],[17,18,null],[18,19,null],[19,20,null],[20,28,"variable"],
        [29,33,"keyword"],[34,43,"keyword"],[43,44,null],[44,47,"def"],[47,48,null],[48,49,null],[50,64,"variable"],
        [64,65,null],[65,66,null],[66,67,null],[67,68,null],[68,69,null],[70,78,"variable"],[78,79,null],
        [79,80,"operator"],[80,84,"variable"],[84,85,null],[85,86,"operator"],[86,87,null],[87,88,"variable"],[88,92,"string"],
        [92,93,null],[94,98,"keyword"],[98,99,null],[99,100,"variable"],[100,101,null],[101,102,"operator"],[102,103,null],
        [103,112,"string"],[113,120,"string"],[120,121,null],[122,130,"type"],[130,131,null],[131,132,"type"],[132,137,"variable"],
        [137,138,null],[138,139,"operator"],[139,140,null],[140,147,"atom"],[147,148,null],[149,150,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "1415e4f9b3db6d8a"
    },
    {
      "export": "squirrel",
      "name": "squirrel",
      "source": "# integration\nclass Worker {\n  local field = true;\n  function work(x) {\n    foreach (value in x)\n      yield clone value;\n  }\n}\nfunction use(item_t value) { return null; }",
      "indents": [0,0,2,2,4,6,2,0,0],
      "tokens": [
        [0,13,"meta"],[14,19,"keyword"],[19,20,null],[20,26,"def"],[26,27,null],[27,28,null],[29,31,null],
        [31,36,"keyword"],[36,37,null],[37,42,"def"],[42,43,null],[43,44,"operator"],[44,45,null],[45,49,"atom"],
        [49,50,null],[51,53,null],[53,61,"keyword"],[61,62,null],[62,66,"def"],[66,67,null],[67,68,"variable"],
        [68,69,null],[69,70,null],[70,71,null],[72,76,null],[76,83,"keyword"],[83,84,null],[84,85,null],
        [85,90,"variable"],[90,91,null],[91,93,"keyword"],[93,94,null],[94,95,"variable"],[95,96,null],[97,103,null],
        [103,108,"keyword"],[108,109,null],[109,114,"keyword"],[114,115,null],[115,120,"variable"],[120,121,null],[122,124,null],
        [124,125,null],[126,127,null],[128,136,"keyword"],[136,137,null],[137,140,"def"],[140,141,null],[141,147,"type"],
        [147,148,null],[148,153,"variable"],[153,154,null],[154,155,null],[155,156,null],[156,157,null],[157,163,"keyword"],
        [163,164,null],[164,168,"atom"],[168,169,null],[169,170,null],[170,171,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "93ea623975a3c933"
    },
    {
      "export": "ceylon",
      "name": "ceylon",
      "source": "@doc shared class Widget() {\n  value text = \"hello ``name``!\";\n  value raw = \"\"\"hi\n    ``item.Name``!\"\"\";\n  value ch = 'x';\n  value num = #a_b + $01_0 + 1.5k;\n  value next = item.name;\n}\n",
      "indents": [0,2,2,null,2,2,2,0,0],
      "tokens": [
        [0,4,"meta"],[4,5,null],[5,11,"builtin"],[11,12,null],[12,17,"keyword"],[17,18,null],[18,24,"type"],
        [24,25,null],[25,26,null],[26,27,null],[27,28,null],[29,31,null],[31,36,"keyword"],[36,37,null],
        [37,41,"variable"],[41,42,null],[42,43,"operator"],[43,44,null],[44,53,"string"],[53,57,"variable"],[57,61,"string"],
        [61,62,null],[63,65,null],[65,70,"keyword"],[70,71,null],[71,74,"variable"],[74,75,null],[75,76,"operator"],
        [76,77,null],[77,82,"string"],[83,87,null],[87,89,"string"],[89,93,"variable"],[93,94,null],[94,98,"variableName.special"],
        [98,104,"string"],[104,105,null],[106,108,null],[108,113,"keyword"],[113,114,null],[114,116,"variable"],[116,117,null],
        [117,118,"operator"],[118,119,null],[119,122,"string.special"],[122,123,null],[124,126,null],[126,131,"keyword"],[131,132,null],
        [132,135,"variable"],[135,136,null],[136,137,"operator"],[137,138,null],[138,142,"number"],[142,143,null],[143,144,"operator"],
        [144,145,null],[145,150,"number"],[150,151,null],[151,152,"operator"],[152,153,null],[153,154,"number"],[154,155,null],
        [155,157,"number"],[157,158,null],[159,161,null],[161,166,"keyword"],[166,167,null],[167,171,"variable"],[171,172,null],
        [172,173,"operator"],[173,174,null],[174,178,"variable"],[178,179,null],[179,183,"variableName.special"],[183,184,null],[185,186,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": ["(","[","{","'","\"","\"\"\""],
      "trigger": [true,false,true,false,false,false,false,false,false,false,false,true],
      "completions": "aff1277e6048f8e4"
    },
    {
      "export": "dart",
      "name": "dart",
      "source": "@meta.override\nrequired Thing run(_Thing input) {\n  final result = r\"$raw\";\n  final s = \"hey $name: ${input.value}\";\n  final t = '''first\n    second $name''';\n  /* top\n  /* in\n  */ after\n  */\n  return const Thing(0xAB_CD, .4e+3);\n}\n",
      "indents": [0,0,2,2,2,null,2,null,null,null,2,0,0],
      "tokens": [
        [0,14,"meta"],[15,23,"keyword"],[23,24,null],[24,29,"type"],[29,30,null],[30,33,"variable"],[33,34,null],
        [34,40,"type"],[40,41,null],[41,46,"variable"],[46,47,null],[47,48,null],[48,49,null],[50,52,null],
        [52,57,"keyword"],[57,58,null],[58,64,"variable"],[64,65,null],[65,66,"operator"],[66,67,null],[67,74,"string"],
        [74,75,null],[76,78,null],[78,83,"keyword"],[83,84,null],[84,85,"variable"],[85,86,null],[86,87,"operator"],
        [87,88,null],[88,93,"string"],[93,94,null],[94,98,"variable"],[98,100,"string"],[100,102,null],[102,107,"variable"],
        [107,108,null],[108,113,"variable"],[113,114,null],[114,115,"string"],[115,116,null],[117,119,null],[119,124,"keyword"],
        [124,125,null],[125,126,"variable"],[126,127,null],[127,128,"operator"],[128,129,null],[129,137,"string"],[138,142,null],
        [142,149,"string"],[149,150,null],[150,154,"variable"],[154,157,"string"],[157,158,null],[159,161,null],[161,167,"comment"],
        [168,170,null],[170,175,"comment"],[176,178,null],[178,186,"comment"],[187,189,null],[189,191,"comment"],[192,194,null],
        [194,200,"keyword"],[200,201,null],[201,206,"keyword"],[206,207,null],[207,212,"type"],[212,213,null],[213,220,"number"],
        [220,221,null],[221,222,null],[222,227,"number"],[227,228,null],[228,229,null],[230,231,null]
      ],
      "commentTokens": {"line":"//","block":{"open":"/*","close":"*/"}},
      "brackets": null,
      "trigger": [true,true,true,true,true,true,false,false,false,false,false,true],
      "completions": "970a2c9dc39ca2da"
    }
  ]
}
"###;
