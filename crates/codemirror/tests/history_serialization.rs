use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Extension, History, HistoryIsolation,
    StateEffectType, Transaction, inverted_effects,
};
use serde_json::{Value, json};

fn cursor(at: usize) -> EditorSelection {
    EditorSelection::single(at, at)
}

fn at(at: usize) -> Value {
    cursor(at).to_json()
}

fn select(at: usize) -> Transaction {
    Transaction::default().with_selection(cursor(at))
}

fn change(from: usize, to: usize, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(from..to, insert)).unwrap())
        .isolate_history(HistoryIsolation::Full)
}

fn apply(history: &mut History<&str>, transaction: Transaction) {
    history
        .update(transaction, "before", |_| (None, "after"))
        .unwrap();
}

#[test]
fn restores_upstream_history_json_with_deferred_mapping_and_selection_only_entries() {
    // Output of @codemirror/commands 6.11.2: two edits, a remote prefix
    // insertion, and one undoSelection. The older edit still uses pre-prefix offsets.
    let data = json!({
        "done": [
            {"selectionsAfter": [at(0)]},
            {"changes": [3, [1]], "startSelection": at(1), "selectionsAfter": [at(4)]},
            {"changes": [5, [1]], "mapped": [0, 1, 4, -1], "startSelection": at(3), "selectionsAfter": []}
        ],
        "undone": [{"selectionsAfter": [at(0)]}]
    });
    let state = EditorState::from_json(&json!({"doc": "PabcXY", "selection": at(6)})).unwrap();
    let mut history = History::from_json(state, &data, "restored").unwrap();
    assert_eq!(history.to_json().unwrap(), data);
    assert!(!history.can_undo_selection());
    assert!(history.can_redo_selection());
    assert_eq!(history.undo(), Some("restored"));
    assert_eq!(history.state().text().as_str(), "PabcX");
    assert_eq!(*history.state().selection(), cursor(3));
    assert_eq!(history.undo_selection("current"), Some("current"));
    assert_eq!(*history.state().selection(), cursor(5));
    assert_eq!(history.undo_selection("current"), Some("restored"));
    assert_eq!(history.state().text().as_str(), "Pabc");
    assert_eq!(*history.state().selection(), cursor(2));
    assert_eq!(history.undo_selection("current"), Some("current"));
    assert_eq!(*history.state().selection(), cursor(0));
    assert_eq!(history.redo(), Some("restored"));
    assert_eq!(history.state().text().as_str(), "PabcX");
    assert_eq!(*history.state().selection(), cursor(2));
    assert_eq!(history.redo(), Some("restored"));
    assert_eq!(history.state().text().as_str(), "PabcXY");
    assert_eq!(*history.state().selection(), cursor(3));
}

#[test]
fn keeps_native_utf8_redo_selection_both_change_branches_and_mapped_edits() {
    let mut history = History::new(EditorState::new("é\r\nb"));
    apply(&mut history, change(2, 2, "X").with_selection(cursor(5)));
    apply(&mut history, select(3));
    apply(&mut history, change(5, 6, "🤖").with_selection(cursor(0)));
    apply(&mut history, change(0, 0, "🌱").with_add_to_history(false));
    assert_eq!(history.undo(), Some("before"));
    let state = EditorState::from_json(&history.state().to_json()).unwrap();
    let data = history.to_json().unwrap();
    assert_eq!(data["undone"][0]["oppositeSelection"], at(7));
    let mut restored = History::from_json(state, &data, "restored").unwrap();
    assert_eq!(restored.to_json().unwrap(), data);

    for history in [&mut history, &mut restored] {
        assert!(history.can_redo());
        assert!(history.can_undo_selection());
        history.redo().unwrap();
        assert_eq!(history.state().text().as_str(), "🌱éX\r\n🤖");
        assert_eq!(*history.state().selection(), cursor(0));
        history.undo().unwrap();
        assert_eq!(*history.state().selection(), cursor(7));
        history.undo_selection("current").unwrap();
        assert_eq!(*history.state().selection(), cursor(9));
        history.undo().unwrap();
        assert_eq!(history.state().text().as_str(), "🌱é\r\nb");
        history.redo().unwrap();
        assert_eq!(*history.state().selection(), cursor(9));
    }
}

