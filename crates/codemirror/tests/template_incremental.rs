//! Pinned npm tarballs verified against `docs/codemirror-upstream.json`:
//! `@codemirror/lang-jinja` 6.0.1, `@codemirror/lang-liquid` 6.3.4,
//! `@codemirror/lang-php` 6.0.2, and `@lezer/php` 1.0.6.
//! In external scratch, fresh and TreeFragment parses agreed. The fixture
//! maps logical UTF-16 offsets back to original UTF-8 bytes.

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::{
        IndentOptions, StreamSettings,
        html::{
            HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlNodeKind, HtmlOptions,
        },
        jinja::{JinjaDocument, JinjaOptions},
        lezer::ReuseStats,
        liquid::{LiquidDocument, LiquidOptions},
        php::{PhpDocument, PhpOptions},
    },
};
use serde_json::{Value, json};

macro_rules! records {
    ($doc:expr, $from:expr, $node:ident => $name:expr) => {{
        let text = $doc.text();
        let from = $from;
        $doc.nodes()
            .map(|$node| {
                let range = $node.range();
                assert!(text.slice(range.clone()).is_some());
                let depth = std::iter::successors($node.parent(), |node| node.parent()).count();
                ($name, range, depth)
            })
            .filter(|(_, range, _)| range.start >= from)
            .map(|(name, range, depth)| json!([name, range.start, range.end, depth]))
            .collect::<Value>()
    }};
}

macro_rules! assert_fresh {
    ($edited:expr, $fresh:expr, $node:ident => $name:expr) => {{
        let (edited, fresh) = (&$edited, &$fresh);
        assert_eq!(edited.text(), fresh.text());
        assert_eq!(records!(edited, 0, $node => $name), records!(fresh, 0, $node => $name));
        let range = 0..edited.text().len();
        assert!(edited.highlights(range.clone()).unwrap().eq(fresh.highlights(range).unwrap()));
        for number in 1..=edited.text().line_count() {
            let line = edited.text().line(number).unwrap().range;
            assert_eq!(edited.foldable(line.start, line.end), fresh.foldable(line.start, line.end));
            assert_eq!(
                edited.indentation(line.start, StreamSettings::default(), IndentOptions::default()),
                fresh.indentation(line.start, StreamSettings::default(), IndentOptions::default()),
            );
        }
    }};
}

