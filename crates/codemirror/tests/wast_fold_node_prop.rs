use std::{
    ops::Range,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence, Text,
    Transaction,
    language::{
        fold::{
            FOLD_GUTTER_CLASS, FoldGutterConfig, fold_gutter, fold_service, foldable_in_state,
            folded_ranges,
        },
        wast::{FOLD, WastDocument, WastFoldCallback},
    },
    language_data::{
        NativeSyntax, SupportedDocument, WastLanguage, attached_language_snapshot, wast_language,
    },
    setup::{SetupEditor, SetupKeyContext, basic_setup},
    view::{
        gutters::{GutterSlot, gutter_slots},
        keymap::{KeyEvent, KeyModifiers, KeyPlatform},
    },
};
use serde_json::{Value, json};

const DOC: &str = "(module\n  (func)\n  1\n)\n(; note\nmore ;)\n\"text\"\n";
const NESTED: &str = "(module (func\n  1))\n";
type Calls = Arc<Mutex<Vec<(usize, usize)>>>;

fn oracle(name: &str) -> Value {
    include_str!("fixtures/wast_fold_node_prop.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["name"] == name)
        .unwrap()["value"]
        .take()
}

fn check(name: &str, state: &EditorState, bounds: Range<usize>, calls: &Calls) {
    let range = foldable_in_state(state, bounds.start, bounds.end).map(|r| [r.start, r.end]);
    assert_eq!(
        json!([range, std::mem::take(&mut *calls.lock().unwrap())]),
        oracle(name),
        "{name}"
    );
}

fn snapshot(state: &EditorState) -> Arc<SupportedDocument> {
    attached_language_snapshot(state).unwrap()
}

fn wast(snapshot: &SupportedDocument) -> &WastDocument {
    let NativeSyntax::Wast(doc) = snapshot.syntax() else {
        panic!("expected WAST attachment");
    };
    doc
}

fn with_app(callback: WastFoldCallback) -> WastLanguage {
    wast_language()
        .with_node_props(&[FOLD.add(move |t| t.is("App").then(|| Some(callback.clone())))])
}

#[test]
fn installed_contiguous_wast_fold_uses_mounted_callback_after_services() {
    use gpui_editor_codemirror::language::markdown::{
        MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
        markdown_with_parser,
    };
    let fixture: Value = serde_json::from_str(
        include_str!("fixtures/wast_host_commands.jsonl")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let source = fixture["source"].as_str().unwrap();
    let offset = |v: &Value| usize::try_from(v.as_u64().unwrap()).unwrap();
    let from = offset(&fixture["line"][0]);
    let to = offset(&fixture["line"][1]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let language = with_app(Arc::new(move |node, state| {
        let range = node.range();
        seen.lock()
            .unwrap()
            .push([range.start, range.end, state.selection().main().head()]);
        Some(range.start + 2..range.end - 1)
    }));
    let installed = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |name| {
                    (name == "wat").then(|| MarkdownCodeParser::Wast(language.clone()))
                }),
                fallback: None,
            }),
            ..Default::default()
        },
    );
    let head = offset(&fixture["selection"]);
    let state = EditorState::create_from_text(
        Text::new(source),
        Some(EditorSelection::single(head, head)),
        installed.clone(),
    )
    .unwrap();
    assert_eq!(
        json!(foldable_in_state(&state, from, to).map(|r| [r.start, r.end])),
        fixture["range"]
    );
    assert_eq!(
        json!(std::mem::take(&mut *calls.lock().unwrap())),
        fixture["calls"]
    );
    let service = fold_service(move |_, _, _| Some(from + 1..to + 1));
    let state =
        EditorState::with_extensions(source, Extension::group([service, installed])).unwrap();
    assert_eq!(foldable_in_state(&state, from, to), Some(from + 1..to + 1));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn scoped_folds_reject_stale_text_and_invalid_host_endpoints() {
    let source = "(module\r\n  (func)\r\n)";
    let prefix = "Prose 🧭\r\n";
    let body = prefix.len()..prefix.len() + source.len();
    let host = EditorState::new(format!("{prefix}{source}\r\nOutside"));
    let stale = EditorState::new(format!(
        "{prefix}{}\r\nOutside",
        source.replace("func", "oops")
    ));
    let first_end = body.start + "(module".len();
    for end in [body.end - 1, body.end + 1, body.end - 2] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let observed = calls.clone();
        let callback: WastFoldCallback = Arc::new(move |node, state| {
            let at = node.range().start;
            observed.lock().unwrap().push(at);
            assert_eq!(state.text().slice(at..at + 1), Some("("));
            Some(at + 2..end)
        });
        let child = EditorState::with_extensions(source, with_app(callback).extension()).unwrap();
        let snapshot = snapshot(&child);
        let doc = wast(&snapshot);
        assert_eq!(
            doc.foldable_in_scope(&stale, body.clone(), body.start, first_end),
            None
        );
        assert!(calls.lock().unwrap().is_empty());
        let actual = doc.foldable_in_scope(&host, body.clone(), body.start, first_end);
        assert_eq!(actual, (end == body.end - 1).then_some(body.start + 2..end));
        assert_eq!(*calls.lock().unwrap(), [body.start]);
    }
}

