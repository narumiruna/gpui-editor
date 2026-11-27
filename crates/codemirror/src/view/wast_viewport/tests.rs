use std::sync::Arc;

use serde_json::Value;

use crate::{
    Assoc, Change, ChangeSet, Compartment, EditError, EditorSelection, EditorState, Extension,
    Text, Transaction,
    language::{
        HighlightTag, LanguageData,
        lezer::{ParseError, SyntaxNode, SyntaxTree},
        wast::{
            GROUP, NodeProp, SUBLANGUAGES, WastDataFacet, WastNodeKind, WastParseJob,
            WastSublanguage, WastSublanguageMode, style_tags,
        },
    },
    language_data::{WastLanguage, attached_language_snapshot, wast_language},
    view::{EditorView, Viewport, WastIdleStep, WastIdleWork, WastViewportError},
};

fn audit(event: &str) -> Value {
    include_str!("../../../tests/fixtures/wast_viewport_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["event"] == event)
        .unwrap()
}

fn number(row: &Value, name: &str) -> usize {
    row[name].as_u64().unwrap().try_into().unwrap()
}

fn view(text: &str) -> (EditorView<()>, Compartment) {
    let slot = Compartment::new();
    let state =
        EditorState::with_extensions(text, slot.of(wast_language().viewport_extension().unwrap()))
            .unwrap();
    (EditorView::new(state), slot)
}

fn report(view: &mut EditorView<()>, row: &Value, from: usize) {
    let range = from..number(row, "vp");
    report_range(view, range);
}

fn report_range(view: &mut EditorView<()>, range: std::ops::Range<usize>) {
    let vp = Viewport::new(view.state().text(), range.clone(), [range]).unwrap();
    view.set_viewport_geometry(vp).unwrap();
}

fn published(view: &EditorView<()>, row: &Value) {
    let output = view.wast_syntax_tree(0).unwrap();
    let through = number(row, "through");
    assert_eq!(output.parsed_to, through, "{}", row["event"]);
    assert_eq!(output.tree.root().range().end, number(row, "pub"));
    assert_eq!(output.complete, row["full"]);
    assert_eq!(
        view.wast_syntax_tree(number(row, "vp")).is_some(),
        row["visible"]
    );
    assert!(view.wast_syntax_tree(through).is_some());
    assert!(view.wast_syntax_tree(through + 1).is_none());
}

fn pending_rounds(view: &mut EditorView<()>, ticket: &WastIdleWork, target: usize, rounds: usize) {
    let mut left = rounds;
    while left > 0 {
        let quota = left.min(2_048);
        assert_eq!(
            view.run_wast_idle(ticket, quota).unwrap(),
            Some(WastIdleStep {
                target,
                rounds: quota,
                published: false,
            })
        );
        left -= quota;
    }
}

fn edit(range: std::ops::Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

fn dispatch(view: &mut EditorView<()>, transaction: Transaction) {
    view.update(transaction, (), |_| (None, ())).unwrap();
}

fn first_identifier(tree: &SyntaxTree) -> SyntaxNode<'_> {
    tree.root()
        .children()
        .find(|node| node.name() == "App")
        .unwrap()
        .children()
        .find(|node| node.name() == "Identifier")
        .unwrap()
}

#[test]
fn configured_type_rows_survive_viewport_staging_and_reconfigure_atomically() {
    let marker = NodeProp::<&str>::new();
    let configure = |value| {
        let key = marker.clone();
        wast_language()
            .with_node_props(&[key.add(move |node| (node.name() == "Identifier").then_some(value))])
    };
    let first = configure("first");
    let second = configure("second");
    let source = "(func $f)\n".repeat(12_000);
    let slot = Compartment::new();
    let state = EditorState::with_extensions(
        source.as_str(),
        slot.of(first.viewport_extension().unwrap()),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    let installed = audit("propsInstall");
    report(&mut view, &installed, 0);
    published(&view, &installed);
    let initial = view.wast_syntax_tree(0).unwrap();
    let node = first_identifier(&initial.tree);
    let ty = node.wast_node_type().unwrap();
    let raw = first.parse("(func $f)").unwrap();
    let raw_type = first_identifier(&raw).wast_node_type().unwrap();
    assert_eq!(serde_json::json!(node.prop(&marker)), installed["marker"]);
    assert_eq!(std::ptr::eq(ty, raw_type), installed["sameTypeAsRaw"]);

    let staged = audit("propsEnsure");
    let through = number(&staged, "through");
    let working = view
        .ensure_wast_syntax_tree(through, 1_200)
        .unwrap()
        .unwrap();
    let working_node = first_identifier(&working.tree);
    assert_eq!(
        serde_json::json!(working_node.prop(&marker)),
        staged["marker"]
    );
    assert_eq!(
        std::ptr::eq(ty, working_node.wast_node_type().unwrap()),
        staged["sameType"]
    );
    assert_eq!(
        Arc::ptr_eq(&initial, &view.wast_syntax_tree(0).unwrap()),
        staged["samePublished"]
    );
    assert_eq!(working.tree.root().range().end, number(&staged, "working"));

    dispatch(
        &mut view,
        Transaction::default()
            .with_effects([slot.reconfigure(second.viewport_extension().unwrap())]),
    );
    let replacement = audit("propsReconfigure");
    published(&view, &replacement);
    let published_tree = view.wast_syntax_tree(0).unwrap();
    let published_node = first_identifier(&published_tree.tree);
    assert_eq!(
        serde_json::json!(published_node.prop(&marker)),
        replacement["marker"]
    );
    assert_eq!(
        serde_json::json!(working_node.prop(&marker)),
        replacement["retainedMarker"]
    );
    assert_eq!(
        std::ptr::eq(ty, published_node.wast_node_type().unwrap()),
        replacement["sameType"]
    );
    assert!(attached_language_snapshot(view.state()).is_none());
    assert_eq!((view.revision(), view.undo_depth()), (0, 0));
}

#[test]
fn viewport_styles_use_published_context_and_clip_to_valid_bytes() {
    let language = wast_language().with_node_props(&[style_tags([
        ("App/...", vec![HighlightTag::new("emphasis")]),
        ("App/Keyword", vec![HighlightTag::new("atom")]),
        ("App/Identifier!", vec![HighlightTag::new("string")]),
    ])
    .unwrap()]);
    let slot = Compartment::new();
    let source = "(func $f)\n".repeat(12_000);
    let state = EditorState::with_extensions(
        source.as_str(),
        slot.of(language.viewport_extension().unwrap()),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    let range = |row: &Value| {
        row["range"][0].as_u64().unwrap() as usize..row["range"][1].as_u64().unwrap() as usize
    };
    let spans = |view: &EditorView<()>, row: &Value| {
        view.wast_viewport_highlights(range(row)).map(|runs| {
            runs.map(|span| {
                serde_json::json!([
                    span.range.start,
                    span.range.end,
                    span.tags.iter().map(HighlightTag::name).collect::<Vec<_>>()
                ])
            })
            .collect::<Vec<_>>()
        })
    };
    let install = audit("stylesInstall");
    assert_eq!(serde_json::json!(spans(&view, &install)), install["spans"]);
    let ensure = audit("stylesEnsure");
    let target = number(&ensure, "through");
    assert!(
        view.ensure_wast_syntax_tree(target, 1_200)
            .unwrap()
            .is_some()
    );
    assert_eq!(ensure["spans"], serde_json::json!([]));
    assert!(spans(&view, &ensure).is_none());
    assert!(
        view.wast_viewport_highlights(3_000..3_001).is_none(),
        "token overrun is not coverage"
    );
    assert!(view.force_wast_parsing(target, 0).unwrap());
    let force = audit("stylesForce");
    assert_eq!(serde_json::json!(spans(&view, &force)), force["spans"]);
    assert_eq!(view.wast_viewport_highlights(0..0).unwrap().count(), 0);
    let revision = view.revision();
    dispatch(
        &mut view,
        Transaction::default()
            .with_effects([slot.reconfigure(wast_language().viewport_extension().unwrap())]),
    );
    assert!(spans(&view, &force).is_none());
    let tags = view
        .wast_viewport_highlights(1..5)
        .unwrap()
        .next()
        .unwrap()
        .tags;
    assert_eq!(tags, &[HighlightTag::new("keyword")]);
    assert_eq!(view.revision(), revision);

    let (unicode, _) = self::view("(data \"😀\")");
    assert!(unicode.wast_viewport_highlights(8..9).is_none());
    assert!(unicode.wast_viewport_highlights(7..11).is_some());
}

#[test]
fn viewport_data_uses_published_nodes_and_current_facets() {
    use WastNodeKind::{Identifier, Keyword, Module, OpenParen};
    use WastSublanguageMode::{Extend, Replace};

    let scope = |value| LanguageData::default().with("scope", value);
    let rules = [
        (Keyword, Extend, "keyword"),
        (Identifier, Replace, "identifier"),
        (OpenParen, Replace, "opener"),
        (Module, Extend, "module"),
    ]
    .map(|(kind, mode, label)| {
        WastSublanguage::new(
            mode,
            WastDataFacet::new(Some(scope(label))),
            move |node, _| {
                assert_eq!(node.parent().is_none(), node.kind() == Module);
                node.kind() == kind
            },
        )
    });
    let language = wast_language().with_node_props(&[
        SUBLANGUAGES.add(move |ty| (ty.name() == "Module").then(|| rules.to_vec()))
    ]);
    let slot = Compartment::new();
    let data_slot = Compartment::new();
    let state = EditorState::with_extensions(
        "(func $f)\n".repeat(12_000),
        Extension::group([
            slot.of(language.viewport_extension().unwrap()),
            data_slot.of(language.data(scope("root"))),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    let install = audit("dataInstall");
    let probes = install["probes"].as_array().unwrap();
    let data = |view: &EditorView<()>, position, side| {
        view.wast_viewport_data_at(position, side)
            .iter()
            .filter_map(|value| value.get::<&str>("scope").copied())
            .collect::<Vec<_>>()
    };
    let read = |state: &EditorState| {
        serde_json::json!(
            probes
                .iter()
                .map(|row| {
                    let side = if row[1] == -1 {
                        Assoc::Before
                    } else {
                        Assoc::After
                    };
                    state.language_data_at::<&str>("scope", row[0].as_u64().unwrap() as usize, side)
                })
                .collect::<Vec<_>>()
        )
    };
    let initial = view.state().clone();
    assert!(!initial.wast_publication.as_ref().unwrap().has_data_nodes());
    assert_eq!(read(view.state()), install["data"]);
    assert!(initial.wast_publication.as_ref().unwrap().has_data_nodes());
    let ensure = audit("dataEnsure");
    let target = number(&ensure, "through");
    assert!(
        view.ensure_wast_syntax_tree(target, 1_200)
            .unwrap()
            .is_some()
    );
    assert_eq!(read(view.state()), ensure["data"]);
    assert!(view.force_wast_parsing(target, 0).unwrap());
    let force = audit("dataForce");
    assert_eq!(read(view.state()), force["data"]);
    assert_eq!(read(&initial), force["retained"]);
    assert_eq!(
        initial.wast_syntax_tree(0).unwrap().tree.root().range().end,
        number(&install, "pub")
    );
    assert_eq!((view.revision(), view.undo_depth()), (0, 0));
    assert_eq!(data(&view, 3_002, Assoc::After), ["keyword", "root"]);

    let published = view.wast_syntax_tree(0).unwrap();
    dispatch(
        &mut view,
        Transaction::default()
            .with_effects([data_slot.reconfigure(language.data(scope("current")))]),
    );
    assert_eq!(data(&view, 3_002, Assoc::After), ["keyword", "current"]);
    assert_eq!(
        view.state()
            .language_data_at::<&str>("scope", 3_002, Assoc::After),
        ["keyword", "current"]
    );
    assert!(Arc::ptr_eq(&published, &view.wast_syntax_tree(0).unwrap()));

    dispatch(
        &mut view,
        Transaction::default().with_effects([slot.reconfigure(Extension::group([]))]),
    );
    let detach = audit("dataDetach");
    assert_eq!(read(view.state()), detach["data"]);
    assert_eq!(read(&initial), detach["retained"]);

    let unicode =
        EditorState::with_extensions(";; 😀\r\n(func $f)", language.viewport_extension().unwrap())
            .unwrap();
    view.set_state(unicode);
    for invalid in [4, 8, view.state().text().len() + 1] {
        assert!(view.wast_viewport_data_at(invalid, Assoc::After).is_empty());
        assert!(
            view.state()
                .language_data_at::<&str>("scope", invalid, Assoc::After)
                .is_empty()
        );
    }
    assert!(
        !view
            .state()
            .wast_publication
            .as_ref()
            .unwrap()
            .has_data_nodes()
    );
    assert_eq!(data(&view, 11, Assoc::After), ["keyword"]);
}

#[test]
fn cold_job_eof_has_stock_types_and_original_ranges() {
    let text = ";; 🦀\r\n(func $f)";
    let mut job = WastParseJob::new(Text::new(text));
    let result = job.advance(text.len(), 128).unwrap().output.unwrap();
    assert!(result.complete);
    let node = first_identifier(&result.tree);
    assert_eq!(&text[node.range()], "$f");
    assert_eq!(node.prop(&GROUP).unwrap(), &["Expression"]);
}

#[test]
fn browser_install_ensure_force_and_idle_keep_old_publications() {
    let source = "(func $f)\n".repeat(12_000);
    let (mut view, _) = view(&source);
    let install = audit("install");
    assert_eq!(view.state().text().len(), number(&install, "length"));
    view.set_focus(true, ()).unwrap();
    assert!(
        view.wast_idle_work().is_none(),
        "wait for measured geometry"
    );
    report(&mut view, &install, 0);
    published(&view, &install);
    let initial = view.wast_syntax_tree(0).unwrap();
    let state = view.state().clone();
    let ensure = audit("ensure");
    let end = number(&ensure, "through");
    assert!(view.ensure_wast_syntax_tree(end, 1_199).unwrap().is_none());
    assert!(!view.wast_syntax_tree_available(end));
    let staged = view.ensure_wast_syntax_tree(end, 1).unwrap().unwrap();
    assert_eq!(staged.parsed_to, end);
    assert_eq!(staged.tree.root().range().end, number(&ensure, "returned"));
    assert_eq!(
        view.wast_syntax_tree_available(end),
        ensure["available5000"]
    );
    assert_eq!(
        view.wast_syntax_tree_available(end + 1),
        ensure["available5001"]
    );
    assert!(view.wast_syntax_tree(end).is_none());
    assert!(Arc::ptr_eq(&initial, &view.wast_syntax_tree(0).unwrap()));
    assert!(view.force_wast_parsing(end, 0).unwrap());
    published(&view, &audit("force"));
    assert!(Arc::ptr_eq(&staged, &view.wast_syntax_tree(0).unwrap()));
    assert_eq!(initial.tree.root().range(), 0..3_001);

    let idle = audit("idle");
    let ticket = view.wast_idle_work().unwrap();
    let target = number(&idle, "through");
    pending_rounds(&mut view, &ticket, target, 57_214);
    assert!(Arc::ptr_eq(&staged, &view.wast_syntax_tree(0).unwrap()));
    assert_eq!(
        view.run_wast_idle(&ticket, 1).unwrap(),
        Some(WastIdleStep {
            target,
            rounds: 1,
            published: true,
        })
    );
    published(&view, &idle);
    assert!(view.wast_idle_work().is_none());
    assert_eq!(view.run_wast_idle(&ticket, 1), Ok(None));
    assert_eq!(view.state().text(), state.text());
    assert_eq!(view.state().selection(), state.selection());
    assert_eq!(
        (view.revision(), view.undo_depth(), view.redo_depth()),
        (0, 0, 0)
    );
    assert!(!view.can_undo_selection());
    assert_eq!(staged.tree.root().range(), 0..5_001);
    assert!(attached_language_snapshot(view.state()).is_none());
}

#[test]
fn geometry_jump_uses_the_current_viewport_before_look_ahead() {
    let (mut view, _) = view(&"(func $f)\n".repeat(12_000));
    report(&mut view, &audit("install"), 0);
    // Queue before geometry changes. Delivery uses the latest measured range.
    let ticket = view.wast_idle_work().unwrap();
    let jump = audit("jump");
    report(&mut view, &jump, 110_000);
    published(&view, &jump);
    let blocked = audit("idleBlocked");
    let target = number(&blocked, "vp");
    assert_eq!(
        view.run_wast_idle(&ticket, 0).unwrap(),
        Some(WastIdleStep {
            target,
            rounds: 0,
            published: false,
        })
    );
    published(&view, &blocked);
    pending_rounds(&mut view, &ticket, target, 64_414);
    published(&view, &jump);
    assert_eq!(
        view.run_wast_idle(&ticket, 1).unwrap(),
        Some(WastIdleStep {
            target,
            rounds: 1,
            published: true,
        })
    );
    published(&view, &audit("idleViewport"));
    let ahead = audit("idleAhead");
    let target = number(&ahead, "length");
    pending_rounds(&mut view, &ticket, target, 5_784);
    assert!(view.wast_syntax_tree(target).is_none());
    assert_eq!(
        view.run_wast_idle(&ticket, 1).unwrap(),
        Some(WastIdleStep {
            target,
            rounds: 1,
            published: true,
        })
    );
    published(&view, &ahead);
    assert!(attached_language_snapshot(view.state()).is_none());
    assert!(view.wast_idle_work().is_none());
    assert_eq!(view.revision(), 0);
}

#[test]
fn accepted_edit_attachment_and_reset_reject_tickets_and_staged_output() {
    let source = "(func $f)\n".repeat(12_000);
    let (mut view, slot) = view(&source);
    report(&mut view, &audit("install"), 0);
    let initial = view.wast_syntax_tree(0).unwrap();
    let edit_work = view.wast_idle_work().unwrap();
    let edit_stage = view.ensure_wast_syntax_tree(5_000, 1_200).unwrap().unwrap();
    let rejected = view.update_checked(
        edit(0..0, ")"),
        (),
        |_| Err(EditError::InvalidRange),
        |_| (None, ()),
    );
    assert_eq!(rejected, Err(EditError::InvalidRange));
    assert_eq!(
        view.run_wast_idle(&edit_work, 0).unwrap().unwrap().rounds,
        0
    );
    assert!(Arc::ptr_eq(&initial, &view.wast_syntax_tree(0).unwrap()));

    dispatch(&mut view, edit(0..0, ")"));
    let edited = view.wast_syntax_tree(0).unwrap();
    let browser_edit = audit("edit");
    assert_eq!(view.state().text().len(), number(&browser_edit, "length"));
    assert_eq!(view.viewport().interval().end, number(&browser_edit, "vp"));
    assert_eq!(
        edited.tree.root().children().next().unwrap().is_error(),
        browser_edit["error0"]
    );
    // Reparse to the mapped staged coverage, not just the published prefix.
    assert_eq!(edited.parsed_to, edit_stage.parsed_to + 1);
    assert_eq!(view.run_wast_idle(&edit_work, usize::MAX), Ok(None));
    assert!(!view.publish_wast_stage(edit_stage).unwrap());
    assert_eq!(initial.tree.root().range(), 0..3_001);

    let attachment_work = view.wast_idle_work().unwrap();
    let attachment_stage = view.ensure_wast_syntax_tree(5_000, 2_000).unwrap().unwrap();
    dispatch(
        &mut view,
        Transaction::default().with_selection(EditorSelection::single(1, 1)),
    );
    assert!(Arc::ptr_eq(&edited, &view.wast_syntax_tree(0).unwrap()));
    assert!(Arc::ptr_eq(
        &attachment_stage,
        &view.ensure_wast_syntax_tree(5_000, 0).unwrap().unwrap()
    ));
    let depths = (
        view.undo_depth(),
        view.redo_depth(),
        view.can_undo_selection(),
    );
    let revision = view.revision();
    assert!(view.force_wast_parsing(5_000, 0).unwrap());
    assert_eq!(
        (
            view.undo_depth(),
            view.redo_depth(),
            view.can_undo_selection()
        ),
        depths
    );
    assert_eq!(view.state().selection(), &EditorSelection::single(1, 1));
    assert_eq!(view.revision(), revision);

    let other = wast_language().configure(None, Some("other"));
    dispatch(
        &mut view,
        Transaction::default()
            .with_effects([slot.reconfigure(other.viewport_extension().unwrap())]),
    );
    assert_eq!(
        view.revision(),
        revision,
        "attachment identity is independent of text revision"
    );
    assert_eq!(view.run_wast_idle(&attachment_work, usize::MAX), Ok(None));
    assert!(!view.publish_wast_stage(attachment_stage).unwrap());
    let detach_work = view.wast_idle_work().unwrap();
    dispatch(
        &mut view,
        Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
    );
    assert!(view.wast_syntax_tree(0).is_none());
    assert!(
        view.ensure_wast_syntax_tree(5_000, 20_000)
            .unwrap()
            .is_none()
    );
    assert!(view.wast_idle_work().is_none());
    assert_eq!(view.run_wast_idle(&detach_work, usize::MAX), Ok(None));

    dispatch(
        &mut view,
        Transaction::default()
            .with_effects([slot.reconfigure(other.viewport_extension().unwrap())]),
    );
    assert_eq!(view.run_wast_idle(&detach_work, 1), Ok(None));
    let reset_work = view.wast_idle_work().unwrap();
    let reset_stage = view.ensure_wast_syntax_tree(5_000, 2_000).unwrap().unwrap();
    view.set_state(view.state().clone());
    assert!(view.wast_idle_work().is_none());
    report(&mut view, &audit("install"), 0);
    assert_eq!(view.run_wast_idle(&reset_work, 1), Ok(None));
    assert!(!view.publish_wast_stage(reset_stage).unwrap());
    assert!(view.wast_syntax_tree(5_000).is_none());
    assert_eq!((view.undo_depth(), view.redo_depth()), (0, 0));
}

#[test]
fn edit_restores_staged_coverage_with_raw_offsets() {
    let source = "(func $f)\n".repeat(12_000);
    let (mut view, _) = view(&source);
    report(&mut view, &audit("install"), 0);
    let stage = view.ensure_wast_syntax_tree(5_000, 1_200).unwrap().unwrap();
    assert!(view.wast_syntax_tree(5_000).is_none());
    let row = audit("editAfterEnsure");
    assert_eq!(stage.tree.root().range().end, number(&row, "retainedStage"));

    // The insert occupies three more raw bytes than normalized UTF-16 units.
    let offset_delta = 3;
    dispatch(&mut view, edit(0..0, ";; 😀\r\n"));
    let through = number(&row, "through") + offset_delta;
    let restored = view.wast_syntax_tree(through).unwrap();
    assert_eq!(
        view.state().text().len(),
        number(&row, "length") + offset_delta
    );
    assert_eq!(
        view.viewport().interval().end,
        number(&row, "vp") + offset_delta
    );
    assert_eq!(restored.parsed_to, through);
    assert_eq!(
        restored.tree.root().range().end,
        number(&row, "pub") + offset_delta
    );
    assert_eq!(stage.parsed_to, 5_000);

    dispatch(&mut view, edit(through..through, ";; x\n"));
    let row = audit("editAtWorkingEnd");
    let restored = view.wast_syntax_tree(through).unwrap();
    assert_eq!(restored.parsed_to, number(&row, "through") + offset_delta);
    assert_eq!(
        restored.tree.root().range().end,
        number(&row, "pub") + offset_delta
    );
    assert!(view.wast_syntax_tree(through + 1).is_none());
}

#[test]
fn edit_restores_ahead_of_the_stage_to_the_mapped_viewport_or_eof() {
    let source = "(func $f)\n".repeat(500);
    let (mut long, _) = view(&source);
    assert!(long.wast_syntax_tree_available(3_000));
    assert!(!long.wast_syntax_tree_available(4_500));
    report_range(&mut long, 4_450..4_500);
    dispatch(&mut long, edit(4_500..4_500, ";; x\n"));
    assert_eq!(long.viewport().interval().end, 4_505);
    assert_eq!(long.wast_syntax_tree(4_505).unwrap().parsed_to, 4_505);
    assert!(!long.wast_syntax_tree_available(4_506));

    let short = &source[..2_500];
    let (mut entire, _) = view(short);
    assert!(entire.wast_syntax_tree(short.len()).unwrap().complete);
    dispatch(&mut entire, edit(2_500..2_500, &source[..550]));
    assert!(entire.wast_syntax_tree_available(3_050));
    assert!(entire.wast_syntax_tree(3_050).unwrap().complete);
    assert!(entire.wast_idle_work().is_none());
}

#[test]
fn idle_restores_a_prefix_that_exceeds_the_edit_quota() {
    let (mut view, _) = view(&"(func $f)\n".repeat(12_000));
    report(&mut view, &audit("install"), 0);
    let before = audit("idle");
    assert!(
        view.force_wast_parsing(number(&before, "through"), 60_000)
            .unwrap()
    );
    let old = view.wast_syntax_tree(0).unwrap();
    dispatch(&mut view, edit(0..0, ")"));
    let edit_row = audit("edit");
    let target = number(&edit_row, "through");
    assert_eq!(view.wast_syntax_tree(0).unwrap().parsed_to, 3_000);
    assert!(!view.wast_syntax_tree_available(target));
    let ticket = view.wast_idle_work().unwrap();
    let mut published = false;
    for _ in 0..60 {
        let step = view.run_wast_idle(&ticket, 2_048).unwrap().unwrap();
        assert_eq!(step.target, target);
        assert!(step.rounds <= 2_048);
        if step.published {
            published = true;
            break;
        }
        assert!(view.wast_syntax_tree(target).is_none());
    }
    assert!(published);
    let output = view.wast_syntax_tree(target).unwrap();
    assert_eq!(output.parsed_to, target);
    assert_eq!(output.tree.root().range().end, number(&edit_row, "pub"));
    assert!(output.tree.root().children().next().unwrap().is_error());
    assert_eq!(old.parsed_to, number(&before, "through"));
    assert!(view.wast_idle_work().is_none());
    assert_eq!((view.revision(), view.undo_depth()), (1, 1));
}

#[test]
fn normalized_start_rounds_an_astral_cutoff_and_reindexes_a_crlf_join() {
    let source = format!(
        "(func $f)\r \n{}      \"😀\"{}",
        "(func $f)\r\n".repeat(298),
        "\n(func $f)".repeat(500)
    );
    let astral = source.find('😀').unwrap();
    let (mut view, _) = view(&source);
    let initial = view.wast_syntax_tree(0).unwrap();
    assert_eq!(initial.parsed_to, astral + 4);
    assert_eq!(initial.tree.root().range().end, astral + 5);
    assert!(view.wast_syntax_tree(astral + 1).is_none());
    assert!(!view.wast_syntax_tree_available(astral + 1));
    assert!(view.wast_syntax_tree(astral + 5).is_none());
    report_range(&mut view, 0..9);
    let ticket = view.wast_idle_work().unwrap();
    dispatch(&mut view, edit(10..11, ""));
    assert_eq!(&view.state().text().as_str()[..11], "(func $f)\r\n");
    assert_eq!(view.wast_syntax_tree(0).unwrap().parsed_to, astral + 3);
    assert_eq!(
        view.wast_syntax_tree(0).unwrap().tree.root().range().end,
        astral + 4
    );
    assert_eq!(view.run_wast_idle(&ticket, usize::MAX), Ok(None));
    assert_eq!(initial.parsed_to, astral + 4);
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.state().text().as_str(), source);
    assert_eq!(
        view.wast_syntax_tree(astral + 4).unwrap().parsed_to,
        initial.parsed_to
    );
    assert_eq!(view.redo(), Some(()));
    assert_eq!(&view.state().text().as_str()[..11], "(func $f)\r\n");
}

#[test]
fn viewport_at_crlf_midpoint_rounds_up_without_a_slice_eof() {
    let source = "(func $f)\r\n".repeat(500);
    let (mut view, _) = view(&source);
    let target = 310 * 11 + 10;
    assert_eq!(&source[target - 1..target + 1], "\r\n");
    report_range(&mut view, target - 20..target);
    let ticket = view.wast_idle_work().unwrap();
    pending_rounds(&mut view, &ticket, target, 65);
    assert!(view.wast_syntax_tree(target).is_none());
    assert_eq!(
        view.run_wast_idle(&ticket, 1).unwrap(),
        Some(WastIdleStep {
            target,
            rounds: 1,
            published: true,
        })
    );
    let output = view.wast_syntax_tree(target).unwrap();
    assert_eq!(output.parsed_to, target + 1);
    assert_eq!(output.tree.root().range().end, target + 2);
    assert!(!output.complete);
    assert!(view.wast_syntax_tree(target + 2).is_none());
}

#[test]
fn look_ahead_counts_normalized_utf16_and_rounds_to_raw_boundaries() {
    let source = format!(
        "(func $f)\r\n;;{}\r\n\"😀\"\r\n{}",
        " ".repeat(100_345),
        "(func $f)\r\n".repeat(300)
    );
    let astral = source.find('😀').unwrap();
    let (mut view, _) = view(&source);
    report_range(&mut view, 0..361);
    let ticket = view.wast_idle_work().unwrap();
    let target = astral + 4;
    let step = view.run_wast_idle(&ticket, 5).unwrap().unwrap();
    assert_eq!(
        (step.target, step.rounds, step.published),
        (target, 2, true)
    );
    let output = view.wast_syntax_tree(target).unwrap();
    assert_eq!(output.parsed_to, target);
    assert_eq!(output.tree.root().range().end, target + 1);
    assert!(view.wast_syntax_tree(target + 1).is_none());
}

#[test]
fn views_share_only_the_incoming_publication() {
    let source = "(func $f)\n".repeat(1_200);
    let (mut first, _) = view(&source);
    report_range(&mut first, 0..9);
    let first_ticket = first.wast_idle_work().unwrap();
    assert!(first.force_wast_parsing(8_000, 3_000).unwrap());
    let shared = first.state().clone();
    let later = first
        .ensure_wast_syntax_tree(10_000, 1_200)
        .unwrap()
        .unwrap();

    let mut second = EditorView::<()>::new(shared.clone());
    report_range(&mut second, 0..9);
    assert_eq!(second.run_wast_idle(&first_ticket, 1), Ok(None));
    assert!(!second.wast_syntax_tree_available(10_000));
    assert!(!second.force_wast_parsing(10_000, 0).unwrap());
    assert!(!second.publish_wast_stage(later).unwrap());
    let worker = second.wast_viewport.worker.as_mut().unwrap();
    worker.advance(5_000, 3_001).unwrap();
    let shorter = worker.ready(5_000).unwrap().clone();
    assert!(!second.publish_wast_stage(shorter).unwrap());
    assert!(Arc::ptr_eq(
        &shared.wast_syntax_tree(8_000).unwrap(),
        &second.wast_syntax_tree(8_000).unwrap()
    ));
    let second_ticket = second.wast_idle_work().unwrap();
    assert_eq!(first.run_wast_idle(&second_ticket, 1), Ok(None));
    assert!(second.force_wast_parsing(source.len(), 8_000).unwrap());
    assert!(first.wast_syntax_tree(10_000).is_none());
    assert!(first.wast_syntax_tree_available(10_000));

    first.set_state(second.state().clone());
    report_range(&mut first, 0..9);
    assert_eq!(first.run_wast_idle(&first_ticket, 1), Ok(None));
    let worker = first.wast_viewport.worker.as_ref().unwrap();
    assert!(worker.job.is_none() && worker.staged.is_none());
    assert!(first.force_wast_parsing(source.len(), 0).unwrap());
    assert!(first.wast_idle_work().is_none());
    assert!(Arc::ptr_eq(
        &first.wast_syntax_tree(source.len()).unwrap(),
        &second.wast_syntax_tree(source.len()).unwrap()
    ));
    assert!(attached_language_snapshot(first.state()).is_none());
}

#[test]
fn opt_in_never_feeds_state_consumers_or_falls_back_on_limit() {
    let source = " ".repeat(4_000) + &"(;".repeat(2_048);
    let (mut view, _) = view(&source);
    let old = view.wast_syntax_tree(0).unwrap();
    report_range(&mut view, 0..2);
    let ticket = view.wast_idle_work().unwrap();
    let error = view.run_wast_idle(&ticket, 10_000).unwrap_err();
    let WastViewportError::Parse(error @ ParseError::LimitExceeded { .. }) = error else {
        panic!("expected a parse limit, got {error:?}");
    };
    assert_eq!(view.wast_parse_error(), Some(error));
    assert_eq!(
        view.force_wast_parsing(source.len(), 0),
        Err(WastViewportError::Parse(error))
    );
    assert!(view.force_wast_parsing(0, 0).unwrap());
    assert!(view.wast_idle_work().is_none());
    assert!(view.wast_syntax_tree(source.len()).is_none());
    assert!(Arc::ptr_eq(&old, &view.wast_syntax_tree(0).unwrap()));
    assert!(attached_language_snapshot(view.state()).is_none());
    assert_eq!(
        WastLanguage::active_name(view.state()).as_deref(),
        Some("wast")
    );
    assert!(wast_language().is_active_at(view.state(), source.len(), Assoc::Before));
    assert_eq!(
        wast_language().find_regions(view.state()),
        vec![0..source.len()]
    );
    assert_eq!(
        view.state()
            .language_data_at::<crate::commands::CommentTokens>("commentTokens", 0, Assoc::After)
            [0]
        .line,
        Some(";;")
    );
    assert!(
        wast_language()
            .configure(Some(true), None)
            .viewport_extension()
            .is_none()
    );
    let synchronous = EditorState::with_extensions(source, wast_language().extension()).unwrap();
    assert!(attached_language_snapshot(&synchronous).is_some());
}
