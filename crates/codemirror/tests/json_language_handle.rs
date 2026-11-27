use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Precedence, Text, Transaction,
    autocomplete::brackets::close_brackets_enabled,
    language::{
        json::{JsonDocument, json_editor_linter, parse_raw},
        lezer::{ParseError, SyntaxTree},
    },
    language_data::{
        JsonLanguage, NativeSyntax, SupportedDocument, attached_language_snapshot, json_extension,
        json_language, lezer_extension,
    },
    lint::LintConfiguration,
    view::EditorView,
};
use serde_json::{Value, json};

fn fixture() -> impl Iterator<Item = Value> {
    include_str!("fixtures/json_language_handle.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn raw_rows(tree: &SyntaxTree) -> Value {
    let mut pending = vec![tree.root()];
    let mut rows = Vec::new();
    while let Some(node) = pending.pop() {
        let range = node.range();
        rows.push(json!([node.name(), range.start, range.end]));
        pending.extend(node.children().rev());
    }
    json!(rows)
}

fn json_doc(snapshot: &SupportedDocument) -> &JsonDocument {
    let NativeSyntax::Json(doc) = snapshot.syntax() else {
        panic!("expected the JSON grammar");
    };
    doc
}

fn doc_rows(doc: &JsonDocument) -> Value {
    json!(
        doc.nodes()
            .map(|node| {
                let range = node.range();
                json!([node.kind().name(), range.start, range.end])
            })
            .collect::<Vec<_>>()
    )
}

fn parse_failure(error: ParseError) -> Value {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("unexpected parser error: {error}");
    };
    json!({"offset": offset})
}

fn attach_result(source: &str, extension: Extension) -> Value {
    match EditorState::with_extensions(source, extension) {
        Ok(state) => {
            assert!(!LintConfiguration::get(&state).has_sources());
            assert!(!close_brackets_enabled(&state).unwrap());
            json!({"tree": doc_rows(json_doc(&attached_language_snapshot(&state).unwrap()))})
        }
        Err(ConfigurationError::StrictSyntax(error)) => parse_failure(error),
        Err(error) => panic!("unexpected configuration error: {error:?}"),
    }
}

fn expect_rejection(error: EditError, expected: &Value) {
    let EditError::Configuration(ConfigurationError::StrictSyntax(error)) = error else {
        panic!("expected a strict syntax rejection, got {error:?}");
    };
    assert_eq!(&parse_failure(error), expected);
}

#[test]
fn raw_and_editor_json_configuration_match_pinned_ranges_and_failures() {
    let stock = json_language();
    let strict = stock.configure(Some(true), Some("strict JSON"));
    let reset = strict.configure(Some(false), Some(""));
    assert_eq!(stock.name(), "json");
    assert_eq!(strict.name(), "strict JSON");
    assert_eq!(reset.name(), strict.name());
    assert!(strict.clone() == strict);
    assert!(strict.configure(None, None) != strict);
    for row in fixture().filter(|row| row["kind"] == "parse") {
        let source = row["source"].as_str().unwrap();
        for (handle, expected) in [(stock, "raw"), (&strict, "strictRaw"), (&reset, "raw")] {
            let actual = match handle.parse(source) {
                Ok(tree) => json!({"tree": raw_rows(&tree)}),
                Err(error) => parse_failure(error),
            };
            assert_eq!(actual, row[expected], "{}: {expected}", row["name"]);
        }
        assert_eq!(
            json!({"tree": raw_rows(&parse_raw(source, false).unwrap())}),
            row["raw"]
        );
        assert_eq!(
            attach_result(source, json_extension()),
            row["editor"],
            "{}",
            row["name"]
        );
        assert_eq!(
            attach_result(source, strict.extension()),
            row["strictEditor"],
            "{}",
            row["name"]
        );
        assert_eq!(
            attach_result(source, reset.extension()),
            row["editor"],
            "{}",
            row["name"]
        );
    }
}

