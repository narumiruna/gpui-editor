//! Oracle: unchanged ESM exports from `@codemirror/legacy-modes` 6.5.5 and
//! `StringStream` from `@codemirror/streamparser` 6.0.0. Both npm tarballs
//! match the SRI digests in `docs/codemirror-upstream.json`. The scratch runner
//! used indent unit 2 and tab size 4. It recorded raw styles for advancing
//! tokens, mapped offsets to UTF-8 bytes, and clamped negative indents to zero.
//! End-state probes also use indent unit 3 with the same state. Tests run Rust only.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::StreamSettings,
    legacy_modes::LegacyMode,
    stream_parser::{StreamParser, StringStream, Tokenizer},
};
use serde_json::{Value, json};

const PREFIX: &str = "@codemirror/legacy-modes/mode/";
const ORACLE: &str = include_str!("fixtures/legacy_script_modes.json");

fn mode(module: &str, export: &str) -> Option<LegacyMode> {
    LegacyMode::from_export(&format!("{PREFIX}{module}"), export)
}

#[test]
fn matches_pinned_script_modes() {
    let oracle: Value = serde_json::from_str(ORACLE).unwrap();
    let settings = StreamSettings::default();
    for (export, expected) in oracle["modes"].as_object().unwrap() {
        let module = expected["module"].as_str().unwrap();
        let mode = mode(module, export).unwrap();
        assert_eq!(json!(mode.name()), expected["name"]);
        assert!(mode.merge_tokens());
        let lang = mode.language();
        assert_eq!(lang.name(), mode.name().unwrap());
        for case in expected["cases"].as_array().unwrap() {
            let title = case["title"].as_str().unwrap();
            let source = case["source"].as_str().unwrap();
            let mut tokens = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|line| line.as_array().unwrap());
            let mut tokenizer = Tokenizer::new(&mode, source, settings.tab_size, 2);
            for token in tokenizer.by_ref() {
                let token = token.unwrap_or_else(|err| panic!("{export}/{title}: {err}"));
                assert_eq!(token.text, &source[token.range.clone()]);
                assert_eq!(
                    &json!([token.range.start, token.range.end, token.style]),
                    tokens.next().expect("extra token"),
                    "{export}/{title}: {:?} at {:?}",
                    token.text,
                    token.range,
                );
            }
            assert!(tokens.next().is_none(), "{export}/{title}: missing tokens");

            let doc = lang.parse(Text::new(source), settings).unwrap();
            let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1));
            let indents = starts
                .map(|pos| doc.indentation(pos, Default::default()).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                json!(indents),
                case["indents"],
                "{export}/{title}: line indentation"
            );
            let end_indents = oracle["afterInputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    [2, 3].map(|unit| mode.indent(tokenizer.state(), input.as_str().unwrap(), unit))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(end_indents),
                case["lastIndents"],
                "{export}/{title}: final indentation"
            );

            let data = doc.language_data_at(0).unwrap();
            let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
            assert_eq!(json!(comments.line), expected["commentTokens"]["line"]);
            assert_eq!(
                json!(
                    comments
                        .block
                        .map(|(open, close)| json!({"open": open, "close": close}))
                ),
                expected["commentTokens"]["block"],
                "{export}: block comments",
            );
            for key in ["autocomplete", "closeBrackets"] {
                assert_eq!(
                    json!(data.get::<&[&str]>(key)),
                    expected[key],
                    "{export}: {key}"
                );
            }
            let triggers = data
                .get::<fn(&str) -> bool>("indentOnInput")
                .map(|trigger| {
                    oracle["triggerInputs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|input| trigger(input.as_str().unwrap()))
                        .collect::<Vec<_>>()
                });
            assert_eq!(
                json!(triggers),
                expected["triggers"],
                "{export}: indentOnInput"
            );
        }
    }
}

