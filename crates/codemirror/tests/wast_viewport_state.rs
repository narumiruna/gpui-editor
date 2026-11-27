use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering::Relaxed},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, StateField, Transaction, TransactionExtension,
    language::{LanguageData, language_data_provider},
    language_data::{attached_language_snapshot, wast_language},
    transaction_extender, transaction_filter,
    view::{
        EditorView, ViewPlugin, ViewPluginType, ViewUpdate, Viewport, WastIdleStep,
        WastViewportError, update_listener,
    },
};

fn viewport_language() -> Extension {
    wast_language().viewport_extension().unwrap()
}

fn edit(insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(0..0, insert)).unwrap())
}

fn measure<M: Clone>(view: &mut EditorView<M>) {
    let viewport = Viewport::new(view.state().text(), 0..9, std::iter::once(0..9)).unwrap();
    view.set_viewport_geometry(viewport).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct Report {
    name: &'static str,
    from: Option<usize>,
    to: Option<usize>,
    doc_changed: bool,
    effects: usize,
}

type Events = Arc<Mutex<Vec<Report>>>;

fn observe(events: &Events, name: &'static str, update: &ViewUpdate<'_>) {
    let coverage = |state: &EditorState| state.wast_syntax_tree(0).map(|out| out.parsed_to);
    let from = coverage(update.start_state);
    let to = coverage(update.state);
    if !update.doc_changed() && update.effects.is_empty() && from != to {
        assert_eq!(update.start_state.text(), update.state.text());
        assert_eq!(update.start_state.selection(), update.state.selection());
        assert_eq!(update.start_viewport, update.viewport);
        assert!(!update.selection_set && !update.viewport_changed && !update.focus_changed);
    }
    events.lock().unwrap().push(Report {
        name,
        from,
        to,
        doc_changed: update.doc_changed(),
        effects: update.effects.len(),
    });
}

struct Probe(Events);

impl ViewPlugin for Probe {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        observe(&self.0, "plugin", update);
    }
}

#[test]
fn only_a_successful_publication_updates_fields_plugins_and_listeners() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let veto = Arc::new(AtomicBool::new(true));
    let field = StateField::new(|_| Ok(0), {
        let calls = calls.clone();
        let veto = veto.clone();
        move |value, update| {
            calls.lock().unwrap().push("field");
            let transaction = update.transaction();
            if !transaction.add_to_history() {
                assert!(transaction.effects().is_empty());
                assert!(transaction.selection().is_none());
                assert!(!transaction.scroll_into_view());
                assert!(update.changes().is_empty());
                if veto.load(Relaxed) {
                    return Err(ConfigurationError::InvalidFieldData);
                }
            }
            Ok(value + 1)
        }
    });
    let filter = transaction_filter({
        let calls = calls.clone();
        move |_, transaction| {
            calls.lock().unwrap().push("filter");
            Ok(transaction)
        }
    });
    let extender = transaction_extender({
        let calls = calls.clone();
        move |_, _| {
            calls.lock().unwrap().push("extender");
            TransactionExtension::default()
        }
    });
    let events = Events::default();
    let plugin = ViewPluginType::define(|state, events: &Events| {
        assert!(state.wast_syntax_tree(3_000).is_some());
        Ok(Probe(events.clone()))
    });
    let listener = update_listener({
        let events = events.clone();
        move |update| {
            observe(&events, "listener", update);
            Ok(())
        }
    });
    let state = EditorState::with_extensions(
        "(func $f)\n".repeat(1_200),
        Extension::group([
            viewport_language(),
            field.extension(),
            filter,
            extender,
            plugin.of(events.clone()),
            listener,
        ]),
    )
    .unwrap();
    assert!(state.wast_syntax_tree(0).is_none());
    let mut view = EditorView::<()>::new(state);
    measure(&mut view);
    events.lock().unwrap().clear();
    let old = view.state().clone();
    let ticket = view.wast_idle_work().unwrap();
    let stage = view.ensure_wast_syntax_tree(5_000, 1_200).unwrap().unwrap();
    assert!(calls.lock().unwrap().is_empty());
    let commit_error = WastViewportError::Commit(ConfigurationError::InvalidFieldData.into());
    assert_eq!(view.force_wast_parsing(5_000, 0), Err(commit_error));
    assert_eq!(view.run_wast_idle(&ticket, 0), Err(commit_error));
    assert!(view.wast_parse_error().is_none());
    assert!(events.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(
        &old.wast_syntax_tree(0).unwrap(),
        &view.wast_syntax_tree(0).unwrap()
    ));
    assert_eq!(*view.state().context().field(&field).unwrap().unwrap(), 0);
    assert!(Arc::ptr_eq(
        &stage,
        &view.ensure_wast_syntax_tree(5_000, 0).unwrap().unwrap()
    ));

    veto.store(false, Relaxed);
    assert_eq!(
        view.run_wast_idle(&ticket, 50_000),
        Ok(Some(WastIdleStep {
            target: 5_000,
            rounds: 0,
            published: true,
        }))
    );
    assert!(Arc::ptr_eq(
        &stage,
        &view.state().wast_syntax_tree(5_000).unwrap()
    ));
    assert!(view.force_wast_parsing(5_000, 0).unwrap());
    assert_eq!(*calls.lock().unwrap(), ["field", "field", "field"]);
    assert_eq!(*old.context().field(&field).unwrap().unwrap(), 0);
    assert_eq!(*view.state().context().field(&field).unwrap().unwrap(), 1);
    assert_eq!(
        (view.revision(), view.undo_depth(), view.redo_depth()),
        (0, 0, 0)
    );
    assert!(view.take_scroll_request().is_none());
    assert_eq!(
        *events.lock().unwrap(),
        ["plugin", "listener"].map(|name| Report {
            name,
            from: Some(3_000),
            to: Some(5_000),
            doc_changed: false,
            effects: 0,
        })
    );
}

