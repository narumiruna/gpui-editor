use std::{collections::HashMap, ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, Transaction,
    language::{LanguageData, lezer::ParseError},
    language_data::{
        NativeSyntax, SupportedDocument, WastLanguage, attached_language_snapshot, json_extension,
        wast_extension, wast_language,
    },
    view::EditorView,
};
use serde_json::{Value, json};

fn oracle() -> HashMap<String, Value> {
    include_str!("fixtures/wast_language_handle.jsonl")
        .lines()
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["step"].as_str().unwrap().into(), row)
        })
        .collect()
}

fn parenthesized_tree(document: &SupportedDocument) -> Value {
    let NativeSyntax::Wast(doc) = document.syntax() else {
        return json!([]);
    };
    let mut records = Vec::new();
    let mut pending = vec![(0, doc.root())];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        records.push(json!([depth, node.kind().name(), range.start, range.end]));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    json!(records)
}

fn append(state: &EditorState, text: &str) -> Transaction {
    let end = state.text().len();
    Transaction::new(ChangeSet::single(Change::new(end..end, text)).unwrap())
}

fn check_error(error: EditError, row: &Value) {
    let EditError::Configuration(ConfigurationError::StrictSyntax(ParseError::UnexpectedToken {
        offset,
        ..
    })) = error
    else {
        panic!("expected strict syntax error, got {error:?}");
    };
    assert_eq!(json!(offset), row["at"], "{}", row["step"]);
}