#[test]
fn json_strict_changes_reject_atomically_and_recovery_restores_undo() {
    let row = fixture().find(|row| row["kind"] == "edit").unwrap();
    let source = row["source"].as_str().unwrap();
    let cut = usize::try_from(row["removeAt"].as_u64().unwrap()).unwrap();
    let cut_close = || Transaction::new(ChangeSet::single(Change::new(cut..cut + 1, "")).unwrap());
    let strict = json_language().configure(Some(true), Some("strict JSON"));
    let reset = strict.configure(Some(false), None);
    let slot = Compartment::new();
    let state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let retained = state.clone();
    let original = attached_language_snapshot(&state).unwrap();
    let mut view = EditorView::new(state);
    let select = Transaction::default().with_selection(EditorSelection::single(1, 1));
    view.update(select, (), |_| (None, ())).unwrap();
    let swap = |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    view.update(swap(strict.extension()), (), |_| (None, ()))
        .unwrap();
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &original
    ));
    let selection = view.state().selection().clone();
    let revision = view.revision();
    expect_rejection(
        view.update(cut_close(), (), |_| (None, ())).unwrap_err(),
        &row["rejected"],
    );
    assert_eq!(view.state().text().as_str(), source);
    assert_eq!(view.state().selection(), &selection);
    assert_eq!(
        (view.revision(), view.undo_depth(), view.redo_depth()),
        (revision, 0, 0)
    );
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &original
    ));

    let edit = cut_close().with_effects([slot.reconfigure(reset.extension())]);
    view.update(edit, (), |_| (None, ())).unwrap();
    let recovered = attached_language_snapshot(view.state()).unwrap();
    assert_eq!(doc_rows(json_doc(&recovered)), row["recovered"]);
    expect_rejection(
        view.update(swap(strict.extension()), (), |_| (None, ()))
            .unwrap_err(),
        &row["rejected"],
    );
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &recovered
    ));

    let repair = Transaction::new(ChangeSet::single(Change::new(cut..cut, "}")).unwrap())
        .with_effects([slot.reconfigure(strict.extension())]);
    view.update(repair, (), |_| (None, ())).unwrap();
    let repaired = attached_language_snapshot(view.state()).unwrap();
    assert_eq!(doc_rows(json_doc(&repaired)), row["repaired"]);
    let history = (view.revision(), view.undo_depth(), view.redo_depth());
    assert_eq!(view.undo(), None);
    assert_eq!(
        (view.revision(), view.undo_depth(), view.redo_depth()),
        history
    );
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &repaired
    ));
    assert_eq!(
        JsonLanguage::active_name(view.state()).as_deref(),
        Some("strict JSON")
    );
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(&retained).unwrap(),
        &original
    ));
    view.update(swap(reset.extension()), (), |_| (None, ()))
        .unwrap();
    assert_eq!(view.undo(), Some(()));
    assert_eq!(
        doc_rows(json_doc(&attached_language_snapshot(view.state()).unwrap())),
        row["recovered"]
    );
    assert_eq!(view.redo(), Some(()));
    assert_eq!(view.state().text().as_str(), source);
}

#[test]
fn json_switching_identity_and_precedence_rebuilds_only_active_snapshots() {
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions("[0]", slot.of(json_extension())).unwrap();
    let original = attached_language_snapshot(&state).unwrap();
    let change = |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    state.update(change(json_language().extension())).unwrap();
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(&state).unwrap(),
        &original
    ));
    let renamed = json_language().configure(None, Some("renamed"));
    state.update(change(renamed.extension())).unwrap();
    let current = attached_language_snapshot(&state).unwrap();
    assert!(!Arc::ptr_eq(&current, &original));
    assert_eq!(
        JsonLanguage::active_name(&state).as_deref(),
        Some("renamed")
    );
    assert_eq!(doc_rows(json_doc(&current)), doc_rows(json_doc(&original)));
    state
        .update(change(Extension::group([
            lezer_extension().with_precedence(Precedence::High),
            renamed.configure(Some(true), None).extension(),
        ])))
        .unwrap();
    assert!(matches!(
        attached_language_snapshot(&state).unwrap().syntax(),
        NativeSyntax::LezerGrammar(_)
    ));
    assert!(JsonLanguage::active_name(&state).is_none());
    state.update(change(Extension::default())).unwrap();
    assert!(attached_language_snapshot(&state).is_none());
}