fn configured(calls: &Calls) -> (WastLanguage, WastFoldCallback) {
    let calls = calls.clone();
    let callback: WastFoldCallback = Arc::new(move |node, state| {
        assert!(node.prop(&FOLD).unwrap().is_some());
        let r = node.range();
        assert_eq!(state.text().slice(r.start..r.start + 1), Some("("));
        let head = state.selection().main().head();
        calls.lock().unwrap().push((r.start, head));
        Some(r.start + 2 + head..r.end - 2)
    });
    let language = with_app(callback.clone())
        .with_node_props(&[FOLD.add(|t| t.is("BlockComment").then_some(None))]);
    (language, callback)
}

#[test]
fn stock_and_configured_callbacks_are_typed_and_stateless_queries_stay_stock() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WastFoldCallback>();
    let calls = Calls::default();
    let (language, callback) = configured(&calls);
    let state = EditorState::with_extensions(DOC, language.extension()).unwrap();
    let stock = EditorState::with_extensions(DOC, wast_language().extension()).unwrap();
    let attached = snapshot(&state);
    let raw = language.parse(DOC).unwrap();
    let mut raw_nodes = raw.root().children();
    let mut state_nodes = wast(&attached).root().children();
    assert!(Arc::ptr_eq(
        raw_nodes
            .next()
            .unwrap()
            .prop(&FOLD)
            .unwrap()
            .as_ref()
            .unwrap(),
        &callback
    ));
    assert!(Arc::ptr_eq(
        state_nodes
            .next()
            .unwrap()
            .prop(&FOLD)
            .unwrap()
            .as_ref()
            .unwrap(),
        &callback
    ));
    assert!(raw_nodes.next().unwrap().prop(&FOLD).unwrap().is_none());
    assert!(state_nodes.next().unwrap().prop(&FOLD).unwrap().is_none());
    assert!(state_nodes.next().unwrap().prop(&FOLD).is_none());
    let stock_snapshot = snapshot(&stock);
    for node in wast(&stock_snapshot).root().children().take(2) {
        let stock_callback = node.prop(&FOLD).unwrap().as_ref().unwrap();
        assert_eq!(stock_callback(node, &stock), node.fold_range());
    }
    assert_eq!(attached.syntax().foldable(0, 7), Some(1..21));
    assert_eq!(wast(&attached).foldable(23, 30), Some(25..36));
    assert_eq!(
        wast(&attached)
            .root()
            .children()
            .nth(1)
            .unwrap()
            .fold_range(),
        Some(25..36)
    );
    check("stock app", &stock, 0..7, &calls);
    check("stock comment", &stock, 23..30, &calls);
    check("configured app", &state, 0..7, &calls);
    check("disabled comment", &state, 23..30, &calls);
    let nested_comment =
        EditorState::with_extensions("(module (; note\nmore ;)\n)\n", language.extension())
            .unwrap();
    check(
        "disabled comment with ancestor",
        &nested_comment,
        0..15,
        &calls,
    );
}

