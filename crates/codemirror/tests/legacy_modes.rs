//! Fixtures recorded from the SRI-verified npm releases in `docs/codemirror-upstream.json`.
//! The oracle ran `@codemirror/legacy-modes` 6.5.5 on `StringStream` 6.0.0 with
//! tab size 4 and indent unit 2. It ran `StreamLanguage.define` and `parser.parse`
//! from `@codemirror/language` 6.13.1, and read the node styles with `getStyleTags`
//! from `@lezer/highlight` 1.2.5. It also queried language data and indentation.
//! It normalized CRLF to LF for parsing, then mapped UTF-16 offsets to original
//! bytes. A token split inside a surrogate pair was joined to the other half
//! with the same style. The tests require only Rust and do not run the oracle.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{HighlightTag, StreamSettings},
    legacy_modes::{CommentTokens, LegacyMode},
    stream_parser::{StreamParser, Tokenizer},
};
use serde_json::{Value, json};

fn mode(name: &str) -> LegacyMode {
    let export = if name == "asciiarmor" {
        "asciiArmor"
    } else {
        name
    };
    LegacyMode::from_export(&format!("@codemirror/legacy-modes/mode/{name}"), export).unwrap()
}

fn tag_name(tag: &HighlightTag) -> String {
    let mut name = tag.name().to_owned();
    for modifier in tag.modifiers() {
        name.push('.');
        name.push_str(&format!("{modifier:?}").to_ascii_lowercase());
    }
    name
}

#[test]
fn matches_raw_tokens_and_document_highlights_from_the_pinned_exports() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in oracle["cases"].as_array().unwrap() {
        let mode = mode(case["mode"].as_str().unwrap());
        let label = case["label"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let tokens = Tokenizer::new(&mode, source, settings.tab_size, settings.indent_unit)
            .map(|token| {
                let token = token.unwrap();
                assert_eq!(&source[token.range.clone()], token.text);
                json!([token.range.start, token.range.end, token.style])
            })
            .collect::<Vec<_>>();
        assert_eq!(json!(tokens), case["raw"], "tokens: {label}");

        let doc = mode.language().parse(Text::new(source), settings).unwrap();
        let highlighted = doc
            .tokens(0..source.len())
            .unwrap()
            .map(|span| {
                let style = span.tags.iter().map(tag_name).collect::<Vec<_>>().join(" ");
                json!([span.range.start, span.range.end, style])
            })
            .collect::<Vec<_>>();
        assert_eq!(
            json!(highlighted),
            case["highlighted"],
            "highlights: {label}"
        );
    }
}

#[test]
fn matches_mode_names_defaults_and_language_data() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let data = oracle["data"].as_object().unwrap();
    assert_eq!(data.len(), 7);
    for (name, expected) in data {
        let mode = mode(name);
        assert_eq!(json!(mode.name()), expected["name"]);
        assert_eq!(json!(mode.merge_tokens()), expected["mergeTokens"]);
        let lang = mode.language();
        assert_eq!(json!(lang.name()), expected["name"]);
        let doc = lang
            .parse(Text::new(""), StreamSettings::default())
            .unwrap();
        assert_eq!(
            json!(doc.indentation(0, Default::default()).unwrap()),
            expected["indent"]
        );
        let data = doc.language_data_at(0).unwrap();
        assert_eq!(
            json!(
                data.get::<CommentTokens>("commentTokens")
                    .map(|tokens| json!({"line": tokens.line}))
            ),
            expected["commentTokens"]
        );
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            expected["autocomplete"]
        );
    }
    assert_eq!(
        LegacyMode::from_export("@codemirror/legacy-modes/mode/asciiarmor", "asciiarmor"),
        None
    );
    assert_eq!(
        LegacyMode::from_export("@codemirror/legacy-modes/mode/diff.js", "diff"),
        None
    );
    assert_eq!(
        LegacyMode::from_export("@codemirror/legacy-modes/mode/yaml", "yaml"),
        Some(LegacyMode::Yaml)
    );
}