#[test]
fn edits_and_reconfigure_replace_the_publication_in_one_view_update() {
    let events = Events::default();
    let count = StateField::new(|_| Ok(0), |value, _| Ok(value + 1));
    let slot = Compartment::new();
    let state = EditorState::with_extensions(
        "(func $f)\n".repeat(1_200),
        Extension::group([
            slot.of(viewport_language()),
            count.extension(),
            update_listener({
                let events = events.clone();
                move |update| {
                    observe(&events, "listener", update);
                    Ok(())
                }
            }),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    view.add_plugin(Probe(events.clone()));
    measure(&mut view);
    events.lock().unwrap().clear();
    let ticket = view.wast_idle_work().unwrap();
    let old = view.state().clone();
    let stage = view.ensure_wast_syntax_tree(5_000, 1_200).unwrap().unwrap();

    let rejected = view.update(edit(")"), (), |post| {
        assert!(post.wast_syntax_tree(0).is_none());
        (Some(EditorSelection::single(99_999, 99_999)), ())
    });
    assert_eq!(rejected, Err(EditError::InvalidSelection));
    assert!(events.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(
        &old.wast_syntax_tree(0).unwrap(),
        &view.state().wast_syntax_tree(0).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &stage,
        &view.ensure_wast_syntax_tree(5_000, 0).unwrap().unwrap()
    ));
    assert_eq!(*view.state().context().field(&count).unwrap().unwrap(), 0);

    view.update(edit(")"), (), |post| {
        assert!(post.wast_syntax_tree(0).is_none());
        assert!(post.text().as_str().starts_with(')'));
        (None, ())
    })
    .unwrap();
    assert_eq!(view.run_wast_idle(&ticket, usize::MAX), Ok(None));
    assert_eq!(*view.state().context().field(&count).unwrap().unwrap(), 1);
    let replacement = wast_language().configure(None, Some("replacement"));
    let next = Transaction::default()
        .with_effects([slot.reconfigure(replacement.viewport_extension().unwrap())]);
    view.update(next, (), |post| {
        assert!(post.wast_syntax_tree(0).is_none());
        (None, ())
    })
    .unwrap();
    assert_eq!(*view.state().context().field(&count).unwrap().unwrap(), 2);
    assert_eq!(old.wast_syntax_tree(0).unwrap().parsed_to, 3_000);
    assert_eq!(
        *events.lock().unwrap(),
        [
            ("plugin", 3_000, 5_001, true, 0),
            ("listener", 3_000, 5_001, true, 0),
            ("plugin", 5_001, 3_000, false, 1),
            ("listener", 5_001, 3_000, false, 1),
        ]
        .map(|(name, from, to, doc_changed, effects)| Report {
            name,
            from: Some(from),
            to: Some(to),
            doc_changed,
            effects,
        })
    );
}

#[test]
fn headless_updates_keep_only_publications_for_the_same_text_and_parser() {
    let language = viewport_language();
    let slot = Compartment::new();
    let state = EditorState::with_extensions("(module)", slot.of(language.clone())).unwrap();
    let view = EditorView::<()>::new(state);
    let published = view.state().wast_syntax_tree(8).unwrap();
    let mut state = view.state().clone();
    let same = ChangeSet::single(Change::new(1..7, "module")).unwrap();
    state.update(Transaction::new(same)).unwrap();
    state
        .update(Transaction::default().with_effects([slot.reconfigure(language)]))
        .unwrap();
    assert!(Arc::ptr_eq(&published, &state.wast_syntax_tree(8).unwrap()));
    state.update(edit(";; prefix\n")).unwrap();
    assert!(state.wast_syntax_tree(0).is_none());
    assert!(Arc::ptr_eq(
        &published,
        &view.state().wast_syntax_tree(8).unwrap()
    ));
}

#[test]
fn publication_preserves_explicit_group_and_redo_metadata() {
    let doc = "(func $f)\n".repeat(1_200);
    let mut view =
        EditorView::new(EditorState::with_extensions(&doc[..], viewport_language()).unwrap());
    view.start_group();
    view.update(edit(";; first\n"), 10, |_| (None, 11)).unwrap();
    assert!(view.force_wast_parsing(5_000, 1_300).unwrap());
    view.update(edit(";; second\n"), 11, |_| (None, 12))
        .unwrap();
    view.end_group();
    assert_eq!(view.undo_depth(), 1);
    assert_eq!(view.undo(), Some(10));
    assert_eq!(view.state().text().as_str(), doc);
    let revision = view.revision();
    assert!(view.force_wast_parsing(doc.len(), 10_000).unwrap());
    assert_eq!(view.revision(), revision);
    assert!(attached_language_snapshot(view.state()).is_none());
    assert_eq!((view.undo_depth(), view.redo_depth()), (0, 1));
    assert_eq!(view.redo(), Some(12));
    assert!(
        view.state()
            .text()
            .as_str()
            .starts_with(";; second\n;; first\n")
    );
}

#[test]
fn viewport_data_stays_in_provider_order_and_ignores_other_providers_in_the_view_query() {
    let scope = |name| LanguageData::default().with("scope", name);
    let provider = |name| {
        let data = Arc::new(scope(name));
        language_data_provider(move |_, _, _| vec![data.clone()])
    };
    let state = EditorState::with_extensions(
        ";; 😀\r\n(func $f)",
        Extension::group([
            provider("last").with_precedence(Precedence::Low),
            viewport_language(),
            wast_language().data(scope("wast")),
            provider("first").with_precedence(Precedence::High),
        ]),
    )
    .unwrap();
    let view = EditorView::<()>::new(state);
    let data = |pos| {
        view.state()
            .language_data_at::<&str>("scope", pos, Assoc::After)
    };
    assert_eq!(data(9), ["first", "wast", "last"]);
    assert_eq!(
        data(8),
        ["first", "last"],
        "only WAST excludes the CRLF interior"
    );
    assert!(data(4).is_empty());
    let values = view.wast_viewport_data_at(9, Assoc::After);
    assert_eq!(
        values
            .iter()
            .filter_map(|d| d.get::<&str>("scope").copied())
            .collect::<Vec<_>>(),
        ["wast"]
    );
}