#[test]
fn registers_the_exact_script_exports() {
    for (module, export, expected) in [
        ("python", "python", LegacyMode::Python),
        ("python", "cython", LegacyMode::Cython),
        ("ruby", "ruby", LegacyMode::Ruby),
        ("groovy", "groovy", LegacyMode::Groovy),
        ("powershell", "powerShell", LegacyMode::PowerShell),
    ] {
        assert_eq!(mode(module, export), Some(expected));
        assert_eq!(mode(&format!("{module}.js"), export), None);
        assert_eq!(mode(module, &export.to_uppercase()), None);
    }
    assert_eq!(mode("python", "mkPython"), None);
    assert_eq!(mode("cython", "cython"), None);
    assert_eq!(mode("powershell", "powershell"), None);
}

#[test]
fn copies_open_script_state_without_sharing_mutations() {
    let settings = StreamSettings::default();
    for (mode, source, closing, probe, open_styles, closed_styles) in [
        (
            LegacyMode::Python,
            "buf = f\"\"\"before {",
            "key} after\"\"\"",
            "key} tail",
            &[Some("variable"), None, Some("string")][..],
            &[Some("variable"), Some("error"), None, Some("variable")][..],
        ),
        (
            LegacyMode::Ruby,
            "doc = <<-TAG",
            "TAG",
            "branch",
            &[Some("string")],
            &[Some("variable")],
        ),
        (
            LegacyMode::Groovy,
            "buf = \"\"\"before ${",
            "key} after\"\"\"",
            "} tail",
            &[Some("string")],
            &[None, None, Some("variable")],
        ),
        (
            LegacyMode::PowerShell,
            "@\"\n$(",
            ") after\n\"@",
            ") tail",
            &[Some("punctuation"), Some("string")],
            &[Some("punctuation"), None, Some("variable")],
        ),
    ] {
        let mut tokenizer = Tokenizer::new(&mode, source, settings.tab_size, 2);
        for token in tokenizer.by_ref() {
            token.unwrap();
        }
        let mut closed = mode.copy_state(tokenizer.state());
        let mut open = mode.copy_state(tokenizer.state());
        let scan = |line, state: &mut _| {
            let mut stream = StringStream::new(line, settings.tab_size, 2);
            let mut styles = Vec::new();
            while !stream.eol() {
                stream.start_token();
                for _ in 0..10 {
                    let style = mode.token(&mut stream, state);
                    if stream.pos() > stream.start() {
                        styles.push(style);
                        break;
                    }
                }
                assert!(stream.pos() > stream.start(), "{mode:?}");
            }
            styles
        };
        for line in closing.lines() {
            scan(line, &mut closed);
        }
        for (branch, state, expected) in [
            ("open", &mut open, open_styles),
            ("closed", &mut closed, closed_styles),
        ] {
            let styles = scan(probe, state);
            let actual: Vec<_> = styles.iter().map(Option::as_deref).collect();
            assert_eq!(actual, expected, "{mode:?} {branch} branch");
        }
    }
}

#[test]
fn replays_edits_from_open_script_state() {
    let settings = StreamSettings::default();
    for (mode, source, old, new) in [
        (
            LegacyMode::Python,
            "def f():\n  msg = f\"\"\"before {\n  n} after\"\"\"\n  return msg\n",
            "n}",
            "compute({\"a\": n})}",
        ),
        (
            LegacyMode::Cython,
            "cdef int f():\n  return helper(1,\n                2)\n",
            "2)",
            "3,\n                4)",
        ),
        (
            LegacyMode::Ruby,
            "text = <<~TAG\n  old\nTAG\nnext\n",
            "old",
            "new\n  second",
        ),
        (
            LegacyMode::Groovy,
            "def msg = \"\"\"before ${\nvalue} after\"\"\"\nprint msg\n",
            "value}",
            "f([key: value])}",
        ),
        (
            LegacyMode::PowerShell,
            "@\"\n$(Get-Item\n  name) tail\n\"@\nGet-Help\n",
            "name)",
            "(name))",
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
        for start in std::iter::once(0).chain(updated.match_indices('\n').map(|(i, _)| i + 1)) {
            assert_eq!(
                replay.indentation(start, Default::default()),
                fresh.indentation(start, Default::default()),
                "{mode:?} at {start}",
            );
        }
    }
}
