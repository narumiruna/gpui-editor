use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorState, Extension, Text, Transaction,
    language::{
        LanguageData,
        lezer::{LANGUAGE_DATA, LanguageFacet, LrParseOutput, NodeProp, SyntaxTree},
    },
    language_data::{
        LezerLanguage, NativeLanguage, attached_language_snapshot, json_language, lezer_language,
    },
    view::{EditorView, Viewport},
};
use serde_json::{Value, json};

fn source() -> String {
    format!("@top Main {{\n{}}}", "  \"hi\"\n".repeat(1500))
}

fn number(row: &Value, key: &str) -> usize {
    usize::try_from(row[key].as_u64().unwrap()).unwrap()
}

fn row(step: &str) -> Value {
    include_str!("fixtures/lezer_viewport_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["step"] == step)
        .unwrap()
}

fn literal_marker(tree: &SyntaxTree, key: &NodeProp<&'static str>) -> Option<&'static str> {
    let mut work = vec![tree.root()];
    while let Some(node) = work.pop() {
        if node.name() == "Literal" {
            return node.node_type()?.prop(key).copied();
        }
        work.extend(node.children().rev());
    }
    None
}

fn check(step: &str, view: &EditorView<()>, key: &NodeProp<&'static str>) -> Arc<LrParseOutput> {
    let expected = row(step);
    let tree = view.lezer_syntax_tree(0).unwrap();
    assert_eq!(tree.language(), NativeLanguage::LezerGrammar);
    assert_eq!(
        tree.tree.root().range().end,
        number(&expected, "root"),
        "{step}"
    );
    if step != "stage" {
        assert_eq!(tree.parsed_to, number(&expected, "coverage"), "{step}");
    }
    assert_eq!(
        json!(literal_marker(&tree.tree, key)),
        expected["marker"],
        "{step}"
    );
    assert_eq!(json!(tree.complete), expected["complete"], "{step}");
    assert!(view.json_syntax_tree(0).is_none());
    assert_eq!(view.state().text().len(), number(&expected, "length"));
    tree
}

#[test]
fn pinned_lezer_publication_lifecycle_and_row_identity() {
    let marker = NodeProp::new();
    let configure = |label| {
        let key = marker.clone();
        lezer_language()
            .with_node_props(&[key.add(move |node| node.is("Literal").then_some(label))])
    };
    let first = configure("first");
    let slot = Compartment::new();
    let text = source();
    let state =
        EditorState::with_extensions(&text, slot.of(first.viewport_extension().unwrap())).unwrap();
    assert!(state.lezer_syntax_tree(0).is_none());
    assert!(attached_language_snapshot(&state).is_none());
    let mut view = EditorView::new(state);
    assert!(view.lezer_idle_work().is_none());
    let visible = 0..number(&row("install"), "viewport");
    let viewport = Viewport::new(
        view.state().text(),
        visible.clone(),
        std::iter::once(visible),
    )
    .unwrap();
    view.set_viewport_geometry(viewport).unwrap();
    let initial = check("install", &view, &marker);
    assert!(view.lezer_syntax_tree(3001).is_none());
    let original = view.state().clone();
    assert!(
        view.lezer_viewport_highlights(1..2)
            .unwrap()
            .next()
            .is_some()
    );
    assert!(view.lezer_viewport_highlights(3000..3001).is_none());
    let expected = row("stage");
    let stage = view.ensure_lezer_syntax_tree(5000, 4096).unwrap().unwrap();
    assert_eq!(stage.tree.root().range().end, number(&expected, "working"));
    assert_eq!(
        json!(literal_marker(&stage.tree, &marker)),
        expected["markerAt5000"]
    );
    assert_eq!(
        json!(view.lezer_syntax_tree_available(5000)),
        expected["available"]
    );
    assert!(Arc::ptr_eq(&initial, &check("stage", &view, &marker)));
    assert!(view.force_lezer_parsing(5000, 0).unwrap());
    assert!(Arc::ptr_eq(&stage, &check("publish", &view, &marker)));
    assert!(Arc::ptr_eq(
        &original.lezer_syntax_tree(0).unwrap(),
        &initial
    ));
    let idle = view.lezer_idle_work().unwrap();
    assert!(
        view.run_lezer_idle(&idle, 16_000)
            .unwrap()
            .unwrap()
            .published
    );
    let full = check("idle", &view, &marker);
    assert!(view.lezer_idle_work().is_none());
    assert_eq!((view.revision(), view.undo_depth()), (0, 0));
    let previous = view.state().clone();
    view.update(
        Transaction::default()
            .with_effects([slot.reconfigure(configure("next").viewport_extension().unwrap())]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    check("swap", &view, &marker);
    assert!(Arc::ptr_eq(
        &previous.lezer_syntax_tree(text.len()).unwrap(),
        &full
    ));
    assert!(view.run_lezer_idle(&idle, 10).unwrap().is_none());
    view.update(
        Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert!(view.lezer_syntax_tree(0).is_none());
}

#[test]
fn switching_between_viewport_grammars_keeps_old_states_and_rejects_old_tickets() {
    let marker = NodeProp::new();
    let data = LanguageFacet::new(Some(LanguageData::default().with("probe", "lezer")));
    let lang = lezer_language().with_node_props(&[
        marker.add(|node| node.is("Literal").then_some("custom")),
        LANGUAGE_DATA.add(move |row| row.is_top().then(|| Some(data.clone()))),
    ]);
    let text = source();
    let mut job = lang.parse_job(Text::new(&text)).unwrap();
    let direct = job.advance(3000, 4096).unwrap().output.unwrap();
    assert_eq!(literal_marker(&direct.tree, &marker), Some("custom"));
    let compartment = Compartment::new();
    let state = EditorState::with_extensions(
        text.as_str(),
        compartment.of(lang.viewport_extension().unwrap()),
    )
    .unwrap();
    assert_eq!(LezerLanguage::active_name(&state).as_deref(), Some("lezer"));
    assert_eq!(
        state.language_data_at::<&str>("probe", 0, Assoc::After),
        ["lezer"]
    );
    assert!(!lang.is_active_at(&state, 0, Assoc::After));
    assert_eq!(
        lang.find_regions(&state).into_iter().next(),
        Some(0..text.len())
    );
    let mut view = EditorView::new(state);
    let viewport = Viewport::new(view.state().text(), 0..200, std::iter::once(0..200)).unwrap();
    view.set_viewport_geometry(viewport).unwrap();
    let retained = view.state().clone();
    let ticket = view.lezer_idle_work().unwrap();
    let transaction =
        Transaction::new(ChangeSet::single(Change::new(0..text.len(), "[\r\n\"😀\"]")).unwrap())
            .with_effects([compartment.reconfigure(json_language().viewport_extension().unwrap())]);
    view.update(transaction, (), |_| (None, ())).unwrap();
    assert!(view.lezer_syntax_tree(0).is_none());
    assert!(!view.lezer_syntax_tree_available(0));
    assert!(!view.force_lezer_parsing(0, 100).unwrap());
    assert!(view.lezer_viewport_highlights(0..1).is_none());
    assert_eq!(
        view.json_syntax_tree(0).unwrap().language(),
        NativeLanguage::Json
    );
    assert!(view.run_json_idle(&ticket, 10).unwrap().is_none());
    assert!(view.run_lezer_idle(&ticket, 10).unwrap().is_none());
    assert_eq!(
        literal_marker(&retained.lezer_syntax_tree(0).unwrap().tree, &marker),
        Some("custom")
    );
}