#[test]
fn candidate_order_and_state_follow_the_active_snapshot() {
    let calls = Calls::default();
    let (language, callback) = configured(&calls);
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(NESTED, slot.of(language.extension())).unwrap();
    let original = snapshot(&state);
    for (name, bounds) in [
        ("inner then outer", 0..13),
        ("inner only", 9..13),
        ("fold starts before line", 11..13),
        ("node starts at line end", 0..8),
        ("node ends at line end", 0..18),
    ] {
        check(name, &state, bounds, &calls);
    }
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot(&state), &original));
    check("same tree, new state", &state, 0..13, &calls);
    let retained = state.clone();
    let mut edited = state.clone();
    edited
        .update(Transaction::new(
            ChangeSet::single(Change::new(16..17, "10")).unwrap(),
        ))
        .unwrap();
    let edited_snapshot = snapshot(&edited);
    let node = wast(&edited_snapshot).root().children().next().unwrap();
    assert!(Arc::ptr_eq(
        node.prop(&FOLD).unwrap().as_ref().unwrap(),
        &callback
    ));
    check("same configuration after edit", &edited, 0..13, &calls);
    state
        .update(
            Transaction::default().with_effects([slot.reconfigure(wast_language().extension())]),
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&snapshot(&state), &original));
    check("reconfigured", &state, 0..13, &calls);
    check("retained state", &retained, 0..13, &calls);

    let log = calls.clone();
    let skip_inner = with_app(Arc::new(move |node, state| {
        let from = node.range().start;
        log.lock()
            .unwrap()
            .push((from, state.selection().main().head()));
        (from == 0).then_some(2..17)
    }));
    let state = EditorState::with_extensions(NESTED, skip_inner.extension()).unwrap();
    check("callback skips inner", &state, 0..13, &calls);
    let crosses_node = with_app(Arc::new(|node, _| {
        (node.range().start == 8).then_some(13..19)
    }));
    let state = EditorState::with_extensions(NESTED, crosses_node.extension()).unwrap();
    check(
        "range starts at line end and crosses node end",
        &state,
        13..13,
        &calls,
    );
}

#[test]
fn invalid_node_ranges_are_ignored_but_invalid_services_do_not_defer() {
    let calls = Calls::default();
    let (language, _) = configured(&calls);
    let invalid_service = fold_service(|_, _, _| Some(2..100)).with_precedence(Precedence::High);
    let valid_service = fold_service(|_, _, _| Some(4..16));
    let state = EditorState::with_extensions(
        NESTED,
        Extension::group([language.extension(), valid_service.clone()]),
    )
    .unwrap();
    check("service wins", &state, 0..13, &calls);
    let state = EditorState::with_extensions(
        NESTED,
        Extension::group([language.extension(), valid_service, invalid_service]),
    )
    .unwrap();
    assert_eq!(foldable_in_state(&state, 0, 13), None);
    assert!(calls.lock().unwrap().is_empty());

    assert_eq!(oracle("upstream accepts past EOF"), json!([[2, 100], []]));
    let log = calls.clone();
    let too_far = with_app(Arc::new(move |node, state| {
        let r = node.range();
        log.lock()
            .unwrap()
            .push((r.start, state.selection().main().head()));
        if r.start == 0 {
            Some(2..100)
        } else {
            Some(r.start + 2..r.end - 2)
        }
    }));
    let state = EditorState::with_extensions(NESTED, too_far.extension()).unwrap();
    assert_eq!(foldable_in_state(&state, 0, 13), Some(10..16));
    assert_eq!(
        std::mem::take(&mut *calls.lock().unwrap()),
        [(8, 0), (0, 0)]
    );
    assert_eq!(
        oracle("upstream queries reversed bounds"),
        json!([null, [[8, 0], [0, 0]]])
    );
    assert_eq!(foldable_in_state(&state, 14, 13), None);
    assert!(calls.lock().unwrap().is_empty());

    assert_eq!(
        oracle("upstream accepts split surrogate"),
        json!([[10, 16], []])
    );
    let log = calls.clone();
    let split = with_app(Arc::new(move |node, state| {
        let r = node.range();
        log.lock()
            .unwrap()
            .push((r.start, state.selection().main().head()));
        Some(10..r.end - 1)
    }));
    let unicode = "(module \"😀\"\n  1)\n";
    let state = EditorState::with_extensions(unicode, split.extension()).unwrap();
    assert_eq!(foldable_in_state(&state, 0, 14), None); // 10 is inside the UTF-8 character.
    assert_eq!(std::mem::take(&mut *calls.lock().unwrap()), [(0, 0)]);

    let state = EditorState::with_extensions(
        unicode,
        Extension::group([
            split.extension(),
            fold_service(|_, _, _| panic!("invalid query reached a service")),
        ]),
    )
    .unwrap();
    for (start, end) in [(15, 14), (0, 10), (10, 14), (0, unicode.len() + 1)] {
        assert_eq!(foldable_in_state(&state, start, end), None);
    }
    let stop_at_eof: WastFoldCallback = Arc::new(|_, _| panic!("node ends at EOF"));
    let root = wast_language()
        .with_node_props(&[FOLD.add(move |t| t.is_top().then(|| Some(stop_at_eof.clone())))]);
    let state = EditorState::with_extensions(NESTED, root.extension()).unwrap();
    assert_eq!(foldable_in_state(&state, NESTED.len(), NESTED.len()), None);
}

