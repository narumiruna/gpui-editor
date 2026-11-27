//! Fresh and fragment trees from SHA-512-verified npm tarballs: `@lezer/html`
//! 1.3.13, `@codemirror/lang-vue` 0.1.3 and `@codemirror/lang-angular` 0.1.4,
//! with the pinned HTML/JavaScript providers and Lezer runtime. The external
//! oracle converts UTF-16 positions to UTF-8. Rust tests do not run JavaScript.

use std::{cell::RefCell, ops::Range};

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings,
        angular::AngularDocument,
        html::{
            HtmlCustomDocument, HtmlDocument, HtmlEmbeddedDocument, HtmlNestedLanguage,
            HtmlNestedParser, HtmlNesting, HtmlNodeKind, HtmlOptions,
        },
        javascript::JavascriptDocument,
        lezer::ReuseStats,
        vue::VueDocument,
    },
};
use serde_json::{Value, json};

fn edit(range: Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

fn read_string<'a>(fixture: &'a Value, name: &str) -> &'a str {
    fixture[name].as_str().unwrap()
}

fn read_offset(fixture: &Value, name: &str) -> usize {
    fixture[name].as_u64().unwrap().try_into().unwrap()
}

fn record(name: &str, range: Range<usize>, depth: usize) -> String {
    format!("{name} {} {} {depth}", range.start, range.end)
}

fn html_records(doc: &HtmlDocument, from: usize) -> Vec<String> {
    doc.nodes()
        .filter(|node| node.range().start >= from)
        .map(|node| {
            let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
            record(node.kind().name(), node.range(), depth)
        })
        .collect()
}

fn js_records(doc: &JavascriptDocument, offset: usize) -> Vec<String> {
    doc.nodes()
        .map(|node| {
            let range = node.range();
            let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
            record(
                node.kind().name(),
                offset + range.start..offset + range.end,
                depth,
            )
        })
        .collect()
}

fn assert_html_fresh(doc: &HtmlDocument, fresh: &HtmlDocument) {
    let source = doc.text().as_str();
    assert_eq!(source, fresh.text().as_str());
    assert_eq!(doc.root().range(), 0..source.len());
    assert_eq!(html_records(doc, 0), html_records(fresh, 0));
    assert!(doc.nodes().all(|node| source.get(node.range()).is_some()));
    assert_eq!(doc.mounts().len(), fresh.mounts().len());
    for (mount, other) in doc.mounts().iter().zip(fresh.mounts()) {
        let range = mount.range();
        assert_eq!(range, other.range());
        assert_eq!(mount.language(), other.language());
        match (mount.document(), other.document()) {
            (HtmlEmbeddedDocument::Javascript(js), HtmlEmbeddedDocument::Javascript(other)) => {
                assert_eq!(js.text().as_str(), &source[range]);
                assert_eq!(js_records(js, 0), js_records(other, 0));
                assert_eq!(js.is_recovered(), other.is_recovered());
                assert_eq!(js.reuse_stats(), ReuseStats::default());
            }
            (HtmlEmbeddedDocument::Css(css), HtmlEmbeddedDocument::Css(other)) => {
                assert_eq!(css.text().as_str(), &source[range]);
                assert!(
                    css.nodes()
                        .map(|node| (node.kind(), node.range()))
                        .eq(other.nodes().map(|node| (node.kind(), node.range())))
                );
            }
            _ => panic!("expected matching stock mounts"),
        }
    }
    assert!(
        doc.highlights(0..source.len())
            .unwrap()
            .eq(fresh.highlights(0..source.len()).unwrap())
    );
    for index in 1..=doc.text().line_count() {
        let line = doc.text().line(index).unwrap().range;
        assert_eq!(
            doc.foldable(line.start, line.end),
            fresh.foldable(line.start, line.end)
        );
        let settings = StreamSettings::default();
        let options = IndentOptions::default();
        assert_eq!(
            doc.indentation(line.start, settings, options),
            fresh.indentation(line.start, settings, options)
        );
    }
}

fn assert_script(
    fixture: &Value,
    range: Range<usize>,
    doc: &JavascriptDocument,
    fresh: &JavascriptDocument,
    source: &str,
) {
    assert_eq!(json!([range.start, range.end]), fixture["range"]);
    assert_eq!(doc.text().as_str(), &source[range.clone()]);
    assert_eq!(doc.text().as_str(), read_string(fixture, "source"));
    assert_eq!(doc.text().as_str(), fresh.text().as_str());
    assert_eq!(doc.is_recovered(), fresh.is_recovered());
    assert_eq!(doc.reuse_stats(), ReuseStats::default());
    let records = js_records(doc, range.start);
    assert_eq!(records, js_records(fresh, range.start));
    assert_eq!(json!(records), fixture["tree"]);
}

fn assert_reused(stats: ReuseStats, at: usize) {
    assert!(
        (1..=at).contains(&stats.resumed_at),
        "{stats:?} before {at}"
    );
    assert!(stats.nodes_reused > 0, "{stats:?}");
}

