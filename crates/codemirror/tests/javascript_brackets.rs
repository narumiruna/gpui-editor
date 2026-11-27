use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        brackets::{BracketConfig, BracketMatch, bracket_near_in},
        html::{HtmlDocument, HtmlEmbeddedDocument},
        javascript::{JavascriptDocument, JavascriptOptions},
    },
};

type Hit = ([usize; 2], Option<[usize; 2]>, bool);

fn hit(hit: Hit) -> BracketMatch {
    let (start, end, matched) = hit;
    let range = |[from, to]: [usize; 2]| from..to;
    BracketMatch {
        start: range(start),
        end: end.map(range),
        matched,
    }
}

#[test]
fn app_fallback_confuses_literal_brackets_with_call_delimiters() {
    let prefix = "字\r\n\r\n```js\r\n";
    let suffix = "\r\n```\r\n)";
    for body in [r#"log(")");"#, "log(`)`);", "log(/* ) */ 1);"] {
        let text = Text::new(format!("{prefix}{body}{suffix}"));
        let scope = prefix.len()..prefix.len() + body.len();
        let open = scope.start + body.find('(').unwrap();
        let literal = scope.start + body.find(')').unwrap();
        let call_close = scope.start + body.rfind(')').unwrap();
        assert_ne!(literal, call_close);
        assert_eq!(
            bracket_near_in(&text, scope, open + 1, BracketConfig::default(), |_, _| {
                true
            }),
            Some(BracketMatch {
                start: open..open + 1,
                end: Some(literal..literal + 1),
                matched: true,
            }),
            "{body}"
        );
        let doc = JavascriptDocument::parse(Text::new(body));
        let from = open - prefix.len();
        let to = call_close - prefix.len();
        assert_eq!(
            doc.bracket_near(from + 1),
            Some(hit(([from, from + 1], Some([to, to + 1]), true)))
        );
    }
}

