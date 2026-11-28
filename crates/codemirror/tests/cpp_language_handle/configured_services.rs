use std::{collections::BTreeMap, sync::Mutex};

use gpui_editor_codemirror::{
    language::{
        brackets::{BracketMatchingOptions, bracket_matching},
        fold::{
            FoldPlaceholderConfig, code_folding_with_placeholders, fold_effect, fold_placeholder,
            fold_service,
        },
        lezer::{BRACKET_MATCHING_HANDLE, LrBracketHandle, LrFoldCallback, LrSyntaxNode},
    },
    setup::minimal_setup,
    view::keymap::KeyPlatform,
};

use super::*;

fn replay() -> impl Iterator<Item = Value> {
    include_str!("../fixtures/cpp_configured_services.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn range(row: &Value) -> Range<usize> {
    byte(&row[0])..byte(&row[1])
}

fn pair(range: Range<usize>) -> [usize; 2] {
    [range.start, range.end]
}

fn span(node: LrSyntaxNode<'_>) -> Value {
    let [start, end] = pair(node.range());
    json!([node.name(), node.node_type().unwrap().term(), start, end])
}

fn with_block(base: &CppLanguage, callback: Option<LrFoldCallback>) -> CppLanguage {
    base.with_node_props(&[FOLD.add(move |ty| (ty.term() == 126).then(|| callback.clone()))])
}

fn select(state: &mut EditorState, head: usize) {
    state
        .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
        .unwrap();
}

fn reconfigure(state: &mut EditorState, compartment: &Compartment, extension: Extension) {
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(extension)]))
        .unwrap();
}

fn make(source: &str, language: &CppLanguage) -> EditorState {
    EditorState::with_extensions(source, language.extension()).unwrap()
}

fn fold_at(state: &EditorState, line: Range<usize>) -> Option<Range<usize>> {
    foldable_in_state(state, line.start, line.end)
}

fn fold_cases() -> BTreeMap<String, Value> {
    replay()
        .filter(|row| !row["case"].as_str().unwrap().starts_with("brackets"))
        .map(|row| (row["case"].as_str().unwrap().to_owned(), row))
        .collect()
}

