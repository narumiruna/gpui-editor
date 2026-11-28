use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, StateField, Transaction,
    language::{
        LanguageData,
        json::JsonParseOutput,
        lezer::{LANGUAGE_DATA, LanguageFacet, NodeProp},
    },
    language_data::{JsonLanguage, attached_language_snapshot, json_language},
    view::{EditorView, JsonViewportError, Viewport, update_listener},
};
use serde_json::{Value, json};

fn source() -> String {
    format!("[\n{}  null\n]", "  \"hi\",\n".repeat(1500))
}

fn count(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn row(event: &str) -> Value {
    include_str!("fixtures/json_viewport_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["event"] == event)
        .unwrap()
}

fn assert_publication(
    view: &EditorView<()>,
    event: &str,
    key: &NodeProp<&'static str>,
) -> Arc<JsonParseOutput> {
    let row = row(event);
    let published = view.json_syntax_tree(0).unwrap();
    let ty = published
        .tree
        .root()
        .children()
        .next()
        .unwrap()
        .children()
        .find(|node| node.name() == "String")
        .unwrap()
        .node_type()
        .unwrap();
    assert_eq!(json!(ty.prop(key)), row["marker"], "{event}");
    assert_eq!(
        published.tree.root().range().end,
        count(&row["rootEnd"]),
        "{event}"
    );
    if event != "ensure" {
        assert_eq!(published.parsed_to, count(&row["through"]), "{event}");
    }
    assert_eq!(json!(published.complete), row["complete"], "{event}");
    assert_eq!(view.state().text().len(), count(&row["length"]));
    published
}

fn geometry(view: &mut EditorView<()>, end: usize) {
    let viewport = Viewport::new(view.state().text(), 0..end, std::iter::once(0..end)).unwrap();
    view.set_viewport_geometry(viewport).unwrap();
}

#[test]
fn json_install_stage_publish_idle_and_reconfigure_match_the_pinned_view() {
    let key = NodeProp::new();
    let configured = |marker| {
        let property = key.clone();
        json_language()
            .with_node_props(&[property.add(move |node| node.is("String").then_some(marker))])
    };
    let source = source();
    let slot = Compartment::new();
    let first = configured("first");
    let second = configured("second");
    assert!(
        json_language()
            .configure(Some(true), None)
            .viewport_extension()
            .is_none()
    );
    let state = EditorState::with_extensions(&source, slot.of(first.viewport_extension().unwrap()))
        .unwrap();
    assert!(state.json_syntax_tree(0).is_none());
    let mut view = EditorView::new(state);
    assert!(view.json_idle_work().is_none());
    geometry(&mut view, count(&row("install")["viewport"]));
    let initial = assert_publication(&view, "install", &key);
    let retained = view.state().clone();
    assert!(view.json_syntax_tree(3001).is_none());
    assert!(view.json_viewport_highlights(2999..3001).is_none());
    assert!(
        view.state()
            .json_viewport_highlight_context(&(4..8))
            .is_some()
    );
    let spans: Vec<_> = view.json_viewport_highlights(4..6).unwrap().collect();
    assert_eq!(spans[0].range, 4..6);
    assert!(attached_language_snapshot(view.state()).is_none());
    // A lower request after a pending EOF request still targets the lower cutoff.
    assert!(
        view.ensure_json_syntax_tree(source.len(), 0)
            .unwrap()
            .is_none()
    );
    assert!(view.ensure_json_syntax_tree(5000, 1).unwrap().is_none());
    let stage = view.ensure_json_syntax_tree(5000, 4096).unwrap().unwrap();
    assert_eq!(
        stage.tree.root().range().end,
        count(&row("ensure")["stage"])
    );
    assert!(view.json_syntax_tree_available(5000));
    assert!(view.json_syntax_tree(5000).is_none());
    assert!(Arc::ptr_eq(
        &initial,
        &assert_publication(&view, "ensure", &key)
    ));
    assert!(view.force_json_parsing(5000, 0).unwrap());
    assert!(Arc::ptr_eq(
        &stage,
        &assert_publication(&view, "force", &key)
    ));
    assert!(Arc::ptr_eq(
        &initial,
        &retained.json_syntax_tree(0).unwrap()
    ));
    let ticket = view.json_idle_work().unwrap();
    let idle = view.run_json_idle(&ticket, 16_000).unwrap().unwrap();
    assert_eq!(idle.target, source.len());
    assert!(idle.published);
    let full = assert_publication(&view, "idle", &key);
    assert!(view.json_idle_work().is_none());
    assert!(view.json_syntax_tree(source.len()).is_some());
    assert_eq!(view.revision(), 0);
    assert_eq!(view.undo_depth(), 0);
    view.set_selection(EditorSelection::single(11, 11)).unwrap();
    assert!(Arc::ptr_eq(
        &full,
        &assert_publication(&view, "selection", &key)
    ));
    let full_state = view.state().clone();
    view.update(
        Transaction::default()
            .with_effects([slot.reconfigure(second.viewport_extension().unwrap())]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert_publication(&view, "reconfigure", &key);
    assert!(view.run_json_idle(&ticket, 0).unwrap().is_none());
    assert!(Arc::ptr_eq(&full, &full_state.json_syntax_tree(0).unwrap()));
    view.update(
        Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert!(view.json_syntax_tree(0).is_none());
    assert!(view.json_idle_work().is_none());
}

#[test]
fn a_rejected_publication_keeps_its_stage_and_an_accepted_edit_restarts_the_job() {
    let reject = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let field = StateField::new(|_| Ok(()), {
        let reject = reject.clone();
        move |_, update| {
            if reject.load(Ordering::SeqCst) && update.transaction().changes().is_empty() {
                return Err(ConfigurationError::InvalidFieldData);
            }
            Ok(())
        }
    });
    let listener = update_listener({
        let calls = calls.clone();
        move |update| {
            calls.lock().unwrap().push((
                update.doc_changed(),
                update.selection_set,
                update.state.json_syntax_tree(0).unwrap().parsed_to,
            ));
            Ok(())
        }
    });
    let source = source();
    let state = EditorState::with_extensions(
        &source,
        Extension::group([
            json_language().viewport_extension().unwrap(),
            field.extension(),
            listener,
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    geometry(&mut view, 280);
    calls.lock().unwrap().clear();
    let initial = view.state().clone();
    let ticket = view.json_idle_work().unwrap();
    let stage = view.ensure_json_syntax_tree(5000, 4096).unwrap().unwrap();
    // A higher unsuccessful target cannot prevent a lower ready query.
    assert!(
        view.ensure_json_syntax_tree(source.len(), 0)
            .unwrap()
            .is_none()
    );
    reject.store(true, Ordering::SeqCst);
    assert_eq!(
        view.force_json_parsing(5000, 0),
        Err(JsonViewportError::Commit(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        )))
    );
    assert!(calls.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(
        &initial.json_syntax_tree(0).unwrap(),
        &view.json_syntax_tree(0).unwrap()
    ));
    reject.store(false, Ordering::SeqCst);
    assert!(view.force_json_parsing(5000, 0).unwrap());
    assert!(Arc::ptr_eq(&stage, &view.json_syntax_tree(5000).unwrap()));
    assert_eq!(*calls.lock().unwrap(), [(false, false, 5000)]);
    let retained = view.state().clone();
    let change = ChangeSet::single(Change::new(2..2, "  \"😀\",\r\n")).unwrap();
    view.update(Transaction::new(change), (), |_| (None, ()))
        .unwrap();
    assert!(view.run_json_idle(&ticket, 100).unwrap().is_none());
    assert_eq!(view.revision(), 1);
    assert_eq!(view.undo_depth(), 1);
    assert_eq!(
        view.state().text().as_str(),
        source.replacen("[\n", "[\n  \"😀\",\r\n", 1)
    );
    assert!(Arc::ptr_eq(
        &stage,
        &retained.json_syntax_tree(5000).unwrap()
    ));
    assert!(view.undo().is_some());
    assert_eq!(view.state().text().as_str(), source);
    assert!(view.json_syntax_tree(0).is_some());
}

#[test]
fn bare_and_published_states_read_the_configured_root_facet() {
    let data = LanguageFacet::new(Some(LanguageData::default().with("probe", "root")));
    let configured = json_language().with_node_props(&[LANGUAGE_DATA.add({
        let data = data.clone();
        move |row| row.is_top().then(|| Some(data.clone()))
    })]);
    let state = EditorState::with_extensions(
        "[\r\n\"😀\"]",
        Extension::group([
            configured.viewport_extension().unwrap(),
            data.of(LanguageData::default().with("probe", "entry")),
        ]),
    )
    .unwrap();
    assert_eq!(JsonLanguage::active_name(&state).as_deref(), Some("json"));
    assert!(
        configured
            .find_regions(&state)
            .into_iter()
            .eq(std::iter::once(0..state.text().len()))
    );
    assert!(!configured.is_active_at(&state, 0, gpui_editor_codemirror::Assoc::After));
    let probe = |state: &EditorState, at| {
        state.language_data_at::<&str>("probe", at, gpui_editor_codemirror::Assoc::After)
    };
    assert_eq!(probe(&state, 0), ["entry", "root"]);
    let view = EditorView::<()>::new(state);
    assert!(view.json_syntax_tree(view.state().text().len()).is_some());
    assert_eq!(probe(view.state(), 0), ["entry", "root"]);
    assert!(probe(view.state(), 2).is_empty());
    assert!(probe(view.state(), 5).is_empty());
}
