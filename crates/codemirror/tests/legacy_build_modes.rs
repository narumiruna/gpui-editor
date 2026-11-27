//! Oracle: the unchanged ESM grammars from `@codemirror/legacy-modes` 6.5.5
//! and `StringStream` from `@codemirror/streamparser` 6.0.0. Both tarballs
//! match their SRI digests in `docs/codemirror-upstream.json`.
//! `token` ran until the cursor advanced. `indent`, where present, ran before
//! each line with indent unit 2 and tab size 4. Offsets use UTF-8 bytes and
//! exclude line endings from the tokens. Tokens split inside a surrogate
//! pair were joined where their styles agreed. Only Rust runs these tests.
//!
//! The first three inputs are excerpts from the mode demos in CodeMirror 5
//! commit `b0c45cf0fbd3dc7cb2016a79fb81c723827f4e31`, `mode/*/index.html`.
//!
//! MIT License
//!
//! Copyright (C) 2017 by Marijn Haverbeke <marijn@haverbeke.berlin> and others
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
//! THE SOFTWARE.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";

fn mode(module: &str, export: &str) -> LegacyMode {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
        .unwrap_or_else(|| panic!("missing {module}/{export}"))
}

#[test]
fn matches_the_pinned_build_mode_oracle() {
    let cases: Vec<Value> = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for case in cases {
        let export = case["export"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let mode = mode(case["module"].as_str().unwrap(), export);
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
                "{export} at bytes {:?} ({:?})",
                token.range,
                token.text,
            );
        }
        assert!(expected.next().is_none(), "{export}: missing tokens");

        let language = mode.language();
        assert_eq!(language.name(), mode.name().unwrap_or_default());
        let doc = language.parse(Text::new(source), settings).unwrap();
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
        let indents = starts
            .map(|pos| doc.indentation(pos, Default::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(json!(indents), case["indents"], "{export} indentation");

        let data = doc.language_data_at(0).unwrap();
        let comments = data.get::<CommentTokens<'static>>("commentTokens");
        assert_eq!(
            json!(comments.and_then(|tokens| tokens.line)),
            case["comments"]
        );
        assert!(comments.is_none_or(|tokens| tokens.block.is_none()));
        assert_eq!(
            json!(data.get::<&[&str]>("closeBrackets")),
            case["brackets"]
        );
        assert_eq!(
            json!(data.get::<&[&str]>("autocomplete")),
            case["autocomplete"]
        );
    }
}

#[test]
fn keeps_the_published_module_and_export_spelling() {
    assert_eq!(mode("shell", "shell"), LegacyMode::Shell);
    assert_eq!(mode("cmake", "cmake"), LegacyMode::CMake);
    assert_eq!(mode("dockerfile", "dockerFile"), LegacyMode::Dockerfile);
    assert!(LegacyMode::from_export(&format!("{PREFIX}dockerfile"), "dockerfile").is_none());
    assert!(LegacyMode::from_export(&format!("{PREFIX}makefile"), "makefile").is_none());
}

#[test]
fn replays_edits_inside_multiline_state() {
    let settings = StreamSettings::default();
    for (mode, source, before, after) in [
        (
            LegacyMode::Shell,
            "cat <<'END'\nbody\nEND\necho done\n",
            "body",
            "END\nbody",
        ),
        (
            LegacyMode::CMake,
            "message(\"first\nsecond ${n}\")\n",
            "second",
            "${n} tail",
        ),
        (
            LegacyMode::Dockerfile,
            "RUN printf \"open\\\npart\" tail\nENV done=1\n",
            "part",
            "done\"\npart",
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
        let fresh = language.parse(Text::new(&updated), settings).unwrap();
        let range = 0..updated.len();
        assert!(
            replay
                .tokens(range.clone())
                .unwrap()
                .eq(fresh.tokens(range).unwrap()),
            "{mode:?}"
        );
    }
}

#[test]
fn copies_dockerfile_pending_capture_tokens() {
    let mode = LegacyMode::Dockerfile;
    let settings = StreamSettings::default();
    let line = "  RUN [\"echo\"]";
    let stream = || StringStream::new(line, settings.tab_size, settings.indent_unit);
    let mut first = stream();
    let mut state = mode.start_state(settings.indent_unit);
    assert_eq!(mode.token(&mut first, &mut state), None);
    assert_eq!(first.pos(), 2);

    let mut second = stream();
    second.set_pos(first.pos()).unwrap();
    let mut copied = mode.copy_state(&state);
    for (text, style) in [
        ("RUN", Some("keyword")),
        (" [", None),
        ("\"echo\"", Some("string")),
    ] {
        for (stream, state) in [(&mut first, &mut state), (&mut second, &mut copied)] {
            stream.start_token();
            assert_eq!(mode.token(stream, state).as_deref(), style);
            assert_eq!(stream.current(), text);
        }
    }
}

// Byte ranges and raw styles, one row per logical line (including blank lines).
const ORACLE: &str = r###"
[
{
  "module": "shell",
  "export": "shell",
  "name": "shell",
  "source": "#!/bin/bash\n\n# here is how to stop the server\nfor pid in `ps aux | grep 'node ../server.js' | awk '{print $2}'` ; do\n  sudo kill -9 $pid 2> /dev/null\ndone\n\nexit 0\n",
  "indents": [null,null,null,null,null,null,null,null,null],
  "autocomplete": ["true","false","if","then","do","else","elif","while","until","for","in","esac","fi","fin","fil","done","exit","set","unset","export","function","ab","awk","bash","beep","cat","cc","cd","chown","chmod","chroot","clear","cp","curl","cut","diff","echo","find","gawk","gcc","get","git","grep","hg","kill","killall","ln","ls","make","mkdir","openssl","mv","nc","nl","node","npm","ping","ps","restart","rm","rmdir","sed","service","sh","shopt","shred","source","sort","sleep","ssh","start","stop","su","sudo","svn","tee","telnet","top","touch","vi","vim","wall","wc","wget","who","write","yes","zsh"],
  "brackets": ["(","[","{","'","\"","`"],
  "comments": "#",
  "tokens": [
    [[0,11,"meta"]],
    [],
    [[13,45,"comment"]],
    [[46,49,"keyword"],[49,50,null],[50,53,null],[53,54,null],[54,56,"keyword"],[56,57,null],[57,72,"quote"],[72,91,"string"],[91,98,"quote"],[98,110,"string"],[110,111,"quote"],[111,112,null],[112,113,null],[113,114,null],[114,116,"keyword"]],
    [[117,119,null],[119,123,"builtin"],[123,124,null],[124,128,"builtin"],[128,129,null],[129,131,"attribute"],[131,132,null],[132,136,"def"],[136,137,null],[137,138,"number"],[138,139,null],[139,140,null],[140,144,null],[144,149,null]],
    [[150,154,"keyword"]],
    [],
    [[156,160,"keyword"],[160,161,null],[161,162,"number"]],
    []
  ]
},
{
  "module": "cmake",
  "export": "cmake",
  "name": "cmake",
  "source": "project (x265)\ncmake_minimum_required (VERSION 2.8.8) # OBJECT libraries require 2.8.8\ninclude(CheckIncludeFiles)\ninclude(CheckFunctionExists)\ninclude(CheckSymbolExists)\ninclude(CheckCXXCompilerFlag)\n\n# X265_BUILD must be incremented each time the public API is changed\nset(X265_BUILD 48)\nconfigure_file(\"${PROJECT_SOURCE_DIR}/x265.def.in\"\n               \"${PROJECT_BINARY_DIR}/x265.def\")\nconfigure_file(\"${PROJECT_SOURCE_DIR}/x265_config.h.in\"\n               \"${PROJECT_BINARY_DIR}/x265_config.h\")\n\nSET(CMAKE_MODULE_PATH \"${PROJECT_SOURCE_DIR}/cmake\" \"${CMAKE_MODULE_PATH}\")\n\n",
  "indents": [null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
  "autocomplete": null,
  "brackets": null,
  "comments": null,
  "tokens": [
    [[0,8,"def"],[8,9,"bracket"],[9,13,null],[13,14,"bracket"]],
    [[15,38,"def"],[38,39,"bracket"],[39,46,null],[46,47,null],[47,48,"number"],[48,50,null],[50,52,null],[52,53,"bracket"],[53,54,null],[54,86,"comment"]],
    [[87,94,"def"],[94,95,"bracket"],[95,112,null],[112,113,"bracket"]],
    [[114,121,"def"],[121,122,"bracket"],[122,141,null],[141,142,"bracket"]],
    [[143,150,"def"],[150,151,"bracket"],[151,168,null],[168,169,"bracket"]],
    [[170,177,"def"],[177,178,"bracket"],[178,198,null],[198,199,"bracket"]],
    [],
    [[201,269,"comment"]],
    [[270,273,"def"],[273,274,"bracket"],[274,284,null],[284,285,null],[285,286,"number"],[286,287,"number"],[287,288,"bracket"]],
    [[289,303,"def"],[303,304,"bracket"],[304,305,"string"],[305,326,"variableName.special"],[326,339,"string"]],
    [[340,355,null],[355,356,"string"],[356,377,"variableName.special"],[377,387,"string"],[387,388,"bracket"]],
    [[389,403,"def"],[403,404,"bracket"],[404,405,"string"],[405,426,"variableName.special"],[426,444,"string"]],
    [[445,460,null],[460,461,"string"],[461,482,"variableName.special"],[482,497,"string"],[497,498,"bracket"]],
    [],
    [[500,503,"def"],[503,504,"bracket"],[504,521,null],[521,522,null],[522,523,"string"],[523,544,"variableName.special"],[544,551,"string"],[551,552,null],[552,553,"string"],[553,573,"variableName.special"],[573,574,"string"],[574,575,"bracket"]],
    [],
    []
  ]
},
{
  "module": "dockerfile",
  "export": "dockerFile",
  "name": null,
  "source": "# Install Ghost blogging platform and run development environment\n#\n# VERSION 1.0.0\n\nFROM ubuntu:12.10\nMAINTAINER Amer Grgic \"amer@livebyt.es\"\nWORKDIR /data/ghost\n\n# Install dependencies for nginx installation\n# Expose port 2368\nEXPOSE 2368\n# Run Ghost\nCMD [\"npm\",\"start\"]\n",
  "indents": [null,null,null,null,null,null,null,null,null,null,null,null,null,null],
  "autocomplete": null,
  "brackets": null,
  "comments": "#",
  "tokens": [
    [[0,65,"comment"]],
    [[66,67,"comment"]],
    [[68,83,"comment"]],
    [],
    [[85,89,"keyword"],[89,90,null],[90,91,null],[91,92,null],[92,93,null],[93,94,null],[94,95,null],[95,96,null],[96,97,null],[97,98,null],[98,99,null],[99,100,null],[100,101,null],[101,102,null]],
    [[103,113,"keyword"],[113,114,null],[114,125,null],[125,142,"string"]],
    [[143,150,"keyword"],[150,151,null],[151,162,null]],
    [],
    [[164,209,"comment"]],
    [[210,228,"comment"]],
    [[229,235,"keyword"],[235,236,null],[236,240,"number"]],
    [[241,252,"comment"]],
    [[253,256,"keyword"],[256,258,null],[258,263,"string"],[263,264,null],[264,271,"string"],[271,272,null]],
    []
  ]
},
{
  "module": "shell",
  "export": "shell",
  "name": "shell",
  "source": "#!/usr/bin/env bash\n\t#! not the first column\nset -eu\nstage=${1:-build}\nname=\"one $stage ${2:-$(printf '%s' \"nested $(date)\")}\" # input\nprintf '%s\\n' \"$name\" \"$0 $12 $$ $? $\"\nnum=123+4\nfor item in src include; do\n  printf \"%s\\n\" \"$item\"\ndone\ncat <<'HEADER'\ninclude: ${not_expanded}\n\nHEADER\ncat <<- EOF\n  body 🦊\n EOF\nEOF\nprintf '%s' <<< \"line \\\n  $stage and \\\"quotes\\\"\"\nlegacy=$'mixed\\tquoted' \\\n  finish-command --flag-name\n",
  "indents": [null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
  "autocomplete": ["true","false","if","then","do","else","elif","while","until","for","in","esac","fi","fin","fil","done","exit","set","unset","export","function","ab","awk","bash","beep","cat","cc","cd","chown","chmod","chroot","clear","cp","curl","cut","diff","echo","find","gawk","gcc","get","git","grep","hg","kill","killall","ln","ls","make","mkdir","openssl","mv","nc","nl","node","npm","ping","ps","restart","rm","rmdir","sed","service","sh","shopt","shred","source","sort","sleep","ssh","start","stop","su","sudo","svn","tee","telnet","top","touch","vi","vim","wall","wc","wget","who","write","yes","zsh"],
  "brackets": ["(","[","{","'","\"","`"],
  "comments": "#",
  "tokens": [
    [[0,19,"meta"]],
    [[20,21,null],[21,44,"comment"]],
    [[45,48,"keyword"],[48,49,null],[49,52,"attribute"]],
    [[53,58,"def"],[58,59,"operator"],[59,70,"def"]],
    [[71,75,"def"],[75,76,"operator"],[76,81,"string"],[81,87,"def"],[87,88,"string"],[88,93,"def"],[93,102,"quote"],[102,106,"string"],[106,107,"quote"],[107,115,"string"],[115,122,"quote"],[122,123,"string"],[123,124,"quote"],[124,125,"def"],[125,126,"string"],[126,127,null],[127,134,"comment"]],
    [[135,141,null],[141,142,null],[142,148,"string"],[148,149,null],[149,150,"string"],[150,155,"def"],[155,156,"string"],[156,157,null],[157,158,"string"],[158,160,"def"],[160,161,"string"],[161,163,"def"],[163,165,"string"],[165,167,"def"],[167,168,"string"],[168,170,"def"],[170,173,"string"]],
    [[174,177,"def"],[177,178,"operator"],[178,181,"number"],[181,182,"operator"],[182,183,"number"]],
    [[184,187,"keyword"],[187,188,null],[188,192,null],[192,193,null],[193,195,"keyword"],[195,196,null],[196,199,null],[199,200,null],[200,207,null],[207,208,null],[208,209,null],[209,211,"keyword"]],
    [[212,214,null],[214,220,null],[220,221,null],[221,227,"string"],[227,228,null],[228,229,"string"],[229,234,"def"],[234,235,"string"]],
    [[236,240,"keyword"]],
    [[241,244,"builtin"],[244,245,null],[245,255,"string.special"]],
    [[256,280,"string.special"]],
    [],
    [[282,288,"string.special"]],
    [[289,292,"builtin"],[292,293,null],[293,300,"string.special"]],
    [[301,312,"string.special"]],
    [[313,317,"string.special"]],
    [[318,321,"string.special"]],
    [[322,328,null],[328,329,null],[329,333,"string"],[333,334,null],[334,337,"operator"],[337,338,null],[338,345,"string"]],
    [[346,348,"string"],[348,354,"def"],[354,370,"string"]],
    [[371,377,"def"],[377,378,"operator"],[378,394,"string"],[394,395,null],[395,396,null]],
    [[397,399,null],[399,413,null],[413,414,null],[414,420,"attribute"],[420,425,"attribute"]],
    []
  ]
},
{
  "module": "cmake",
  "export": "cmake",
  "name": "cmake",
  "source": "# Update a build stamp\nproject (demo LANGUAGES C CXX)\nset(MAJOR 12)\nconfigure_file(\"${CMAKE_SOURCE_DIR}/version.in\" \"version.hpp\")\nmessage(STATUS \"first \\$literal ${MAJOR}\n\n  second $ENV{BUILD} $not_braced} end\")\nmessage('single $LITERAL\n  still a string')\nmessage(\"escaped slash \\\\$not_var and quote \\\" after\")\nx() xy() xx  ()\ninclude(${-invalid})\nset(FILE 🦊.cmake)\n",
  "indents": [null,null,null,null,null,null,null,null,null,null,null,null,null,null],
  "autocomplete": null,
  "brackets": null,
  "comments": null,
  "tokens": [
    [[0,22,"comment"]],
    [[23,31,"def"],[31,32,"bracket"],[32,36,null],[36,37,null],[37,46,null],[46,47,null],[47,48,null],[48,49,null],[49,52,null],[52,53,"bracket"]],
    [[54,57,"def"],[57,58,"bracket"],[58,63,null],[63,64,null],[64,65,"number"],[65,66,"number"],[66,67,"bracket"]],
    [[68,82,"def"],[82,83,"bracket"],[83,84,"string"],[84,103,"variableName.special"],[103,115,"string"],[115,116,null],[116,129,"string"],[129,130,"bracket"]],
    [[131,138,"def"],[138,139,"bracket"],[139,145,null],[145,146,null],[146,163,"string"],[163,171,"variableName.special"]],
    [],
    [[173,175,null],[175,182,"string"],[182,186,"variableName.special"],[186,194,"string"],[194,206,"variableName.special"],[206,207,null],[207,211,"string"],[211,212,"bracket"]],
    [[213,220,"def"],[220,221,"bracket"],[221,237,"string"]],
    [[238,240,null],[240,255,"string"],[255,256,"bracket"]],
    [[257,264,"def"],[264,265,"bracket"],[265,303,"string"],[303,304,null],[304,309,null],[309,311,"string"]],
    [[312,327,"string"]],
    [[328,336,"string"],[336,337,"variable"],[337,348,"string"]],
    [[349,369,"string"]],
    []
  ]
},
{
  "module": "dockerfile",
  "export": "dockerFile",
  "name": null,
  "source": "﻿# syntax=test\r\n\tFrOm  busybox:1 AS builder\r\nFROM # missing\r\nFROM\r\n\r\n  # continuation of FROM\r\n RUN # missing\r\n  ExPoSe 8080/tcp 53/udp\r\n  RUN printf %s \\\r\n    \"first\\\"second\" tail \\\r\n    'single\\'quote' tail `\r\n    # comment leaves arguments active\r\n    \"last\"\r\nRUN printf \"open\\\r\n  continued\" and done\r\nCMD [\r\n \"printf\", \"unclosed\r\n \"next\"]\r\nENTRYPOINT [\"printf\",\"%s\",\"β\"]\r\nCOPY --from=builder /out /\r\nRUN\r\nRUN # no args\r\nFROMÉ\r\nFROM-asset\r\nrunK command\r\nRUN echo 🦊\r\n",
  "indents": [null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
  "autocomplete": null,
  "brackets": null,
  "comments": "#",
  "tokens": [
    [[0,16,"comment"]],
    [[18,19,null],[19,23,"keyword"],[23,35,null],[35,37,"keyword"],[37,38,null],[38,39,null],[39,40,null],[40,41,null],[41,42,null],[42,43,null],[43,44,null],[44,45,null]],
    [[47,51,"keyword"],[51,52,null],[52,61,"error"]],
    [[63,67,"keyword"]],
    [],
    [[71,73,null],[73,95,"error"]],
    [[97,98,null],[98,101,"keyword"],[101,102,null],[102,111,"error"]],
    [[113,115,null],[115,121,"keyword"],[121,122,null],[122,126,"number"],[126,131,null],[131,133,"number"],[133,137,null]],
    [[139,141,null],[141,144,"keyword"],[144,145,null],[145,156,null]],
    [[158,162,null],[162,163,"string"],[163,164,"string"],[164,165,"string"],[165,166,"string"],[166,167,"string"],[167,168,"string"],[168,170,"string"],[170,171,"string"],[171,172,"string"],[172,173,"string"],[173,174,"string"],[174,175,"string"],[175,176,"string"],[176,177,"string"],[177,184,null]],
    [[186,190,null],[190,191,"string"],[191,192,"string"],[192,193,"string"],[193,194,"string"],[194,195,"string"],[195,196,"string"],[196,197,"string"],[197,199,"string"],[199,200,"string"],[200,201,"string"],[201,202,"string"],[202,203,"string"],[203,204,"string"],[204,205,"string"],[205,212,null]],
    [[214,251,"comment"]],
    [[253,257,null],[257,263,"string"]],
    [[265,268,"keyword"],[268,269,null],[269,276,null],[276,277,"string"],[277,278,"string"],[278,279,"string"],[279,280,"string"],[280,281,"string"],[281,282,null]],
    [[284,285,"string"],[285,286,"string"],[286,287,"string"],[287,288,"string"],[288,289,"string"],[289,290,"string"],[290,291,"string"],[291,292,"string"],[292,293,"string"],[293,294,"string"],[294,295,"string"],[295,296,"string"],[296,305,null]],
    [[307,310,"keyword"],[310,312,null]],
    [[314,315,null],[315,323,"string"],[323,324,null],[324,325,null],[325,334,"string"]],
    [[336,337,null],[337,343,"string"],[343,344,null]],
    [[346,356,"keyword"],[356,358,null],[358,366,"string"],[366,367,null],[367,371,"string"],[371,372,null],[372,376,"string"],[376,377,null]],
    [[379,383,"keyword"],[383,384,null],[384,405,null]],
    [[407,410,"keyword"]],
    [[412,415,"keyword"],[415,416,null],[416,425,"error"]],
    [[427,431,"keyword"],[431,433,null]],
    [[435,439,"keyword"],[439,440,null],[440,441,null],[441,442,null],[442,443,null],[443,444,null],[444,445,null]],
    [[447,448,null],[448,449,null],[449,450,null],[450,453,null],[453,454,null],[454,455,null],[455,456,null],[456,457,null],[457,458,null],[458,459,null],[459,460,null],[460,461,null]],
    [[463,466,"keyword"],[466,467,null],[467,476,null]],
    []
  ]
}
]
"###;