#[test]
fn pinned_fold_rules_read_current_state_and_preserve_retained_cpp() {
    let cases = fold_cases();
    let inputs = &cases["fold inputs"];
    let source = inputs["source"].as_str().unwrap();
    let stock = cpp_language();
    let rows = stock
        .node_types()
        .filter(|ty| ty.prop(&FOLD).is_some_and(Option::is_some))
        .map(|ty| json!([ty.term(), ty.name()]))
        .collect::<Vec<_>>();
    let expected = cases["pins"]["rows"].as_array().unwrap();
    assert_eq!(
        json!(rows),
        json!(
            expected
                .iter()
                .map(|row| [row[0].clone(), row[1].clone()])
                .collect::<Vec<_>>()
        )
    );
    assert!(stock.node_types().all(|ty| !ty.is("DeclarationList")));

    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback: LrFoldCallback = Arc::new({
        let calls = calls.clone();
        move |node, state| {
            let head = state.selection().main().head();
            calls.lock().unwrap().push(json!({
                "node": span(node), "parent": node.parent().map(span),
                "children": node.children().map(span).collect::<Vec<_>>(), "head": head,
            }));
            let inside = node.fold_inside()?;
            let step = 1 + usize::from(state.text().as_str()[..inside.end].ends_with("\r\n"));
            Some(inside.start + usize::from(head != 0)..inside.end - step)
        }
    });
    let configured = with_block(stock, Some(callback.clone()));
    let disabled = with_block(&configured, None);
    assert!(Arc::ptr_eq(
        configured
            .node_types()
            .nth(126)
            .unwrap()
            .prop(&FOLD)
            .unwrap()
            .as_ref()
            .unwrap(),
        &callback,
    ));
    assert!(matches!(
        disabled.node_types().nth(126).unwrap().prop(&FOLD),
        Some(None)
    ));
    for (row, original) in configured.node_types().zip(stock.node_types()) {
        assert_eq!(ptr::eq(row, original), row.term() != 126);
    }

    let check = |name: &str, state: &EditorState| {
        let row = &cases[name];
        let expected = &row["fold"];
        // An out-of-bounds or half-scalar result has no native projection.
        let expected = (!expected[1].is_null()).then(|| range(expected));
        assert_eq!(fold_at(state, range(&row["line"])), expected, "{name}");
        assert_eq!(
            json!(std::mem::take(&mut *calls.lock().unwrap())),
            row["calls"],
            "{name}"
        );
    };
    let language = Compartment::new();
    let services = Compartment::new();
    let initial = EditorState::with_extensions(
        source,
        Extension::group([
            language.of(configured.extension()),
            services.of(Extension::default()),
        ]),
    )
    .unwrap();
    let original = snapshot(&initial);
    check("configured", &initial);
    check("stock", &make(source, stock));
    check(
        "configured",
        &make(source, &configured.configure(Some(true), None)),
    );
    let mut selected = initial.clone();
    select(
        &mut selected,
        byte(&cases["selection only"]["calls"][0]["head"]),
    );
    assert!(Arc::ptr_eq(&snapshot(&selected), &original));
    check("selection only", &selected);
    let at = source.find('字').unwrap();
    let mut edited = selected.clone();
    edited
        .update(replace(at..at + '字'.len_utf8(), "橋橋"))
        .unwrap();
    let edited_tree = snapshot(&edited);
    assert!(!Arc::ptr_eq(&edited_tree, &original));
    check("edit", &edited);
    let mut removed = selected.clone();
    reconfigure(&mut removed, &language, disabled.extension());
    let removed_tree = snapshot(&removed);
    assert!(!Arc::ptr_eq(&removed_tree, &original));
    check("explicit removal", &removed);
    for (snapshot, handle) in [
        (&original, &configured),
        (&edited_tree, &configured),
        (&removed_tree, &disabled),
    ] {
        let doc = cpp(snapshot);
        let node = doc
            .nodes()
            .find(|node| node.node_type().term() == 126)
            .unwrap();
        assert!(ptr::eq(
            node.node_type(),
            handle.node_types().nth(126).unwrap()
        ));
        let line = range(&cases["stock"]["line"]);
        assert_eq!(doc.foldable(line.start, line.end), node.fold_range());
        assert_eq!(
            snapshot.syntax().foldable(line.start, line.end),
            node.fold_range()
        );
    }
    reconfigure(&mut removed, &language, stock.extension());
    check("restore", &removed);
    check("retained selected", &selected);
    check("retained initial", &initial);

    let order = Arc::new(Mutex::new(Vec::new()));
    let provider = |name, precedence, proposal: Option<Range<usize>>| {
        let order = order.clone();
        fold_service(move |_, _, _| {
            order.lock().unwrap().push(name);
            proposal.clone()
        })
        .with_precedence(precedence)
    };
    for (name, proposal) in [("service wins", 18..50), ("invalid first service", 18..999)] {
        let mut serviced = selected.clone();
        reconfigure(
            &mut serviced,
            &services,
            Extension::group([
                provider("low", Precedence::Low, Some(18..60)),
                provider("normal", Precedence::Default, Some(proposal)),
                provider("high", Precedence::High, None),
            ]),
        );
        assert!(Arc::ptr_eq(&snapshot(&serviced), &original));
        check(name, &serviced);
        assert_eq!(
            json!(std::mem::take(&mut *order.lock().unwrap())),
            cases[&format!("{name} order")]["order"]
        );
        reconfigure(&mut serviced, &services, Extension::default());
        check("selection only", &serviced);
    }

    let ascii = inputs["ascii"].as_str().unwrap();
    check("stock ASCII CRLF", &make(ascii, stock));
    check("removed ASCII CRLF", &make(ascii, &disabled));
    let lists = make(inputs["stockLists"].as_str().unwrap(), &configured);
    for line in [1, 4, 7] {
        check(&format!("stock lists {line}"), &lists);
    }
    for (index, comment) in inputs["comments"].as_array().unwrap().iter().enumerate() {
        check(
            &format!("comment {index}"),
            &make(comment.as_str().unwrap(), &configured),
        );
    }
}