#[test]
fn default_brackets_match_the_pinned_plugin() {
    for row in include_str!("fixtures/javascript_brackets.jsonl").lines() {
        let case: serde_json::Value = serde_json::from_str(row).unwrap();
        let repeat: Option<(String, usize)> =
            serde_json::from_value(case["repeat"].clone()).unwrap();
        let source = case["source"].as_str().unwrap();
        let source = match repeat {
            Some((unit, count)) => source.replacen('$', &unit.repeat(count), 1),
            None => source.to_owned(),
        };
        let mode = case["mode"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let checks: Vec<(usize, Option<Hit>)> =
            serde_json::from_value(case["checks"].clone()).unwrap();
        let doc = JavascriptDocument::parse_with_options(
            Text::new(&source),
            JavascriptOptions {
                typescript: mode.starts_with("ts"),
                jsx: mode.ends_with('x'),
            },
        );
        for (head, expected) in checks {
            assert_eq!(
                doc.bracket_near(head),
                expected.map(hit),
                "{mode}: {name} at byte {head}; tree {:?}",
                doc.nodes()
                    .map(|n| (n.kind().name(), n.range()))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                doc.bracket_near_with_config(head, BracketConfig::default()),
                expected.map(hit),
                "{mode}: {name} at byte {head} with explicit defaults"
            );
        }
    }
}

#[test]
fn configured_brackets_match_the_pinned_plugin() {
    for row in include_str!("fixtures/javascript_bracket_options.jsonl").lines() {
        let case: serde_json::Value = serde_json::from_str(row).unwrap();
        let mode = case["mode"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let doc = JavascriptDocument::parse_with_options(
            Text::new(case["source"].as_str().unwrap()),
            JavascriptOptions {
                typescript: mode.starts_with("ts"),
                jsx: mode.ends_with('x'),
            },
        );
        let defaults = BracketConfig::default();
        let options = &case["options"];
        let config = BracketConfig {
            brackets: options["brackets"].as_str().unwrap_or(defaults.brackets),
            max_scan_distance: serde_json::from_value::<Option<usize>>(
                options["maxScanDistance"].clone(),
            )
            .unwrap()
            .unwrap_or(defaults.max_scan_distance),
            after_cursor: options["afterCursor"]
                .as_bool()
                .unwrap_or(defaults.after_cursor),
        };
        let checks: Vec<(usize, Option<Hit>)> =
            serde_json::from_value(case["checks"].clone()).unwrap();
        for (head, expected) in checks {
            assert_eq!(
                doc.bracket_near_with_config(head, config),
                expected.map(hit),
                "{mode}: {name} at byte {head}; tree {:?}",
                doc.nodes()
                    .map(|n| (n.kind().name(), n.range()))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn checks_the_delimiter_on_the_left_before_the_one_on_the_right() {
    let doc = JavascriptDocument::parse(Text::new("`${[0]}`"));
    for head in [1, 2, 3] {
        assert_eq!(
            doc.bracket_near(head),
            Some(hit(([1, 3], Some([6, 7]), true)))
        );
    }
    assert_eq!(doc.bracket_near(6), Some(hit(([5, 6], Some([3, 4]), true))));
    assert_eq!(doc.bracket_near(7), Some(hit(([6, 7], Some([1, 3]), true))));

    let doc = JavascriptDocument::parse_with_options(
        Text::new("<A></B>"),
        JavascriptOptions {
            jsx: true,
            ..JavascriptOptions::default()
        },
    );
    // An unmatched `>` still wins over the matched `</` on the right.
    assert_eq!(doc.bracket_near(3), Some(hit(([2, 3], None, false))));
    // This is punctuation matching, even when A and B are different names.
    assert_eq!(doc.bracket_near(4), Some(hit(([3, 5], Some([6, 7]), true))));
}

#[test]
fn caches_original_byte_projection_only_for_the_current_document() {
    let source = "// 😀\r\nlog(\"\\\r\n)\");";
    let doc = JavascriptDocument::parse(Text::new(source));
    let open = source.find('(').unwrap();
    let close = source.rfind(')').unwrap();
    assert_eq!(
        doc.bracket_near(open),
        Some(hit(([open, open + 1], Some([close, close + 1]), true)))
    );
    for head in [4, 5, 6, 8, source.len() + 1, usize::MAX] {
        assert_eq!(doc.bracket_near(head), None, "at {head}");
        assert_eq!(
            doc.bracket_near_with_config(head, BracketConfig::default()),
            None,
            "at {head} with explicit defaults"
        );
    }
    let edited = doc
        .reparse(&ChangeSet::single(Change::new(close..close + 1, "")).unwrap())
        .unwrap();
    assert_eq!(
        edited.bracket_near(open),
        Some(hit(([open, open + 1], None, false)))
    );
    assert!(doc.bracket_near(open).unwrap().matched);
}

#[test]
fn mounted_javascript_matches_only_its_own_text() {
    let doc = HtmlDocument::parse(Text::new("😀<script>log(\")\"</script>)"));
    let mount = &doc.mounts()[0];
    let HtmlEmbeddedDocument::Javascript(script) = mount.document() else {
        panic!("expected a JavaScript mount");
    };
    assert_eq!(doc.text().slice(mount.range()), Some("log(\")\""));
    assert_eq!(script.bracket_near(4), Some(hit(([3, 4], None, false))));
    assert_eq!(script.bracket_near(script.text().len() + 1), None);
}

#[test]
fn mounted_interpolation_reports_ranges_in_original_script_bytes() {
    let source = "😀<script>`😀\r\n${\rx}`</script>}";
    let doc = HtmlDocument::parse(Text::new(source));
    let mount = &doc.mounts()[0];
    let HtmlEmbeddedDocument::Javascript(script) = mount.document() else {
        panic!("expected a JavaScript mount");
    };
    assert_eq!(doc.text().slice(mount.range()), Some("`😀\r\n${\rx}`"));
    assert_eq!(
        script.bracket_near(8),
        Some(hit(([7, 9], Some([11, 12]), true)))
    );
    assert_eq!(
        script.bracket_near(12),
        Some(hit(([11, 12], Some([7, 9]), true)))
    );
}