#[test]
fn replays_state_from_the_start_of_edited_lines() {
    let fixtures = [
        (
            LegacyMode::AsciiArmor,
            "-----BEGIN TYPE-----\nAttr: value\n\nAA==\n-----END TYPE-----",
            "AA==",
            "BB/=",
        ),
        (
            LegacyMode::Brainfuck,
            "[x] \n\n  +\t-\n",
            "  +\t-",
            "  ++\t->",
        ),
        (
            LegacyMode::Http,
            "GET / HTTP/1.1\nA: B\n\nBody",
            "Body",
            "X: not a header",
        ),
        (
            LegacyMode::Properties,
            "x=y\\\n\n continued\nlast:key",
            "continued",
            "more\\",
        ),
        (
            LegacyMode::Toml,
            "k = [\n  [1,\n  2]\n]\nname = true",
            "2]",
            "3, 4]",
        ),
    ];
    for (mode, source, old, insert) in fixtures {
        let settings = StreamSettings::default();
        let lang = mode.language();
        let mut doc = lang.parse(Text::new(source), settings).unwrap();
        let start = source.find(old).unwrap();
        let change = Change::new(start..start + old.len(), insert);
        doc.apply_changes(&ChangeSet::single(change).unwrap())
            .unwrap();
        let expected = Text::new(source.replacen(old, insert, 1));
        let fresh = lang.parse(expected.clone(), settings).unwrap();
        assert_eq!(doc.text().as_str(), expected.as_str());
        let range = 0..expected.len();
        assert!(
            doc.tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?}"
        );
    }
}

#[test]
fn toml_strings_advance_through_unicode_line_separators() {
    // The JS scanner can stall on a separator directly after a quote or
    // escape. Rust treats it as a scalar in the token, not as a line ending.
    let mode = LegacyMode::Toml;
    let source = "value = \"\u{2028}x\\a\r\u{2029}b\"";
    let settings = StreamSettings::default();
    let doc = mode.language().parse(Text::new(source), settings).unwrap();
    let mut tokens = doc.tokens(8..source.len()).unwrap();
    let span = tokens.next().unwrap();
    assert_eq!(span.range, 8..source.len());
    assert_eq!(span.tags, &[HighlightTag::new("string")]);
    assert!(tokens.next().is_none());
}