#[test]
fn reuses_strict_html_checkpoints_and_rebuilds_vue_and_angular_mounts() {
    for line in include_str!("fixtures/html_checkpoint_reuse.jsonl").lines() {
        let fixture: Value = serde_json::from_str(line).unwrap();
        let source = read_string(&fixture, "source");
        let mode = read_string(&fixture, "mode");
        let dialect = read_string(&fixture, "dialect");
        let options = HtmlOptions {
            match_closing_tags: !dialect.contains("noMatch"),
            self_closing_tags: dialect.contains("selfClosing"),
        };
        let parse = |text| HtmlDocument::parse_with_options(text, options);
        let at = read_offset(&fixture, "at");
        let remove = read_string(&fixture, "remove");
        let remove_range = at..at + remove.len();
        assert_eq!(&source[remove_range.clone()], remove);
        let insert = read_string(&fixture, "insert");
        let changes = edit(remove_range.clone(), insert);
        let mut updated = source.to_owned();
        updated.replace_range(remove_range, insert);
        let from = read_offset(&fixture, "from");

        let old = parse(Text::new(source));
        let html = old.reparse(&changes).unwrap();
        let fresh = parse(Text::new(&updated));
        assert_eq!(old.reuse_stats(), ReuseStats::default());
        assert_reused(html.reuse_stats(), at);
        assert_html_fresh(&html, &fresh);
        assert_eq!(
            json!(html_records(&html, from)),
            fixture["oracle"],
            "{mode} {dialect}"
        );
        if let Some(svg) = source.find("<svg>") {
            assert!(html.reuse_stats().resumed_at > svg + "<svg>".len());
        }
        if let Some(script) = source.find("<script>") {
            assert!(html.reuse_stats().resumed_at > script + "<script>".len());
        }

        let oracle = fixture["scripts"].as_array().unwrap();
        let all = 0..updated.len();
        match mode {
            "vue" => {
                let doc = VueDocument::parse(Text::new(source))
                    .reparse(&changes)
                    .unwrap();
                let full = VueDocument::parse(Text::new(&updated));
                assert_eq!(doc.reuse_stats(), html.reuse_stats());
                assert_html_fresh(doc.host(), &fresh);
                assert!(
                    doc.highlights(all.clone())
                        .unwrap()
                        .eq(full.highlights(all).unwrap())
                );
                let expressions = doc.expressions();
                assert_eq!(expressions.len(), oracle.len());
                assert_eq!(expressions.len(), full.expressions().len());
                for ((expr, full), oracle) in expressions.iter().zip(full.expressions()).zip(oracle)
                {
                    assert_eq!(expr.range(), full.range());
                    assert_eq!(expr.kind(), full.kind());
                    assert_script(
                        oracle,
                        expr.range(),
                        expr.document(),
                        full.document(),
                        &updated,
                    );
                }
            }
            "angular" => {
                let doc = AngularDocument::parse(Text::new(source))
                    .reparse(&changes)
                    .unwrap();
                let full = AngularDocument::parse(Text::new(&updated));
                assert_eq!(doc.reuse_stats(), html.reuse_stats());
                assert_html_fresh(doc.html(), &fresh);
                assert!(
                    doc.highlights(all.clone())
                        .unwrap()
                        .eq(full.highlights(all).unwrap())
                );
                assert_eq!(doc.regions().len(), full.regions().len());
                let mut count = 0;
                for (region, full) in doc.regions().iter().zip(full.regions()) {
                    assert_eq!(region.kind(), full.kind());
                    assert_eq!(region.is_recovered(), full.is_recovered());
                    assert!(
                        region
                            .nodes()
                            .map(|node| (node.name(), node.range()))
                            .eq(full.nodes().map(|node| (node.name(), node.range())))
                    );
                    assert_eq!(region.scripts().len(), full.scripts().len());
                    for (script, full) in region.scripts().iter().zip(full.scripts()) {
                        assert_eq!(script.range(), full.range());
                        assert_script(
                            &oracle[count],
                            script.range(),
                            script.document(),
                            full.document(),
                            &updated,
                        );
                        count += 1;
                    }
                }
                assert_eq!(count, oracle.len());
            }
            _ => assert!(oracle.is_empty()),
        }
    }
}