#[test]
fn pinned_nested_fold_precedence_and_native_boundary_checks() {
    let cases = fold_cases();
    let source = cases["fold inputs"]["nested"].as_str().unwrap();
    let stock = cpp_language();
    let line = range(&cases["nested outer wins"]["line"]);
    assert_eq!(
        fold_at(&make(source, stock), line.clone()),
        Some(range(&cases["nested outer wins"]["fold"]))
    );
    let inner = range(&cases["outer returns null"]["fold"]);
    for (name, proposal) in [
        ("outer returns null", None),
        ("outer starts after line", Some(24..37)),
        ("outer ends at line end", Some(9..22)),
        ("outer past EOF", Some(9..999)),
        ("outer splits CRLF", Some(9..41)),
    ] {
        let order = Arc::new(Mutex::new(Vec::new()));
        let handle = with_block(
            stock,
            Some(Arc::new({
                let order = order.clone();
                move |node, _| {
                    let parent = node.parent()?.name();
                    order.lock().unwrap().push(parent);
                    if parent == "FunctionDefinition" {
                        proposal.clone()
                    } else {
                        node.fold_inside()
                    }
                }
            })),
        );
        let state = make(source, &handle);
        let expected = cases
            .get(name)
            .filter(|row| !row["fold"][1].is_null())
            .map_or_else(|| inner.clone(), |row| range(&row["fold"]));
        assert_eq!(fold_at(&state, line.clone()), Some(expected), "{name}");
        assert_eq!(
            json!(std::mem::take(&mut *order.lock().unwrap())),
            cases["outer returns null order"]["order"]
        );
        assert_eq!(fold_at(&state, line.start..line.end + 1), None);
        assert!(order.lock().unwrap().is_empty());
    }
    let source = cases["fold inputs"]["source"].as_str().unwrap();
    let line = range(&cases["stock"]["line"]);
    let half_scalar = source.find('😀').unwrap() + 1;
    let configured = with_block(
        stock,
        Some(Arc::new(move |node, _| {
            Some(node.fold_inside()?.start..half_scalar)
        })),
    );
    let state = make(source, &configured);
    assert_eq!(fold_at(&state, line.clone()), None);
    assert_eq!(fold_at(&state, line.start..half_scalar), None);
}

#[test]
fn removing_a_cpp_rule_does_not_remove_or_prepare_existing_folds() {
    let cases = fold_cases();
    let source = cases["fold inputs"]["source"].as_str().unwrap();
    let language = Compartment::new();
    let prepares = Arc::new(Mutex::new(0));
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([
            language.of(cpp_language().extension()),
            code_folding_with_placeholders(FoldPlaceholderConfig {
                prepare: Some(Arc::new({
                    let prepares = prepares.clone();
                    move |_, _| {
                        *prepares.lock().unwrap() += 1;
                        "stored".into()
                    }
                })),
                render: Some(Arc::new(|_, _, saved| saved.map(Arc::from))),
                ..Default::default()
            }),
        ]),
    )
    .unwrap();
    let folded = range(&cases["stock"]["fold"]);
    state
        .update(Transaction::default().with_effects([fold_effect().of(folded.clone())]))
        .unwrap();
    let retained = state.clone();
    reconfigure(
        &mut state,
        &language,
        with_block(cpp_language(), None).extension(),
    );
    assert_eq!(fold_at(&state, range(&cases["stock"]["line"])), None);
    assert_eq!(
        fold_placeholder(&state, &folded).unwrap().text.as_ref(),
        "stored"
    );
    let at = source.find('字').unwrap();
    state
        .update(replace(at..at + '字'.len_utf8(), "橋橋"))
        .unwrap();
    assert_eq!(
        fold_placeholder(&state, &(folded.start..folded.end + 3))
            .unwrap()
            .text
            .as_ref(),
        "stored"
    );
    assert_eq!(
        fold_placeholder(&retained, &folded).unwrap().text.as_ref(),
        "stored"
    );
    assert_eq!(*prepares.lock().unwrap(), 1);
}

fn bracket_language(props: &Value) -> CppLanguage {
    let list = |value: &Value| serde_json::from_value::<Vec<String>>(value.clone()).ok();
    let closed = props["closedBy"].clone();
    let opened = props["openedBy"].clone();
    let handles = props["handles"].clone();
    cpp_language().with_node_props(&[
        CLOSED_BY.add(move |ty| list(&closed[ty.term().to_string()])),
        OPENED_BY.add(move |ty| list(&opened[ty.term().to_string()])),
        BRACKET_MATCHING_HANDLE.add(move |ty| {
            let handle: LrBracketHandle = match handles[ty.term().to_string()].as_str()? {
                "second" => Arc::new(|node| node.children().nth(1)),
                "skip" => Arc::new(|_| None),
                _ => unreachable!(),
            };
            Some(Some(handle))
        }),
    ])
}

