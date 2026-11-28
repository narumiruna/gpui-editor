use serde_json::Value;

use crate::{
    Change, ChangeSet, EditorState, Transaction,
    language_data::{NativeLanguage, json_language, lezer_language},
    view::{EditorView, Viewport},
};

#[test]
fn edits_publish_mapped_in_flight_lr_prefixes() {
    let records = include_str!("../../../tests/fixtures/lr_edit_in_flight_browser.jsonl")
        .lines()
        .map(|row| serde_json::from_str::<Value>(row).unwrap())
        .collect::<Vec<_>>();
    let number = |row: &Value, key| usize::try_from(row[key].as_u64().unwrap()).unwrap();
    for (name, language, source, extension) in [
        (
            "json",
            NativeLanguage::Json,
            format!("[\n{}0\n]", "1,2,3,4,5,6,7,8,9,\n".repeat(800)),
            json_language().viewport_extension().unwrap(),
        ),
        (
            "lezer",
            NativeLanguage::LezerGrammar,
            format!("@top Main {{\n{}}}", "  \"hi\"\n".repeat(1500)),
            lezer_language().viewport_extension().unwrap(),
        ),
    ] {
        let row = |event| {
            records
                .iter()
                .find(|row| row["grammar"] == name && row["event"] == event)
                .unwrap()
        };
        let state = EditorState::with_extensions(source, extension).unwrap();
        let mut view = EditorView::new(state);
        let visible = 0..number(row("install"), "viewport");
        let viewport = Viewport::new(
            view.state().text(),
            visible.clone(),
            std::iter::once(visible),
        )
        .unwrap();
        view.set_viewport_geometry(viewport).unwrap();
        let retained = view.state().clone();
        let initial = retained.lr_syntax_tree(language, 0).unwrap();
        assert_eq!(initial.parsed_to, number(row("install"), "through"));
        assert_eq!(
            initial.tree.root().range().end,
            number(row("install"), "pub")
        );
        let requested = number(row("pending"), "requested");
        let frontier = number(row("pending"), "inFlight");
        // Match the pinned frontier, not the browser's wall-clock budget.
        let private_progress = |view: &EditorView<()>| {
            view.lr_viewport
                .worker
                .as_ref()
                .unwrap()
                .job
                .as_ref()
                .unwrap()
                .in_flight_progress()
                .unwrap_or(0)
        };
        for _ in 0..requested * 2 {
            assert!(
                view.ensure_lr_syntax_tree(language, requested, 1)
                    .unwrap()
                    .is_none()
            );
            if private_progress(&view) >= frontier {
                break;
            }
        }
        assert_eq!(private_progress(&view), frontier, "{name}");
        assert_eq!(
            view.lr_syntax_tree_available(language, frontier),
            row("pending")["available"]
        );
        assert!(std::sync::Arc::ptr_eq(
            &initial,
            &view.state().lr_syntax_tree(language, 0).unwrap()
        ));
        let insert = row("edit")["insert"].as_str().unwrap();
        view.update(
            Transaction::new(ChangeSet::single(Change::new(0..0, insert)).unwrap()),
            (),
            |_| (None, ()),
        )
        .unwrap();
        let through = number(row("edit"), "through");
        assert_eq!(
            view.lr_syntax_tree_available(language, through),
            row("edit")["available"],
            "{name}"
        );
        let output = view.state().lr_syntax_tree(language, through).unwrap();
        assert_eq!(output.parsed_to, through, "{name}");
        assert_eq!(
            output.tree.root().range().end,
            number(row("edit"), "pub"),
            "{name}"
        );
        assert!(std::sync::Arc::ptr_eq(
            &initial,
            &retained.lr_syntax_tree(language, 0).unwrap()
        ));
    }
}