#[test]
fn json_cut_is_separate_from_lint() {
    let row = fixture().find(|row| row["kind"] == "depth").unwrap();
    let depth = usize::try_from(row["depth"].as_u64().unwrap()).unwrap();
    let source = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    let strict = json_language().configure(Some(true), None);
    assert_eq!(
        parse_failure(strict.parse(&source).unwrap_err()),
        row["strict"]
    );
    assert_eq!(attach_result(&source, strict.extension()), row["strict"]);
    assert!(json_editor_linter(&Text::new(source)).is_empty());
}

#[test]
fn json_attachment_reuses_projected_edges_without_splitting_original_ranges() {
    let start = "[\r\n".to_owned() + &"0,\r\n".repeat(320);
    let source = start.clone() + "1,\r\n" + &"2,\r\n".repeat(320) + "3\r\n]";
    let retained = EditorState::with_extensions(&source, json_extension()).unwrap();
    let mut state = retained.clone();
    let at = start.len();
    let replace = Transaction::new(ChangeSet::single(Change::new(at..at + 1, "111")).unwrap());
    state.update(replace).unwrap();
    let check = |state: &EditorState| {
        let snap = attached_language_snapshot(state).unwrap();
        let doc = json_doc(&snap);
        assert_eq!(doc.text(), state.text());
        let cold = EditorState::with_extensions(state.text().as_str(), json_extension()).unwrap();
        assert_eq!(
            doc_rows(doc),
            doc_rows(json_doc(&attached_language_snapshot(&cold).unwrap()))
        );
        for node in doc.nodes() {
            let range = node.range();
            assert!(state.text().slice(range.clone()).is_some());
            for endpoint in [range.start, range.end] {
                assert_ne!(
                    state
                        .text()
                        .as_str()
                        .as_bytes()
                        .get(endpoint.saturating_sub(1)..=endpoint),
                    Some(b"\r\n".as_slice())
                );
            }
        }
        snap
    };
    let shifted = check(&state);
    let normalized = source.replace("\r\n", "\n");
    let mut reference = EditorState::with_extensions(&normalized, json_extension()).unwrap();
    let normalized_at = start.replace("\r\n", "\n").len();
    reference
        .update(Transaction::new(
            ChangeSet::single(Change::new(normalized_at..normalized_at + 1, "111")).unwrap(),
        ))
        .unwrap();
    let expected = attached_language_snapshot(&reference)
        .unwrap()
        .reuse_stats();
    let raw = shifted.text().as_str();
    let mut crlf = 0;
    for (at, _) in raw.match_indices("\r\n") {
        if at - crlf >= expected.resumed_at {
            break;
        }
        crlf += 1;
    }
    assert!(crlf > 140);
    assert_eq!(shifted.reuse_stats().resumed_at, expected.resumed_at + crlf);
    assert_eq!(shifted.reuse_stats().nodes_reused, expected.nodes_reused);
    assert!(expected.nodes_reused > 100);
    assert!(json_doc(&shifted).reused_suffix_actions() > 100);
    // Split a CRLF pair in the original bytes. The normalized text is the same,
    // but a published range or resumed offset must not count the old LF twice.
    let lf = state.text().as_str().find("111,\r\n").unwrap() + "111,\r".len();
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(lf..lf + 1, "")).unwrap(),
        ))
        .unwrap();
    let split = check(&state);
    assert_ne!(split.text(), shifted.text());
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(lf..lf, "\n")).unwrap(),
        ))
        .unwrap();
    assert_eq!(check(&state).text(), shifted.text());
    assert_eq!(retained.text().as_str(), source);
}
