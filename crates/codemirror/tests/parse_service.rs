//! Shared observations from @codemirror/language 6.13.1, @codemirror/state
//! 6.7.6, and @lezer/common 1.5.7. The oracle used custom parsers and a fake
//! clock, a deferred `parseMixed` mount, and `ParseContext.changes`.

use std::{cell::Cell, convert::Infallible, ops::Range};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::{
        ParseJob, ParseRequest, ParseService, ParseSkip, ParseStep, html::HtmlEmbeddedLanguage,
        json::JsonDocument, whole_document,
    },
    language_data::{NativeLanguage, NativeSyntax},
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/parse_service.json")).unwrap()
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn range(value: &Value) -> Range<usize> {
    number(&value["from"])..number(&value["to"])
}

fn availability<J: ParseJob>(service: &ParseService<J>, targets: &Value) -> Value {
    json!(
        targets
            .as_array()
            .unwrap()
            .iter()
            .map(|at| service.available(number(at)))
            .collect::<Vec<_>>()
    )
}

struct Chunked<'a> {
    calls: &'a Cell<usize>,
    end: usize,
}

impl ParseJob for Chunked<'_> {
    type Tree = String;
    type Error = Infallible;

    fn advance(
        &mut self,
        request: ParseRequest<'_, String>,
    ) -> Result<ParseStep<String>, Infallible> {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        if calls == 2 {
            return Ok(ParseStep::Pending);
        }
        if !request.dirty_ranges.is_empty() {
            assert_eq!(request.previous.unwrap().tree(), "abcde");
            assert_eq!(self.end, 0, "edits must restart in-flight work");
        }
        self.end = (self.end + 2).min(request.upto);
        let prefix = request.text.slice(0..self.end).unwrap();
        Ok(ParseStep::publish(prefix.to_owned(), self.end))
    }

    fn restart(&mut self) {
        self.end = 0;
    }
}

#[test]
fn ensures_partial_prefixes_and_keeps_work_across_budgets() {
    let case = &fixture()["partial"];
    let calls = Cell::new(0);
    let document = case["document"].as_str().unwrap();
    let mut service = ParseService::new(
        Text::new(document),
        Chunked {
            calls: &calls,
            end: 0,
        },
    );
    assert!(service.syntax_tree().is_none());
    assert!(!service.available(0));
    assert!(service.ensure(2, 0).unwrap().is_none());
    assert_eq!(calls.get(), 0);

    service.work(1).unwrap();
    assert_eq!(
        service.syntax_tree().unwrap().parsed_to(),
        number(&case["initialEnd"])
    );
    assert_eq!(availability(&service, &case["targets"]), case["before"]);
    let upto = number(&case["target"]);
    assert!(service.ensure(upto, 1).unwrap().is_none()); // The job yielded no tree.
    assert_eq!(service.syntax_tree().unwrap().tree(), "ab");
    assert!(service.ensure(upto, 1).unwrap().is_none()); // New tree is still too short.
    let ensured = service.ensure(upto, 2).unwrap().unwrap();
    assert_eq!(ensured.parsed_to(), number(&case["ensuredEnd"]));
    assert_eq!(ensured.tree(), &document[..upto]);
    assert_eq!(availability(&service, &case["targets"]), case["after"]);
    assert_eq!(service.viewport(), 0..document.len());
    assert!(service.ensure(upto, 50).unwrap().is_some());
    assert_eq!(calls.get(), 4);

    service
        .apply_changes(&ChangeSet::single(Change::new(0..1, "x")).unwrap())
        .unwrap();
    assert!(service.ensure(usize::MAX, 1).unwrap().is_none());
    assert_eq!(service.syntax_tree().unwrap().tree(), "xb");
}

struct DeferredChild<'a> {
    range: Range<usize>,
    ready: &'a Cell<bool>,
    calls: usize,
}

impl ParseJob for DeferredChild<'_> {
    type Tree = Option<JsonDocument>;
    type Error = Infallible;

    fn advance(
        &mut self,
        request: ParseRequest<'_, Self::Tree>,
    ) -> Result<ParseStep<Self::Tree>, Infallible> {
        self.calls += 1;
        if self.ready.get() {
            assert_eq!(request.dirty_ranges, std::slice::from_ref(&self.range));
            assert!(request.skipped_ranges.is_empty());
            assert!(request.previous.unwrap().tree().is_none());
            Ok(ParseStep::publish(
                request
                    .text
                    .slice_owned(self.range.clone())
                    .map(JsonDocument::parse),
                request.text.len(),
            ))
        } else {
            assert_eq!(self.calls, 1, "do not retry a blocked mount");
            Ok(ParseStep::Publish {
                tree: None,
                parsed_to: request.text.len(),
                skipped: vec![ParseSkip::Deferred(self.range.clone())],
            })
        }
    }

    fn restart(&mut self) {}
}

#[test]
fn deferred_mount_blocks_availability_until_explicit_wake() {
    let case = &fixture()["deferred"];
    let prefix = number(&case["prefix"]);
    let child = case["child"].as_str().unwrap();
    let suffix = number(&case["suffix"]);
    let document = format!("{}{child}{}", "x".repeat(prefix), "y".repeat(suffix));
    let ready = Cell::new(false);
    let job = DeferredChild {
        range: prefix..prefix + child.len(),
        ready: &ready,
        calls: 0,
    };
    let mut service = ParseService::new(Text::new(document), job);
    service.work(3).unwrap();
    assert!(service.syntax_tree().unwrap().tree().is_none());
    assert_eq!(availability(&service, &case["targets"]), case["before"]);
    assert!(service.ensure(usize::MAX, 3).unwrap().is_none());
    ready.set(true);
    assert!(service.resume_deferred());
    assert!(!service.resume_deferred());
    assert!(!service.available(prefix + 1));
    assert_eq!(
        service
            .ensure(usize::MAX, 1)
            .unwrap()
            .unwrap()
            .tree()
            .as_ref()
            .unwrap()
            .text()
            .as_str(),
        child
    );
    assert_eq!(availability(&service, &case["targets"]), case["after"]);
}