const ORACLE: &str = r###"
{
  "data": {
    "asciiarmor": {"name":"asciiarmor","mergeTokens":true,"indent":null,"commentTokens":null,"autocomplete":null},
    "brainfuck": {"name":"brainfuck","mergeTokens":true,"indent":null,"commentTokens":null,"autocomplete":null},
    "diff": {"name":"diff","mergeTokens":true,"indent":null,"commentTokens":null,"autocomplete":null},
    "http": {"name":"http","mergeTokens":true,"indent":null,"commentTokens":null,"autocomplete":null},
    "properties": {"name":"properties","mergeTokens":true,"indent":null,"commentTokens":null,"autocomplete":null},
    "protobuf": {"name":"protobuf","mergeTokens":true,"indent":null,"commentTokens":null,"autocomplete":["package","message","import","syntax","required","optional","repeated","reserved","default","extensions","packed","bool","bytes","double","enum","float","string","int32","int64","uint32","uint64","sint32","sint64","fixed32","fixed64","sfixed32","sfixed64","option","service","rpc","returns"]},
    "toml": {"name":"toml","mergeTokens":true,"indent":null,"commentTokens":{"line":"#"},"autocomplete":null}
  },
  "cases": [
    {
      "mode": "asciiarmor",
      "label": "ASCII armor blocks and footer validation",
      "source": "unexpected\r\n \u00a0\n-----BEGIN PGP MESSAGE-----\t\nVersion: 1\nComment: example\n\nAQ+/=!?🦊\n-----END OTHER-----\n--not a marker--\n-----END PGP MESSAGE-----\ufeff\n\t\nextra",
      "raw": [
        [0,10,"error"],[12,15,null],[16,44,"tag"],[45,53,"atom"],[53,55,"string"],[56,64,"atom"],
        [64,72,"string"],[74,79,null],[79,80,"error"],[80,81,"error"],[81,85,"error"],[86,105,"error"],
        [106,107,"error"],[107,108,"error"],[108,111,null],[111,112,"error"],[112,113,null],[113,114,"error"],
        [114,120,null],[120,121,"error"],[121,122,"error"],[123,151,"tag"],[152,153,null],[154,159,"error"]
      ],
      "highlighted": [
        [0,10,"invalid"],[16,44,"tagName"],[45,53,"atom"],[53,55,"string"],[56,64,"atom"],[64,72,"string"],
        [79,85,"invalid"],[86,105,"invalid"],[106,108,"invalid"],[111,112,"invalid"],[113,114,"invalid"],
        [120,122,"invalid"],[123,151,"tagName"],[154,159,"invalid"]
      ]
    },
    {
      "mode": "asciiarmor",
      "label": "ASCII armor with no separating blank line",
      "source": "-----BEGIN X------\n \t\nFlag:\ncontinued value\nNotAHeader\nAB/+==\n-----END X------",
      "raw": [
        [0,18,"tag"],[19,21,null],[22,27,"atom"],[28,43,"string"],[44,54,"error"],[55,61,null],[62,78,"tag"]
      ],
      "highlighted": [
        [0,18,"tagName"],[22,27,"atom"],[28,43,"string"],[44,54,"invalid"],[62,78,"tagName"]
      ]
    },
    {
      "mode": "asciiarmor",
      "label": "ASCII armor with optional empty type and a malformed header",
      "source": "-----BEGIN -----\nBad Header: data\n-----END -----\n",
      "raw": [
        [0,16,"tag"],[17,33,"error"],[34,48,"tag"]
      ],
      "highlighted": [
        [0,16,"tagName"],[17,33,"invalid"],[34,48,"tagName"]
      ]
    },
    {
      "mode": "brainfuck",
      "label": "Brainfuck operations and comment continuation",
      "source": "+-<>.,[]\n  +[-] x+ -  \n \t[->] \n+xyz<\n \n [x] \tx+?\n []\n🦊+\t\n",
      "raw": [
        [0,1,"keyword"],[1,2,"keyword"],[2,3,"atom"],[3,4,"atom"],[4,5,"def"],[5,6,"def"],[6,7,"bracket"],
        [7,8,"bracket"],[9,11,null],[11,12,"keyword"],[12,13,"bracket"],[13,14,"keyword"],[14,15,"bracket"],
        [15,16,null],[16,17,"comment"],[17,18,"comment"],[18,19,null],[19,20,"comment"],[20,22,null],
        [23,25,null],[25,26,"comment"],[26,27,"comment"],[27,28,"comment"],[28,29,"comment"],[29,30,null],
        [31,32,"keyword"],[32,33,"comment"],[33,34,"comment"],[34,35,"comment"],[35,36,"comment"],[37,38,null],
        [39,40,null],[40,41,"bracket"],[41,42,"comment"],[42,43,"comment"],[43,45,null],[45,46,"comment"],
        [46,47,"comment"],[47,48,"comment"],[49,50,null],[50,51,"bracket"],[51,52,"bracket"],[53,57,"comment"],
        [57,58,"comment"],[58,59,null]
      ],
      "highlighted": [
        [0,2,"keyword"],[2,4,"atom"],[4,6,"variableName.definition"],[6,8,"bracket"],[11,12,"keyword"],
        [12,13,"bracket"],[13,14,"keyword"],[14,15,"bracket"],[16,18,"comment"],[19,20,"comment"],
        [25,29,"comment"],[31,32,"keyword"],[32,36,"comment"],[40,41,"bracket"],[41,43,"comment"],
        [45,48,"comment"],[50,52,"bracket"],[53,58,"comment"]
      ]
    },
    {
      "mode": "diff",
      "label": "Diff prefixes and trailing whitespace",
      "source": "+猫\t  \r\n-🦊 \n@@ hunk\t\n context\t \n \t   \n!plain\n+++\n-\n@\n+nbsp\u00a0\n-\t\r\n+ \rcontent\n",
      "raw": [
        [0,4,"inserted"],[4,7,"error inserted"],[9,14,"deleted"],[14,15,"error deleted"],[16,23,"meta"],
        [23,24,"error meta"],[25,33,null],[33,35,"error"],[36,41,"error"],[42,48,null],[49,52,"inserted"],
        [53,54,"deleted"],[55,56,"meta"],[57,64,"inserted"],[65,66,"deleted"],[66,67,"error deleted"],
        [69,79,"inserted"]
      ],
      "highlighted": [
        [0,4,"inserted"],[4,7,"inserted"],[9,14,"deleted"],[14,15,"deleted"],[16,23,"meta"],[23,24,"meta"],
        [33,35,"invalid"],[36,41,"invalid"],[49,52,"inserted"],[53,54,"deleted"],[55,56,"meta"],
        [57,64,"inserted"],[65,66,"deleted"],[66,67,"deleted"],[69,79,"inserted"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP request headers and body",
      "source": " \tGET /猫?q=x\tHTTP/1.1\r\nHost: example.com\n:blank name\n\tcontinued: no name\nBad header\n  \nLast:\n\nnot:a header\nHTTP/2.0 200 OK\n",
      "raw": [
        [0,2,null],[2,5,"keyword"],[5,6,null],[6,14,"string.special"],[14,15,null],[15,23,"keyword"],
        [25,30,"atom"],[30,42,"string"],[43,44,"atom"],[44,54,"string"],[55,74,"string"],[75,85,"error"],
        [86,88,"string"],[89,94,"atom"],[96,108,null],[109,124,null]
      ],
      "highlighted": [
        [2,5,"keyword"],[6,14,"string.special"],[15,23,"keyword"],[25,30,"atom"],[30,42,"string"],[43,44,"atom"],
        [44,54,"string"],[55,74,"string"],[75,85,"invalid"],[86,88,"string"],[89,94,"atom"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP response and Unicode whitespace",
      "source": "HTTP/1.1\u00a0000200 OK\nA:B: C\nA\u2028B:C\n\ufeffName: v\n",
      "raw": [
        [0,8,"keyword"],[8,10,null],[10,16,"atom"],[16,17,null],[17,19,null],[20,22,"atom"],[22,26,"string"],
        [27,34,"error"],[35,43,"atom"],[43,45,"string"]
      ],
      "highlighted": [
        [0,8,"keyword"],[10,16,"atom"],[20,22,"atom"],[22,26,"string"],[27,34,"invalid"],[35,43,"atom"],
        [43,45,"string"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP error response",
      "source": "HTTP/1.0 400 Bad Request\nHeader: value",
      "raw": [
        [0,8,"keyword"],[8,9,null],[9,12,"error"],[12,13,null],[13,24,null],[25,32,"atom"],[32,38,"string"]
      ],
      "highlighted": [
        [0,8,"keyword"],[9,12,"invalid"],[25,32,"atom"],[32,38,"string"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP status boundary and a reason on its own line",
      "source": "HTTP/1.1 399\nReason text\nHeader: value",
      "raw": [
        [0,8,"keyword"],[8,9,null],[9,12,"atom"],[13,24,null],[25,32,"atom"],[32,38,"string"]
      ],
      "highlighted": [
        [0,8,"keyword"],[9,12,"atom"],[25,32,"atom"],[32,38,"string"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP malformed method and header recovery",
      "source": "GET! / HTTP/1.1\nX:\n",
      "raw": [
        [0,15,"error"],[16,18,"atom"]
      ],
      "highlighted": [
        [0,15,"invalid"],[16,18,"atom"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP malformed protocol at line end",
      "source": "POST /x HTTP/2.0 \nX: header",
      "raw": [
        [0,4,"keyword"],[4,5,null],[5,7,"string.special"],[7,8,null],[8,17,"error"],[18,20,"atom"],
        [20,27,"string"]
      ],
      "highlighted": [
        [0,4,"keyword"],[5,7,"string.special"],[8,17,"invalid"],[18,20,"atom"],[20,27,"string"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP missing status code",
      "source": "HTTP/1.1 status\nX: header",
      "raw": [
        [0,8,"keyword"],[8,9,null],[9,15,"error"],[16,18,"atom"],[18,25,"string"]
      ],
      "highlighted": [
        [0,8,"keyword"],[9,15,"invalid"],[16,18,"atom"],[18,25,"string"]
      ]
    },
    {
      "mode": "http",
      "label": "HTTP body after initial blank line",
      "source": "\nGET / HTTP/1.1\n",
      "raw": [
        [1,15,null]
      ],
      "highlighted": [

      ]
    },
    {
      "mode": "properties",
      "label": "Properties comments, sections, separators, and continuation",
      "source": " # comment\n!also\n;last\n [first][β] ;after section\n[a] k=\u00a0v:x\\\r\n\n  continued\\\n# next comment\na=b=c:d\r\n   \n[broken\n [🦊] 🦊:z\n",
      "raw": [
        [0,10,"comment"],[11,16,"comment"],[17,22,"comment"],[23,31,"header"],[31,35,"header"],[35,50,"comment"],
        [51,54,"header"],[54,56,"def"],[56,57,null],[57,59,"quote"],[59,60,"quote"],[60,61,null],[61,62,"quote"],
        [62,63,"quote"],[66,69,"quote"],[69,70,"quote"],[70,71,"quote"],[71,72,"quote"],[72,73,"quote"],
        [73,74,"quote"],[74,75,"quote"],[75,76,"quote"],[76,77,"quote"],[77,78,"quote"],[79,93,"comment"],
        [94,95,"def"],[95,96,null],[96,97,"quote"],[97,98,null],[98,99,"quote"],[99,100,null],[100,101,"quote"],
        [103,106,"def"],[107,108,"header"],[108,109,"def"],[109,110,"def"],[110,111,"def"],[111,112,"def"],
        [112,113,"def"],[113,114,"def"],[115,122,"header"],[122,127,"def"],[127,128,null],[128,129,"quote"]
      ],
      "highlighted": [
        [0,10,"comment"],[11,16,"comment"],[17,22,"comment"],[23,35,"heading"],[35,50,"comment"],
        [51,54,"heading"],[54,56,"variableName.definition"],[57,60,"quote"],[61,63,"quote"],[66,78,"quote"],
        [79,93,"comment"],[94,95,"variableName.definition"],[96,97,"quote"],[98,99,"quote"],[100,101,"quote"],
        [103,106,"variableName.definition"],[107,108,"heading"],[108,114,"variableName.definition"],
        [115,122,"heading"],[122,127,"variableName.definition"],[128,129,"quote"]
      ]
    },
    {
      "mode": "properties",
      "label": "Properties continuation into a section",
      "source": "key:value\\\n[section]tail\n[a]\npost\nkey\\: hi\n",
      "raw": [
        [0,1,"def"],[1,2,"def"],[2,3,"def"],[3,4,null],[4,5,"quote"],[5,6,"quote"],[6,7,"quote"],[7,8,"quote"],
        [8,9,"quote"],[9,10,"quote"],[11,20,"header"],[20,21,"def"],[21,22,"def"],[22,23,"def"],[23,24,"def"],
        [25,28,"header"],[29,30,"def"],[30,31,"def"],[31,32,"def"],[32,33,"def"],[34,35,"def"],[35,36,"def"],
        [36,37,"def"],[37,38,"def"],[38,39,null],[39,40,"quote"],[40,41,"quote"],[41,42,"quote"]
      ],
      "highlighted": [
        [0,3,"variableName.definition"],[4,10,"quote"],[11,20,"heading"],[20,24,"variableName.definition"],
        [25,28,"heading"],[29,33,"variableName.definition"],[34,38,"variableName.definition"],[39,42,"quote"]
      ]
    },
    {
      "mode": "protobuf",
      "label": "Protobuf numeric forms, literals, keywords, and identifier boundaries",
      "source": "syntax = \"proto3\";\nMESSAGE X { optional_ Option optioné _v1 Δ猫 é🦊 £𐐀 +0xAE -0x2 0Xf .1D+3 -.8 -8e2 1.e+2 0x 5D- *# } // µ\n\"a\"\"b\" 'a''b' \"unclosed\n\ufeffreserved \u0085 reserved\n",
      "raw": [
        [0,6,"keyword"],[6,7,null],[7,8,null],[8,9,null],[9,17,"string"],[17,18,null],[19,26,"keyword"],
        [26,27,null],[27,28,"variable"],[28,29,null],[29,30,null],[30,31,null],[31,40,"variable"],[40,41,null],
        [41,47,"keyword"],[47,48,null],[48,54,"keyword"],[54,56,"variable"],[56,57,null],[57,60,"variable"],
        [60,61,null],[61,66,"variable"],[66,67,null],[67,73,"variable"],[73,74,null],[74,80,"variable"],
        [80,81,null],[81,86,"number"],[86,87,null],[87,91,"number"],[91,92,null],[92,93,"number"],
        [93,95,"variable"],[95,96,null],[96,101,"number"],[101,102,null],[102,105,"number"],[105,106,null],
        [106,110,"number"],[110,111,null],[111,112,"number"],[112,113,null],[113,114,"variable"],
        [114,116,"number"],[116,117,null],[117,118,"number"],[118,119,"variable"],[119,120,null],
        [120,121,"number"],[121,122,"variable"],[122,123,null],[123,124,null],[124,125,null],[125,126,null],
        [126,127,null],[127,128,null],[128,129,null],[129,134,"comment"],[135,141,"string"],[141,142,null],
        [142,148,"string"],[148,149,null],[149,150,null],[150,158,"variable"],[159,162,null],[162,170,"keyword"],
        [170,171,null],[171,173,null],[173,174,null],[174,182,"keyword"]
      ],
      "highlighted": [
        [0,6,"keyword"],[9,17,"string"],[19,26,"keyword"],[27,28,"variableName"],[31,40,"variableName"],
        [41,47,"keyword"],[48,54,"keyword"],[54,56,"variableName"],[57,60,"variableName"],[61,66,"variableName"],
        [67,73,"variableName"],[74,80,"variableName"],[81,86,"number"],[87,91,"number"],[92,93,"number"],
        [93,95,"variableName"],[96,101,"number"],[102,105,"number"],[106,110,"number"],[111,112,"number"],
        [113,114,"variableName"],[114,116,"number"],[117,118,"number"],[118,119,"variableName"],
        [120,121,"number"],[121,122,"variableName"],[129,134,"comment"],[135,141,"string"],[142,148,"string"],
        [150,158,"variableName"],[162,170,"keyword"],[174,182,"keyword"]
      ]
    },
    {
      "mode": "toml",
      "label": "TOML tables, comments, date, booleans, arrays and number forms",
      "source": "[[bin]] # table\n[a]tail = 2024-02-03T12:30:45.0Z\n\"key x\" = trueish false, -4.20 +3 1e4\nlist = [1,\n\n  [false, -2], # tail\n  \"v\", 3]\n[unfinished",
      "raw": [
        [0,7,"atom"],[7,8,null],[8,15,"comment"],[16,19,"atom"],[19,23,"property"],[23,24,null],[24,25,null],
        [25,26,null],[26,48,"atom"],[49,56,"string"],[56,57,null],[57,58,null],[58,59,null],[59,63,"atom"],
        [63,64,null],[64,65,null],[65,66,null],[66,67,null],[67,72,"atom"],[72,73,null],[73,74,null],
        [74,79,"number"],[79,80,null],[80,81,null],[81,82,"number"],[82,83,null],[83,84,"number"],[84,85,null],
        [85,86,"number"],[87,91,"property"],[91,92,null],[92,93,null],[93,94,null],[94,95,"bracket"],
        [95,96,"number"],[96,97,null],[99,101,null],[101,102,"bracket"],[102,107,"atom"],[107,108,null],
        [108,109,null],[109,111,"number"],[111,112,"bracket"],[112,113,null],[113,114,null],[114,120,"comment"],
        [121,123,null],[123,126,"string"],[126,127,null],[127,128,null],[128,129,"number"],[129,130,"bracket"],
        [131,142,"property"]
      ],
      "highlighted": [
        [0,7,"atom"],[8,15,"comment"],[16,19,"atom"],[19,23,"propertyName"],[26,48,"atom"],[49,56,"string"],
        [59,63,"atom"],[67,72,"atom"],[74,79,"number"],[81,82,"number"],[83,84,"number"],[85,86,"number"],
        [87,91,"propertyName"],[94,95,"bracket"],[95,96,"number"],[101,102,"bracket"],[102,107,"atom"],
        [109,111,"number"],[111,112,"bracket"],[114,120,"comment"],[123,126,"string"],[128,129,"number"],
        [129,130,"bracket"],[131,142,"propertyName"]
      ]
    },
    {
      "mode": "toml",
      "label": "TOML multiline strings, escape pairs, key and value styles",
      "source": "k\tpart = '''\nline\n' \\' end'''\n\"\"\"key\nφ\"\"\" = \"a\\\"b\"\nvalue = \"\"\"\n\nline \\\\ text \"\" close\"\"\"    # note\n'lhs' = 'one'\nlast = \"open\ncontinued🦊\n",
      "raw": [
        [0,6,"property"],[6,7,null],[7,8,null],[8,9,null],[9,12,"string"],[13,17,"string"],[18,29,"string"],
        [30,36,"string"],[37,42,"string"],[42,43,null],[43,44,null],[44,45,null],[45,51,"string"],
        [52,57,"property"],[57,58,null],[58,59,null],[59,60,null],[60,63,"string"],[65,89,"string"],[89,93,null],
        [93,99,"comment"],[100,105,"string"],[105,106,null],[106,107,null],[107,108,null],[108,113,"string"],
        [114,118,"property"],[118,119,null],[119,120,null],[120,121,null],[121,126,"string"],[127,140,"string"]
      ],
      "highlighted": [
        [0,6,"propertyName"],[9,12,"string"],[13,17,"string"],[18,29,"string"],[30,36,"string"],[37,42,"string"],
        [45,51,"string"],[52,57,"propertyName"],[60,63,"string"],[65,89,"string"],[93,99,"comment"],
        [100,105,"string"],[108,113,"string"],[114,118,"propertyName"],[121,126,"string"],[127,140,"string"]
      ]
    }
  ]
}
"###;