fn bracket_options(row: &Value) -> BracketMatchingOptions {
    BracketMatchingOptions {
        brackets: row["brackets"].as_str().map(Arc::from),
        max_scan_distance: serde_json::from_value(row["maxScanDistance"].clone()).unwrap(),
        ..Default::default()
    }
}

fn verify_marks(state: EditorState, expected: &Value, label: &str) {
    assert_eq!(state.selection().main().head(), byte(&expected["head"]));
    let editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    let marks = editor
        .decorations()
        .unwrap()
        .bracket_matches
        .iter(0)
        .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
        .collect::<Vec<_>>();
    assert_eq!(json!(marks), expected["marks"], "{label}");
}

#[test]
fn pinned_cpp_bracket_roles_and_handles_drive_native_marks() {
    for row in replay() {
        match row["case"].as_str().unwrap() {
            "pins" => {
                let roles = cpp_language()
                    .node_types()
                    .filter_map(|ty| {
                        assert!(ty.prop(&BRACKET_MATCHING_HANDLE).is_none());
                        let (open, close) = (ty.prop(&OPENED_BY), ty.prop(&CLOSED_BY));
                        (open.is_some() || close.is_some())
                            .then(|| json!([ty.term(), ty.name(), open, close]))
                    })
                    .collect::<Vec<_>>();
                assert_eq!(json!(roles), row["stockRoles"]);
            }
            "brackets" => {
                let source = row["source"].as_str().unwrap();
                let options = bracket_options(&row["options"]);
                let config = options.config();
                let language = bracket_language(&row["props"]);
                let state = EditorState::with_extensions(
                    source,
                    Extension::group([language.extension(), bracket_matching(options.clone())]),
                )
                .unwrap();
                let tree = snapshot(&state);
                for query in row["queries"].as_array().unwrap() {
                    let mut selected = state.clone();
                    select(&mut selected, byte(&query["head"]));
                    assert!(Arc::ptr_eq(&snapshot(&selected), &tree));
                    verify_marks(selected, query, row["label"].as_str().unwrap());
                }
                for at in (1..source.len())
                    .filter(|&at| {
                        !source.is_char_boundary(at) || source.as_bytes()[at - 1..=at] == *b"\r\n"
                    })
                    .chain([source.len() + 1])
                {
                    assert!(cpp(&tree).bracket_near_with_config(at, config).is_none());
                }
            }
            _ => (),
        }
    }
}

#[test]
fn cpp_bracket_rows_survive_edit_and_reset_only_on_the_new_snapshot() {
    let row = replay()
        .find(|row| row["case"] == "brackets retained")
        .unwrap();
    let queries = row["queries"].as_array().unwrap();
    let language = bracket_language(&row["props"]);
    let stock = cpp_language();
    let options = bracket_options(&row["options"]);
    let compartment = Compartment::new();
    let mut selected = EditorState::with_extensions(
        row["source"].as_str().unwrap(),
        Extension::group([
            compartment.of(language.extension()),
            bracket_matching(options),
        ]),
    )
    .unwrap();
    let initial_tree = snapshot(&selected);
    select(&mut selected, byte(&queries[0]["head"]));
    assert!(Arc::ptr_eq(&snapshot(&selected), &initial_tree));
    let mut edited = selected.clone();
    let edit = &row["edit"];
    edited
        .update(replace(
            byte(&edit["from"])..byte(&edit["to"]),
            edit["insert"].as_str().unwrap(),
        ))
        .unwrap();
    let edited_tree = snapshot(&edited);
    assert!(!Arc::ptr_eq(&edited_tree, &initial_tree));
    let mut reset = edited.clone();
    reconfigure(&mut reset, &compartment, stock.extension());
    let reset_tree = snapshot(&reset);
    assert!(!Arc::ptr_eq(&reset_tree, &edited_tree));
    for (tree, handle) in [
        (&initial_tree, &language),
        (&edited_tree, &language),
        (&reset_tree, stock),
    ] {
        let actual = cpp(tree)
            .nodes()
            .find(|node| node.node_type().term() == 33)
            .unwrap()
            .node_type();
        assert!(ptr::eq(actual, handle.node_types().nth(33).unwrap()));
    }
    for (state, query) in [selected.clone(), edited.clone(), reset, selected, edited]
        .into_iter()
        .zip(queries)
    {
        verify_marks(state, query, "retained angle roles");
    }
}
