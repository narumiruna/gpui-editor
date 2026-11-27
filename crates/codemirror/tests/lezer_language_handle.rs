use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, Transaction,
    commands::CommentTokens,
    language::{
        lezer::{ParseError, SyntaxTree},
        lezer_grammar::parse_raw,
    },
    language_data::{
        LezerLanguage, NativeSyntax, SupportedDocument, attached_language_snapshot, json_extension,
        lezer_extension, lezer_language,
    },
    view::EditorView,
};
use serde_json::{Value, json};

fn fixtures() -> impl Iterator<Item = Value> {
    include_str!("fixtures/lezer_language_handle.jsonl")
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

fn attached_rows(document: &SupportedDocument) -> Value {
    let NativeSyntax::LezerGrammar(document) = document.syntax() else {
        panic!("expected a Lezer grammar document");
    };
    let mut pending = vec![document.root()];
    let mut rows = Vec::new();
    while let Some(node) = pending.pop() {
        let range = node.range();
        rows.push(json!([node.kind().name(), range.start, range.end]));
        pending.extend(node.children().rev());
    }
    json!(rows)
}

fn parse_failure(error: ParseError) -> Value {
    match error {
        ParseError::UnexpectedToken { offset, .. } => json!({"offset": offset}),
        _ => panic!("unexpected parser error: {error}"),
    }
}

fn attach_result(source: &str, extension: Extension) -> Value {
    match EditorState::with_extensions(source, extension) {
        Ok(state) => json!({"tree": attached_rows(&attached_language_snapshot(&state).unwrap())}),
        Err(ConfigurationError::StrictSyntax(error)) => parse_failure(error),
        Err(error) => panic!("unexpected configuration error: {error:?}"),
    }
}

fn expect_strict_error(error: EditError, expected: &Value) {
    let EditError::Configuration(ConfigurationError::StrictSyntax(error)) = error else {
        panic!("expected a strict syntax rejection, got {error:?}");
    };
    assert_eq!(&parse_failure(error), expected);
}

#[test]
fn direct_and_attached_parsers_match_pinned_trees_and_failures() {
    let stock = lezer_language();
    let strict = stock.configure(Some(true), Some("strict grammar"));
    let reset = strict.configure(Some(false), Some(""));
    assert_eq!(stock.name(), "lezer");
    assert_eq!(strict.name(), "strict grammar");
    assert_eq!(reset.name(), strict.name());
    assert!(strict.clone() == strict);
    assert!(strict.configure(None, None) != strict);
    for row in fixtures().filter(|row| row["kind"] == "parse") {
        let source = row["source"].as_str().unwrap();
        for (language, expected) in [(stock, "raw"), (&strict, "strictRaw"), (&reset, "raw")] {
            let actual = match language.parse(source) {
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
            attach_result(source, lezer_extension()),
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
fn strict_rejection_is_atomic_and_reconfiguration_can_restore_recovery() {
    let row = fixtures().find(|row| row["kind"] == "edit").unwrap();
    let source = row["source"].as_str().unwrap();
    let from = usize::try_from(row["removeAt"].as_u64().unwrap()).unwrap();
    let remove = || Transaction::new(ChangeSet::single(Change::new(from..from + 1, "")).unwrap());
    let slot = Compartment::new();
    let strict = lezer_language().configure(Some(true), Some("strict grammar"));
    let reset = strict.configure(Some(false), None);
    let state = EditorState::with_extensions(source, slot.of(strict.extension())).unwrap();
    let retained = state.clone();
    let original = attached_language_snapshot(&state).unwrap();
    let mut view = EditorView::<()>::new(state);
    let select = Transaction::default().with_selection(EditorSelection::single(1, 1));
    view.update(select, (), |_| (None, ())).unwrap();
    let reconfigure =
        |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    view.update(reconfigure(strict.extension()), (), |_| (None, ()))
        .unwrap();
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &original
    ));
    let revision = view.revision();
    let selection = view.state().selection().clone();
    expect_strict_error(
        view.update(remove(), (), |_| (None, ())).unwrap_err(),
        &row["rejected"],
    );
    assert_eq!(
        (view.revision(), view.undo_depth(), view.redo_depth()),
        (revision, 0, 0)
    );
    assert_eq!(view.state().selection(), &selection);
    assert_eq!(view.state().text().as_str(), source);
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &original
    ));
    let edit = remove().with_effects([slot.reconfigure(reset.extension())]);
    view.update(edit, (), |_| (None, ())).unwrap();
    let recovered = attached_language_snapshot(view.state()).unwrap();
    assert_eq!(attached_rows(&recovered), row["recovered"]);
    expect_strict_error(
        view.update(reconfigure(strict.extension()), (), |_| (None, ()))
            .unwrap_err(),
        &row["rejected"],
    );
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &recovered
    ));
    let repair = Transaction::new(ChangeSet::single(Change::new(from..from, "}")).unwrap())
        .with_effects([slot.reconfigure(strict.extension())]);
    view.update(repair, (), |_| (None, ())).unwrap();
    assert_eq!(
        attached_rows(&attached_language_snapshot(view.state()).unwrap()),
        row["repaired"]
    );
    let repaired = attached_language_snapshot(view.state()).unwrap();
    let history = (view.undo_depth(), view.redo_depth(), view.revision());
    assert!(view.undo_depth() > 0);
    assert_eq!(view.undo(), None);
    assert_eq!(
        (view.undo_depth(), view.redo_depth(), view.revision()),
        history
    );
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(view.state()).unwrap(),
        &repaired
    ));
    assert_eq!(attached_rows(&original), row["repaired"]);
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(&retained).unwrap(),
        &original
    ));
    assert_eq!(
        LezerLanguage::active_name(view.state()).as_deref(),
        Some("strict grammar")
    );
    assert_eq!(
        view.state()
            .language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After)[0]
            .line,
        Some("//")
    );
    view.update(reconfigure(reset.extension()), (), |_| (None, ()))
        .unwrap();
    assert_eq!(view.undo(), Some(()));
    assert_eq!(
        attached_rows(&attached_language_snapshot(view.state()).unwrap()),
        row["recovered"]
    );
    assert_eq!(view.redo(), Some(()));
    assert_eq!(
        attached_rows(&attached_language_snapshot(view.state()).unwrap()),
        row["repaired"]
    );
}