struct ViewportSkips {
    skips: Vec<ParseSkip>,
}

impl ParseJob for ViewportSkips {
    type Tree = Text;
    type Error = Infallible;

    fn advance(&mut self, request: ParseRequest<'_, Text>) -> Result<ParseStep<Text>, Infallible> {
        Ok(ParseStep::Publish {
            tree: request.text.clone(),
            parsed_to: request.text.len(),
            skipped: std::mem::take(&mut self.skips),
        })
    }

    fn restart(&mut self) {}
}

#[test]
fn maps_viewport_skips_and_dirty_ranges_across_edits() {
    let case = &fixture()["mapping"];
    let job = ViewportSkips {
        skips: case["oldSkips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skip| ParseSkip::UntilVisible(range(skip)))
            .collect(),
    };
    let mut service = ParseService::new(Text::new(case["document"].as_str().unwrap()), job);
    assert!(!service.set_viewport(range(&case["oldViewport"])).unwrap());
    service.work(1).unwrap();
    let changes = ChangeSet::new(
        case["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|change| {
                let from = number(&change["from"]);
                Change::new(
                    from..change["to"]
                        .as_u64()
                        .map_or(from, |to| usize::try_from(to).unwrap()),
                    change["insert"].as_str().unwrap_or(""),
                )
            })
            .collect(),
    )
    .unwrap();
    service.apply_changes(&changes).unwrap();
    assert_eq!(service.viewport(), range(&case["newViewport"]));
    assert!(service.syntax_tree().is_none());
    assert_eq!(
        service.dirty_ranges(),
        &[3..4, 9..10, 12..13, 17..19, 20..21, 22..22]
    );
    let skip = range(&case["newSkips"][0]);
    assert_eq!(
        service.skipped_ranges(),
        &[ParseSkip::UntilVisible(skip.clone())]
    );
    assert_eq!(
        service.set_viewport(skip.start - 1..skip.start).unwrap(),
        case["beforeTouch"].as_bool().unwrap()
    );
    assert_eq!(
        service
            .set_viewport(skip.start - 1..skip.start + 1)
            .unwrap(),
        case["afterOverlap"].as_bool().unwrap()
    );
    assert!(service.skipped_ranges().is_empty());

    service
        .apply_changes(&ChangeSet::single(Change::new(11..19, "é")).unwrap())
        .unwrap();
    assert_eq!(service.dirty_ranges(), &[3..4, 9..10, 11..15, 16..16]);
    assert_eq!(service.text().as_str(), "abc[defgh]iép?q");
    service.work(1).unwrap();
    assert!(service.available(usize::MAX));
    assert!(service.dirty_ranges().is_empty());
}

#[test]
fn ensures_outside_view_and_restores_the_saved_viewport() {
    let mut service = ParseService::new(
        Text::new("0123456789"),
        ViewportSkips {
            skips: vec![ParseSkip::UntilVisible(4..10)],
        },
    );
    service.set_viewport(8..10).unwrap();
    assert!(service.ensure(4, 1).unwrap().is_some());
    assert_eq!(service.viewport(), 8..10);
    assert_eq!(service.dirty_ranges(), std::slice::from_ref(&(4..10)));
    assert!(!service.available(5));
    service.work(1).unwrap();
    assert!(service.available(10));

    let mut service = ParseService::new(
        Text::new("0123456789"),
        ViewportSkips {
            skips: vec![ParseSkip::UntilVisible(0..4)],
        },
    );
    service.set_viewport(8..10).unwrap();
    service.work(1).unwrap();
    assert!(!service.available(4));
    assert!(service.ensure(4, 0).unwrap().is_none());
    assert_eq!(service.skipped_ranges(), &[ParseSkip::UntilVisible(0..4)]);
    assert!(service.ensure(4, 1).unwrap().is_some());
    assert!(service.skipped_ranges().is_empty());
    assert_eq!(service.viewport(), 8..10);
}

#[test]
fn wraps_native_html_with_mounted_syntax_and_validates_utf8_edits() {
    let text = Text::new("😀<script>let x = 1;</script>");
    let job = whole_document(|text| NativeLanguage::Html.parse(text.clone()));
    let mut service = ParseService::new(text, job);
    assert_eq!(service.set_viewport(1..2), Err(EditError::InvalidRange));
    let NativeSyntax::Html(doc) = service.ensure(1, 1).unwrap().unwrap().tree() else {
        panic!("expected HTML");
    };
    assert_eq!(doc.mounts()[0].language(), HtmlEmbeddedLanguage::JavaScript);
    assert_eq!(doc.mounts()[0].range(), 12..22);
    assert!(service.available(usize::MAX));

    let invalid = ChangeSet::single(Change::new(1..2, "?")).unwrap();
    assert_eq!(
        service.apply_changes(&invalid),
        Err(EditError::InvalidRange)
    );
    service.apply_changes(&ChangeSet::default()).unwrap();
    assert!(service.available(usize::MAX));
    let edit = ChangeSet::single(Change::new(4..4, "é")).unwrap();
    service.apply_changes(&edit).unwrap();
    assert_eq!(service.dirty_ranges(), std::slice::from_ref(&(4..6)));
    assert!(service.syntax_tree().is_none());
    let NativeSyntax::Html(doc) = service.ensure(usize::MAX, 1).unwrap().unwrap().tree() else {
        panic!("expected HTML");
    };
    assert_eq!(doc.mounts()[0].range(), 14..24);
}