fn edit(from: usize, to: usize, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(from..to, insert)).unwrap()
}

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn fixtures() -> impl Iterator<Item = Value> {
    include_str!("fixtures/template_incremental.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn assert_html(
    html: &HtmlDocument,
    ranges: impl Iterator<Item = std::ops::Range<usize>>,
    source: &str,
    oracle: &Value,
) {
    let projected = html.text().as_str();
    assert_eq!(projected, oracle["text"].as_str().unwrap());
    let ranges: Vec<_> = ranges.collect();
    assert_eq!(
        ranges
            .iter()
            .map(|range| &source[range.clone()])
            .collect::<String>(),
        projected
    );
    assert_eq!(
        ranges
            .iter()
            .map(|range| json!([range.start, range.end]))
            .collect::<Value>(),
        oracle["ranges"]
    );
    let mounts = html.mounts().iter().map(|mount| {
        let range = mount.range();
        let HtmlEmbeddedDocument::Javascript(document) = mount.document() else {
            panic!("expected a script or JSON mount");
        };
        assert_eq!(projected.get(range.clone()), Some(document.text().as_str()));
        assert!(!document.is_recovered());
        let name = match mount.language() {
            HtmlEmbeddedLanguage::JavaScript => "Script",
            HtmlEmbeddedLanguage::Json => "SingleExpression",
            other => panic!("unexpected mount {other:?}"),
        };
        json!([name, range.start, range.end])
    });
    assert_eq!(mounts.collect::<Value>(), oracle["mounts"]);
    if let Some(start) = projected.find("<box/>") {
        assert!(html.nodes().any(|node| {
            node.kind() == HtmlNodeKind::SelfClosingTag && node.range() == (start..start + 6)
        }));
    }
}

macro_rules! exercise {
    ($case:expr, $parse:expr, $node:ident => $name:expr, $assert_projection:expr) => {{
        let (case, parse, assert_projection) = (&$case, $parse, $assert_projection);
        let mode = case["mode"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let from = offset(&case["from"]);
        let seed = parse(Text::new(source));
        assert_eq!(seed.reuse_stats(), ReuseStats::default());
        assert_eq!(records!(seed, from, $node => $name), case["before"], "{mode}");
        let edits = case["edits"].as_array().unwrap();
        let changes = ChangeSet::new(edits.iter().map(|entry| {
            Change::new(offset(&entry[0])..offset(&entry[1]), entry[2].as_str().unwrap())
        }).collect()).unwrap();
        let updated = seed.reparse(&changes).unwrap();
        assert_eq!(updated.text().as_str(), case["updated"].as_str().unwrap());
        assert_eq!(records!(updated, from, $node => $name), case["after"], "{mode}");
        assert_fresh!(updated, parse(updated.text().clone()), $node => $name);
        assert_projection(&seed, &case["oldHtml"]);
        assert_projection(&updated, &case["newHtml"]);
        assert_eq!(seed.text().as_str(), source);
        let stats = updated.reuse_stats();
        let prefix = &source[..offset(&edits[0][0])];
        let limit = prefix.len() - prefix.matches("\r\n").count();
        assert!(stats.nodes_reused > 0 && (1..=limit).contains(&stats.resumed_at), "{mode}: {stats:?}");
        let logical = source.replace("\r\n", "\n");
        if let Some(at) = logical.find("$value[1]") {
            // Resume inside the quoted string, before the stack-dependent subscript scan.
            assert_eq!(stats.resumed_at, at, "{mode}: {stats:?}");
        } else {
            // Resume inside the raw block, before its closing tag can switch tokenizers.
            assert!(stats.resumed_at > logical.find("characters\n").unwrap(), "{mode}: {stats:?}");
        }

        // Use a split inside an original CRLF to exercise prefix mapping at the edit boundary.
        let source = updated.text().as_str();
        let split_at = source.rfind("\r\n").unwrap() + 1;
        let split = updated.reparse(&edit(split_at, split_at, " ")).unwrap();
        assert_fresh!(split, parse(split.text().clone()), $node => $name);
        let joined = split.reparse(&edit(split_at, split_at + 1, "")).unwrap();
        assert_fresh!(joined, updated, $node => $name);

        let inside_scalar = source.char_indices().find(|(_, ch)| ch.len_utf8() == 4).unwrap().0 + 1;
        assert!(matches!(updated.reparse(&edit(inside_scalar, inside_scalar, "x")), Err(EditError::InvalidRange)));
        assert!(matches!(updated.reparse(&edit(source.len(), source.len() + 1, "")), Err(EditError::InvalidRange)));
        assert_eq!(updated.text().as_str(), source);
    }};
}

#[test]
fn stateful_reparses_match_pinned_trees_and_rebuild_html_mounts() {
    let html = HtmlOptions {
        self_closing_tags: true,
        ..HtmlOptions::default()
    };
    for case in fixtures() {
        match case["mode"].as_str().unwrap() {
            "jinja" => {
                exercise!(case, |text| JinjaDocument::parse_with_options(text, JinjaOptions { html: Some(html) }), node => node.name(), |doc: &JinjaDocument, oracle| {
                    let mount = doc.html().unwrap();
                    assert_html(mount.document(), mount.source_ranges(0..mount.document().text().len()).unwrap().into_iter(), doc.text().as_str(), oracle);
                })
            }
            "liquid" => {
                exercise!(case, |text| LiquidDocument::parse_with_options(text, LiquidOptions { html }), node => node.kind().name(), |doc: &LiquidDocument, oracle| {
                    let mount = doc.html().unwrap();
                    assert_html(mount.document(), mount.source_ranges(0..mount.document().text().len()).unwrap(), doc.text().as_str(), oracle);
                })
            }
            "php" => {
                exercise!(case, PhpDocument::parse, node => node.name(), |doc: &PhpDocument, oracle| {
                    let mount = doc.html().unwrap();
                    assert_html(mount.document(), mount.source_ranges(0..mount.document().text().len()).unwrap().into_iter(), doc.text().as_str(), oracle);
                })
            }
            "php-plain" => {
                exercise!(case, |text| PhpDocument::parse_with_options(text, PhpOptions { plain: true, html: true }), node => node.name(), |doc: &PhpDocument, _: &Value| {
                    assert_eq!(doc.root().name(), "Program");
                    assert!(doc.html().is_none());
                })
            }
            mode => panic!("unexpected fixture mode {mode}"),
        }
    }
}

#[test]
fn preserves_disabled_projections_and_can_build_new_projections() {
    let source = Text::new("{{ name }}");
    let changes = edit(
        source.len(),
        source.len(),
        "<script>const x = '🦀';</script>",
    );
    let jinja = JinjaDocument::parse_with_options(source.clone(), JinjaOptions { html: None });
    assert!(jinja.reparse(&changes).unwrap().html().is_none());
    let jinja = JinjaDocument::parse(source.clone());
    assert!(jinja.html().is_none());
    let liquid = LiquidDocument::parse(source);
    assert!(liquid.html().is_none());
    assert_eq!(
        jinja
            .reparse(&changes)
            .unwrap()
            .html()
            .unwrap()
            .document()
            .mounts()
            .len(),
        1
    );
    assert_eq!(
        liquid
            .reparse(&changes)
            .unwrap()
            .html()
            .unwrap()
            .document()
            .mounts()
            .len(),
        1
    );

    let source = Text::new("<?php echo 'é';");
    let changes = edit(
        source.len(),
        source.len(),
        " ?><script>const x = '🦀';</script>",
    );
    let php = PhpDocument::parse_with_options(
        source.clone(),
        PhpOptions {
            plain: false,
            html: false,
        },
    );
    let updated = php.reparse(&changes).unwrap();
    assert_eq!(updated.root().name(), "Template");
    assert!(updated.html().is_none());
    let php = PhpDocument::parse(source);
    assert!(php.html().is_none());
    assert_eq!(
        php.reparse(&changes)
            .unwrap()
            .html()
            .unwrap()
            .document()
            .mounts()
            .len(),
        1
    );
}

#[test]
fn recovery_discards_checkpoints() {
    macro_rules! check {
        ($parse:expr, $source:expr, $bad:literal, $node:ident => $name:expr) => {{
            let parse = $parse;
            let seed = parse(Text::new($source));
            let no_changes = ChangeSet::default();
            assert!(seed.reparse(&no_changes).unwrap().reuse_stats().nodes_reused > 0);
            let end = seed.text().len();
            let changes = edit(end, end, $bad);
            let broken = seed.reparse(&changes).unwrap();
            assert!(broken.nodes().any(|$node| $name == "⚠"));
            assert_eq!(broken.reuse_stats(), ReuseStats::default());
            assert_fresh!(broken, parse(broken.text().clone()), $node => $name);
            let repaired = broken.reparse(&changes.inverted(seed.text()).unwrap()).unwrap();
            assert_eq!(repaired.reuse_stats(), ReuseStats::default());
            assert_fresh!(repaired, seed, $node => $name);
            assert!(repaired.reparse(&no_changes).unwrap().reuse_stats().nodes_reused > 0);
        }};
    }
    let prefix = "<p>😀</p>\r\n".repeat(18);
    let source = format!("{prefix}{{{{ name }}}}");
    check!(JinjaDocument::parse, &source, "{{", node => node.name());
    check!(LiquidDocument::parse, &source, "{{", node => node.kind().name());
    let source = "echo '😀';\r\n".repeat(18);
    check!(|text| PhpDocument::parse_with_options(text, PhpOptions { plain: true, html: true }), &source, "$x = [;", node => node.name());
}

#[test]
fn liquid_glr_fallback_discards_checkpoints() {
    let mut source = "<p>😀</p>\r\n".repeat(18);
    let seed = LiquidDocument::parse(Text::new(&source));
    let at = source.len();
    let tail = "{{ forloop.index }}";
    let next = seed.reparse(&edit(at, at, tail)).unwrap();
    source.push_str(tail);
    assert_fresh!(next, LiquidDocument::parse(Text::new(source)), node => node.kind().name());
    assert!(next.nodes().all(|node| node.kind().name() != "⚠"));
    assert!(next.nodes().any(|node| node.kind().name() == "forloop"));
    assert_eq!(next.reuse_stats(), ReuseStats::default());
    let restored = next.reparse(&edit(at, next.text().len(), "")).unwrap();
    assert_eq!(restored.reuse_stats(), ReuseStats::default());
    assert_fresh!(restored, seed, node => node.kind().name());
}