#[test]
fn competing_attachment_and_parser_identity_control_syntax_reuse() {
    let slot = Compartment::new();
    let source = "R { Word }";
    let mut state = EditorState::with_extensions(source, slot.of(lezer_extension())).unwrap();
    let original = attached_language_snapshot(&state).unwrap();
    let swap = |extension| Transaction::default().with_effects([slot.reconfigure(extension)]);
    state.update(swap(lezer_language().extension())).unwrap();
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(&state).unwrap(),
        &original
    ));
    let renamed = lezer_language().configure(None, Some("renamed"));
    state.update(swap(renamed.extension())).unwrap();
    let current = attached_language_snapshot(&state).unwrap();
    assert!(!Arc::ptr_eq(&current, &original));
    assert_eq!(
        LezerLanguage::active_name(&state).as_deref(),
        Some("renamed")
    );
    assert_eq!(attached_rows(&current), attached_rows(&original));
    let strict = lezer_language().configure(Some(true), None);
    state
        .update(swap(Extension::group([
            json_extension().with_precedence(Precedence::High),
            strict.extension(),
        ])))
        .unwrap();
    let json = attached_language_snapshot(&state).unwrap();
    assert!(matches!(json.syntax(), NativeSyntax::Json(_)));
    assert!(LezerLanguage::active_name(&state).is_none());
    state.update(swap(Extension::default())).unwrap();
    assert!(attached_language_snapshot(&state).is_none());
}