#[test]
fn omits_runtime_effects_and_grouping_state() {
    let effect = StateEffectType::<()>::new();
    let invert = inverted_effects(move |_, _, _| vec![effect.of(())]);
    let mut history = History::new(EditorState::with_extensions("abc", invert).unwrap());
    apply(&mut history, select(2));
    assert!(!history.can_undo_selection());
    let data = history.to_json().unwrap();
    assert_eq!(data["done"][0]["changes"], json!([3]));
    assert!(data["done"][0].get("effects").is_none());
    let state = EditorState::from_json(&history.state().to_json()).unwrap();
    let mut restored = History::from_json(state, &data, "restored").unwrap();
    assert_eq!(restored.undo(), Some("restored"));
    assert_eq!(*restored.state().selection(), cursor(0));
    assert!(!restored.can_redo());
    assert!(restored.can_redo_selection());

    let mut history = History::new(EditorState::new(""));
    history.start_group();
    let insert =
        |at| Transaction::new(ChangeSet::single(Change::new(at..at, "a")).unwrap()).with_time(1000);
    apply(&mut history, insert(0));
    let mut restored = History::from_json(
        history.state().clone(),
        &history.to_json().unwrap(),
        "restored",
    )
    .unwrap();
    apply(&mut restored, insert(1));
    assert_eq!(restored.undo(), Some("before"));
    assert_eq!(restored.state().text().as_str(), "a");
}

#[test]
fn combined_history_json_restores_state_and_branches_with_the_supplied_policy() {
    let mut history = History::new(EditorState::new("é"));
    apply(&mut history, change(2, 2, "X").with_selection(cursor(3)));
    apply(&mut history, change(3, 3, "Y").with_selection(cursor(4)));
    assert_eq!(history.undo(), Some("before"));
    let data = history.to_state_json().unwrap();
    assert_eq!(data["doc"], "éX");
    assert_eq!(data["selection"], at(3));
    assert_eq!(data["history"], history.to_json().unwrap());

    let config = EditorState::history_config(Some(2), Some(10));
    let mut restored = History::from_state_json(&data, config, "restored").unwrap();
    assert_eq!(restored.to_state_json().unwrap(), data);
    assert_eq!(restored.redo(), Some("restored"));
    assert_eq!(restored.state().text().as_str(), "éXY");
    for time in [1000, 1010] {
        let pos = restored.state().text().len();
        apply(
            &mut restored,
            Transaction::new(ChangeSet::single(Change::new(pos..pos, "!")).unwrap())
                .with_time(time),
        );
    }
    assert_eq!(restored.undo_depth(), 4);
    let empty =
        History::from_state_json(&restored.state().to_json(), Extension::default(), ()).unwrap();
    assert_eq!(empty.state().text().as_str(), "éXY!!");
    assert_eq!(empty.undo_depth(), 0);
}

#[test]
fn rejects_invalid_utf8_boundaries_and_inconsistent_mappings() {
    for done in [
        json!([{"selectionsAfter": [at(1)]}]),
        json!([{"changes": [[1], 2], "startSelection": at(0), "selectionsAfter": []}]),
        json!([{"changes": [[2], 1], "mapped": [2, -1], "startSelection": at(0), "selectionsAfter": []}]),
        // The newest edit keeps the first two bytes unchanged. After the
        // deferred mapping reaches the older edit, its deletion splits 'é'.
        json!([
            {"changes": [[1], 1], "startSelection": at(0), "selectionsAfter": []},
            {"changes": [2, [1]], "mapped": [2, -1], "startSelection": at(0), "selectionsAfter": []}
        ]),
        json!([{"selectionsAfter": []}, {"selectionsAfter": []}]),
    ] {
        let data = json!({"done": done, "undone": []});
        assert!(History::from_json(EditorState::new("éa"), &data, ()).is_err());
    }
}