#[test]
fn recovered_host_drops_checkpoints_and_a_subsequent_strict_parse_rebuilds_them() {
    let prefix = "<main><p>🦀 &amp; text</p>\r\n".to_owned() + &"<p>other text</p>\r\n".repeat(16);
    let broken = format!("{prefix}<section><b></section></main>");
    let valid = format!("{prefix}<section><b></b></section></main>");
    let parse = |source: &str| HtmlDocument::parse(Text::new(source));
    let seed = parse(&valid);
    let at = valid.find("</b>").unwrap();
    let recovered = seed.reparse(&edit(at..at + 4, "")).unwrap();
    assert!(
        recovered
            .nodes()
            .any(|node| node.kind() == HtmlNodeKind::Error)
    );
    assert_eq!(recovered.reuse_stats(), ReuseStats::default());
    assert_html_fresh(&recovered, &parse(&broken));

    let repaired = recovered.reparse(&edit(at..at, "</b>")).unwrap();
    assert_eq!(repaired.reuse_stats(), ReuseStats::default());
    assert_html_fresh(&repaired, &seed);
    let no_edit = repaired.reparse(&ChangeSet::default()).unwrap();
    assert_reused(no_edit.reuse_stats(), valid.len());
    assert_html_fresh(&no_edit, &seed);
}

#[test]
fn rejects_a_checkpoint_that_read_the_edited_tag_name() {
    // The opening `<` lands on a checkpoint, but its scanner has already
    // read the tag name to choose between an ordinary and a raw element.
    let prefix = format!("<main>{}xxxxxxxxxxxxxxx<", "<p>text</p>".repeat(10));
    let at = prefix.len();
    let source = format!("{prefix}span>body</span></main>");
    let seed = HtmlDocument::parse(Text::new(&source));
    let no_edit = seed.reparse(&ChangeSet::default()).unwrap();
    assert_eq!(no_edit.reuse_stats().resumed_at, at);

    let close = source.find("</span>").unwrap() + 2;
    let changes = ChangeSet::new(vec![
        Change::new(at..at + 4, "script"),
        Change::new(close..close + 4, "script"),
    ])
    .unwrap();
    let next = seed.reparse(&changes).unwrap();
    assert_reused(next.reuse_stats(), at - 1);
    assert!(
        next.nodes()
            .any(|node| node.kind() == HtmlNodeKind::ScriptText)
    );
    let fresh = HtmlDocument::parse(Text::new(format!("{prefix}script>body</script></main>")));
    assert_html_fresh(&next, &fresh);
}

#[test]
fn earliest_edit_bounds_reuse_and_utf8_edits_are_validated() {
    let source = "<main><p>🦀</p>".to_owned() + &"<p>text</p>".repeat(16) + "</main>";
    let seed = HtmlDocument::parse(Text::new(&source));
    let end = source.rfind("text").unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(0..0, "<!doctype html>"),
        Change::new(end..end + 4, "edited"),
    ])
    .unwrap();
    let doc = seed.reparse(&changes).unwrap();
    assert_eq!(doc.reuse_stats(), ReuseStats::default());
    let mut updated = source.clone();
    updated.replace_range(end..end + 4, "edited");
    updated.insert_str(0, "<!doctype html>");
    assert_html_fresh(&doc, &HtmlDocument::parse(Text::new(&updated)));

    let invalid = source.find('🦀').unwrap() + 1;
    let changes = edit(invalid..invalid, "!");
    assert!(seed.reparse(&changes).is_err());
    assert!(
        VueDocument::parse(Text::new(&source))
            .reparse(&changes)
            .is_err()
    );
    assert!(
        AngularDocument::parse(Text::new(&source))
            .reparse(&changes)
            .is_err()
    );
    assert_eq!(seed.text().as_str(), source);
}

#[derive(Debug)]
struct CustomDocument;
impl HtmlCustomDocument for CustomDocument {}

#[test]
fn reparse_can_rebuild_borrowed_custom_mounts_without_changing_host_reuse() {
    let source =
        "<x-data>old</x-data>".to_owned() + &"<p>prefix</p>".repeat(16) + "<x-data>end</x-data>";
    let calls = RefCell::new(Vec::new());
    let parse = |text: Text| -> Box<dyn HtmlCustomDocument> {
        calls.borrow_mut().push(text.as_str().to_owned());
        Box::new(CustomDocument)
    };
    let nesting = HtmlNesting {
        nested_languages: &[HtmlNestedLanguage {
            tag: "x-data",
            attrs: None,
            parser: HtmlNestedParser {
                name: "test",
                parse: &parse,
            },
        }],
        ..HtmlNesting::default()
    };
    let doc = HtmlDocument::parse_with_nesting(Text::new(&source), HtmlOptions::default(), nesting);
    assert_eq!(*calls.borrow(), ["old", "end"]);
    calls.borrow_mut().clear();
    let at = source.rfind("end").unwrap();
    let changes = edit(at..at + 3, "🦀");
    let next = doc.reparse_with_nesting(&changes, nesting).unwrap();
    assert_reused(next.reuse_stats(), at);
    assert_eq!(*calls.borrow(), ["old", "🦀"]);
    assert_eq!(next.mounts().len(), 2);
    assert_eq!(next.mounts()[1].range(), at..at + 4);
    let stock = doc.reparse(&changes).unwrap();
    assert!(stock.mounts().is_empty());
    assert_eq!(stock.reuse_stats(), next.reuse_stats());
    assert_eq!(html_records(&stock, 0), html_records(&next, 0));
}