fn open_lines(slots: &[GutterSlot]) -> Vec<usize> {
    slots
        .iter()
        .find(|slot| slot.class == FOLD_GUTTER_CLASS)
        .unwrap()
        .rows
        .iter()
        .filter(|row| {
            row.markers
                .iter()
                .any(|m| m.content.as_deref() == Some("⌄"))
        })
        .map(|row| row.line_number)
        .collect()
}

fn folds(editor: &SetupEditor<()>) -> Vec<(usize, usize)> {
    folded_ranges(editor.view().state())
        .iter(0)
        .map(|r| (r.from, r.to))
        .collect()
}

#[test]
fn setup_and_gutters_use_effective_callbacks_and_current_overrides() {
    let calls = Calls::default();
    let (language, _) = configured(&calls);
    // The logical line projection still uses the original byte offsets.
    let doc = NESTED.replace('\n', "\r");
    let state = EditorState::with_extensions(&doc, language.extension()).unwrap();
    let configured_snapshot = snapshot(&state);
    let disabled = wast_language().with_node_props(&[FOLD.add(|t| t.is("App").then_some(None))]);
    let disabled_state = EditorState::with_extensions(&doc, disabled.extension()).unwrap();
    let disabled_snapshot = snapshot(&disabled_state);
    let disabled = disabled_snapshot.syntax();
    let stale = NativeSyntax::Wast(WastDocument::parse(Text::new("")));
    let stock = NativeSyntax::Wast(WastDocument::parse(state.text().clone()));
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    assert_eq!(open_lines(&editor.gutters(1..=3, None).unwrap()), [1]);
    assert!(open_lines(&editor.gutters(1..=3, Some(disabled)).unwrap()).is_empty());
    assert_eq!(
        open_lines(&editor.gutters(1..=3, Some(&stale)).unwrap()),
        [1]
    );
    assert!(
        !editor
            .toggle_line_fold(1, Some(disabled), Duration::ZERO, ())
            .unwrap()
    );
    assert!(
        editor
            .toggle_line_fold(1, None, Duration::ZERO, ())
            .unwrap()
    );
    assert_eq!(folds(&editor), [(2, 17)]);
    assert!(
        editor
            .toggle_line_fold(1, Some(disabled), Duration::ZERO, ())
            .unwrap()
    );
    assert!(folds(&editor).is_empty());

    let fold_key = |editor: &mut SetupEditor<()>, syntax, all: bool| {
        let modifiers = KeyModifiers {
            control: true,
            alt: all,
            shift: !all,
            ..Default::default()
        };
        let event = if all {
            KeyEvent::new("[", modifiers)
        } else {
            KeyEvent {
                base_name: Some("["),
                ..KeyEvent::new("{", modifiers)
            }
        };
        editor
            .key_event_with(
                event,
                Duration::ZERO,
                (),
                SetupKeyContext {
                    syntax,
                    ..Default::default()
                },
            )
            .unwrap()
            .key
            .handled
    };
    assert!(!fold_key(&mut editor, Some(disabled), true));
    assert!(fold_key(&mut editor, None, false));
    assert_eq!(folds(&editor), [(2, 17)]);
    assert!(
        editor
            .toggle_line_fold(1, None, Duration::ZERO, ())
            .unwrap()
    );
    assert!(fold_key(&mut editor, Some(&stock), true));
    assert_eq!(folds(&editor), [(1, 18)]);

    let mut editor = basic_setup::<()>(disabled_state, KeyPlatform::Linux).unwrap();
    editor
        .view_mut()
        .update(
            Transaction::default().with_selection(EditorSelection::single(1, 1)),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert!(open_lines(&editor.gutters(1..=3, None).unwrap()).is_empty());
    assert!(fold_key(
        &mut editor,
        Some(configured_snapshot.syntax()),
        true
    ));
    assert_eq!(folds(&editor), [(3, 17)]); // Explicit syntax, but the current editor's selection.

    let state = EditorState::with_extensions(
        NESTED,
        Extension::group([
            language.extension(),
            fold_gutter(FoldGutterConfig::default()),
        ]),
    )
    .unwrap();
    assert_eq!(open_lines(&gutter_slots(&state, 1..=3).unwrap()), [1]);
}