#[test]
fn configured_language_preserves_shared_metadata_but_rebuilds_its_parser() {
    let rows = oracle();
    let original = rows["initial"]["source"].as_str().unwrap();
    let stock = wast_language();
    let renamed = stock.configure(None, Some("renamed-wast"));
    let strict = renamed.configure(Some(true), Some("strict-wast"));
    let reset = strict.configure(Some(false), Some("recovered-wast"));
    assert_eq!(stock.name(), "wast");
    assert_eq!(renamed.name(), "renamed-wast");
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(
        original,
        Extension::group([
            renamed
                .data(LanguageData::default().with("wordChars", "@"))
                .with_precedence(Precedence::High),
            stock.data(LanguageData::default().with("wordChars", "$")),
            slot.of(wast_extension()),
        ]),
    )
    .unwrap();
    let retained = state.clone();
    let mut last: Option<Arc<SupportedDocument>> = None;
    let mut snapshot = |state: &EditorState, name| {
        let row = &rows[name];
        let active = attached_language_snapshot(state);
        let reused = matches!((&last, &active), (Some(old), Some(new)) if Arc::ptr_eq(old, new));
        assert_eq!(json!(reused), row["reused"], "{name}");
        assert_eq!(
            state.text().as_str(),
            row["source"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            json!(state.selection().main().head()),
            row["head"],
            "{name}"
        );
        let root = match active.as_deref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Wast(doc)) => doc.root().kind().name(),
            Some(NativeSyntax::Json(doc)) => doc.root().kind().name(),
            None => "",
            _ => unreachable!(),
        };
        assert_eq!(json!(root), row["root"], "{name}");
        let chosen_name = WastLanguage::active_name(state);
        if root != "JsonText" {
            assert_eq!(json!(chosen_name.as_deref()), row["name"], "{name}");
        } else {
            assert!(chosen_name.is_none());
        }
        let positions = [0, state.text().len()];
        assert_eq!(
            json!(positions.map(|pos| stock.is_active_at(state, pos, Assoc::Before))),
            row["defaultActive"],
            "{name}"
        );
        assert_eq!(
            json!(positions.map(|pos| renamed.is_active_at(state, pos, Assoc::Before))),
            row["configuredActive"],
            "{name}"
        );
        let ranges = |regions: Vec<Range<usize>>| {
            regions
                .into_iter()
                .map(|range| [range.start, range.end])
                .collect::<Vec<_>>()
        };
        assert_eq!(
            json!(ranges(stock.find_regions(state))),
            row["regions"],
            "{name}"
        );
        assert_eq!(
            json!(ranges(strict.find_regions(state))),
            row["regions"],
            "{name}"
        );
        assert_eq!(
            json!(state.language_data_at::<&str>("wordChars", 0, Assoc::Before)),
            row["words"],
            "{name}"
        );
        if let Some(snapshot) = &active {
            assert_eq!(snapshot.text(), state.text(), "{name}");
            assert_eq!(parenthesized_tree(snapshot), row["tree"], "{name}");
        }
        last = active;
    };
    let configure = |state: &mut EditorState, extension| {
        state.update(Transaction::default().with_effects([slot.reconfigure(extension)]))
    };
    snapshot(&state, "initial");
    configure(&mut state, wast_extension()).unwrap();
    snapshot(&state, "same factory language");
    configure(&mut state, renamed.clone().extension()).unwrap();
    snapshot(&state, "renamed configuration");
    configure(&mut state, renamed.configure(None, Some("")).extension()).unwrap();
    snapshot(&state, "distinct parser with the same name");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    snapshot(&state, "selection only");
    configure(&mut state, strict.extension()).unwrap();
    snapshot(&state, "strict configuration");
    let tx = append(&state, "(");
    check_error(state.update(tx).unwrap_err(), &rows["strict edit"]);
    snapshot(&state, "strict edit left old tree");
    let tx = append(&state, "(func)");
    state.update(tx).unwrap();
    snapshot(&state, "strict valid edit");
    configure(&mut state, reset.extension()).unwrap();
    snapshot(&state, "reset strict");
    let tx = append(&state, "(");
    state.update(tx).unwrap();
    snapshot(&state, "recovery after reset");
    let install = EditorState::with_extensions(state.text().as_str(), strict.extension());
    check_error(install.unwrap_err().into(), &rows["invalid strict install"]);
    check_error(
        configure(&mut state, strict.extension()).unwrap_err(),
        &rows["invalid strict reconfiguration"],
    );
    snapshot(&state, "rejected reconfiguration left old tree");
    configure(
        &mut state,
        Extension::group([
            json_extension().with_precedence(Precedence::High),
            strict.extension(),
        ]),
    )
    .unwrap();
    snapshot(&state, "competing language wins");
    configure(&mut state, Extension::default()).unwrap();
    snapshot(&state, "detached");
    snapshot(&retained, "retained state");
    assert!(!stock.is_active_at(&retained, original.find('😀').unwrap() + 1, Assoc::Before));
}

#[test]
fn strict_editor_rejection_leaves_history_and_revision_unchanged() {
    let strict = wast_language().configure(Some(true), None);
    let bad = ";; 😀\r\n(";
    assert!(strict.parse(bad).is_err());
    assert!(
        strict
            .configure(None, Some("still strict"))
            .parse(bad)
            .is_err()
    );
    assert!(strict.configure(Some(false), None).parse(bad).is_ok());
    let state = EditorState::with_extensions("(module)", strict.extension()).unwrap();
    let original = attached_language_snapshot(&state).unwrap();
    let mut view = EditorView::new(state);
    let revision = view.revision();
    let before = view.state().selection().clone();
    let invalid = append(view.state(), "(").with_selection(EditorSelection::single(9, 9));
    assert!(matches!(
        view.update(invalid, (), |_| (None, ())),
        Err(EditError::Configuration(ConfigurationError::StrictSyntax(
            _
        )))
    ));
    assert_eq!(view.revision(), revision);
    assert_eq!(view.undo_depth(), 0);
    assert_eq!(view.state().selection(), &before);
    assert!(Arc::ptr_eq(
        &original,
        &attached_language_snapshot(view.state()).unwrap()
    ));
    let valid = append(view.state(), "\r\n(func)");
    view.update(valid, (), |_| (None, ())).unwrap();
    assert_eq!(view.undo_depth(), 1);
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.state().text(), original.text());
}
