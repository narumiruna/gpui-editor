use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorState, Text, Transaction,
    language::{
        lezer::{NodeProp, ParseError, SyntaxTree},
        lezer_grammar::{LezerParseJob, LezerParseOutput},
    },
    language_data::{LezerLanguage, lezer_language},
    view::{EditorView, LezerViewportError, Viewport},
};

// Projected UTF-16 cutoffs 3000, 5000, 5268, and 5269 from the pinned
// @codemirror/lang-lezer 6.0.2 / @lezer/lr 1.4.13 DocInput probe.
const INITIAL: usize = 3430;
const MIDDLE: usize = 5715;
const CLOSE: usize = 6022;
const TRAILING: usize = 6023;

fn source() -> String {
    format!("// 😀\r\n@top Main {{\r\n{}}})", "  \"hi\"\r\n".repeat(750))
}

type Row = (usize, u16, Range<usize>);

fn rows(tree: &SyntaxTree, source: &str) -> Vec<Row> {
    let mut pending = vec![(0, tree.root())];
    let mut rows = Vec::new();
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some(), "{range:?}");
        for at in [range.start, range.end] {
            assert!(at == 0 || source.as_bytes().get(at - 1..=at) != Some(b"\r\n"));
        }
        assert_eq!(node.node_type().unwrap().name(), node.name());
        rows.push((depth, node.term(), range));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    rows
}

fn prefix(out: &LezerParseOutput, cutoff: usize, source: &str) {
    let (end, literals, last) = match cutoff {
        INITIAL => (3432, 426, (3, 0, 3432..3432)),
        MIDDLE => (5716, 712, (3, 0, 5716..5716)),
        CLOSE | TRAILING => (TRAILING, 750, (3, 15, CLOSE..TRAILING)),
        _ => unreachable!(),
    };
    let sequence_end = if literals == 750 { 6020 } else { end };
    let mut expected = vec![
        (0, 3, 0..end),
        (1, 1, 0..7),
        (1, 48, 9..end),
        (2, 49, 9..13),
        (2, 5, 14..18),
        (2, 19, 19..end),
        (3, 16, 19..20),
        (3, 40, 24..sequence_end),
    ];
    expected.extend((0..literals).map(|i| (4, 13, 24 + i * 8..28 + i * 8)));
    expected.push(last);
    assert_eq!(rows(&out.tree, source), expected, "cutoff {cutoff}");
    assert_eq!(out.parsed_to, cutoff);
    assert_eq!(out.complete, cutoff == TRAILING);
}

fn drive(job: &mut LezerParseJob, cutoff: usize) -> LezerParseOutput {
    const ROUNDS: usize = 11;
    for _ in 0..256 {
        let step = job.advance(cutoff, ROUNDS).unwrap();
        assert!(step.rounds <= ROUNDS);
        if let Some(out) = step.output {
            assert!(step.output_work > 0 || out.complete);
            return out;
        }
        assert_eq!((step.rounds, step.output_work), (ROUNDS, 0));
    }
    panic!("cutoff {cutoff} not reached in the test budget");
}

fn error_at(error: ParseError, at: usize) {
    assert!(
        matches!(error, ParseError::UnexpectedToken { offset, .. } if offset == at),
        "{error:?}"
    );
}

fn errors(tree: &SyntaxTree, source: &str) -> Vec<(usize, usize)> {
    rows(tree, source)
        .into_iter()
        .filter_map(|(_, term, range)| (term == 0).then_some((range.start, range.end)))
        .collect()
}

#[test]
fn strict_job_stages_pinned_prefixes_without_recovering_live() {
    let strict = lezer_language().configure(Some(true), None);
    let raw = source();
    let text = Text::new(&raw);
    let mut job = strict.parse_job(text.clone()).unwrap();
    assert!(job.advance(INITIAL, 0).unwrap().output.is_none());
    for cutoff in [INITIAL, MIDDLE, CLOSE] {
        let continued = drive(&mut job, cutoff);
        let cold = drive(&mut strict.parse_job(text.clone()).unwrap(), cutoff);
        let restaged = job.advance(cutoff, 0).unwrap();
        assert_eq!(restaged.rounds, 0);
        assert!(restaged.output_work > 0);
        for output in [continued, cold, restaged.output.unwrap()] {
            prefix(&output, cutoff, &raw);
        }
    }
    assert!(job.advance(TRAILING, 0).unwrap().output.is_none());
    error_at(job.advance(TRAILING, 1).unwrap_err(), TRAILING);
    let ended = job.advance(raw.len(), 1).unwrap();
    assert_eq!((ended.rounds, ended.output_work), (0, 0));
    assert!(ended.output.is_none());

    let repaired = &raw[..TRAILING];
    let mut valid = strict.parse_job(Text::new(repaired)).unwrap();
    prefix(&drive(&mut valid, CLOSE), CLOSE, repaired);
    prefix(&drive(&mut valid, TRAILING), TRAILING, repaired);
}

#[test]
fn strict_cutoff_includes_tokens_at_the_boundary_and_projects_ranges() {
    let strict = lezer_language().configure(Some(true), None);
    let start = |raw: &str| strict.parse_job(Text::new(raw)).unwrap();
    let raw = "R {})";
    let mut job = start(raw);
    let open = drive(&mut job, 2);
    assert_eq!(errors(&open.tree, raw), [(3, 3)]);
    let closed = drive(&mut job, 3);
    assert_eq!(closed.tree.root().range(), 0..4);
    assert!(errors(&closed.tree, raw).is_empty());
    error_at(job.advance(4, 1).unwrap_err(), 4);

    let raw = "@top Foo";
    let mut job = start(raw);
    assert_eq!(errors(&drive(&mut job, 7).tree, raw), [(8, 8)]);
    error_at(job.advance(8, 1).unwrap_err(), 8);

    let raw = "R { \"long 😀é\" })";
    let mut job = start(raw);
    let literal = drive(&mut job, 4);
    assert_eq!(literal.tree.root().range(), 0..17);
    assert_eq!(errors(&literal.tree, raw), [(17, 17)]);
    let inside_scalar = job.advance(11, 0).unwrap().output.unwrap();
    assert_eq!(inside_scalar.parsed_to, 14);
    assert_eq!(rows(&inside_scalar.tree, raw), rows(&literal.tree, raw));
    error_at(job.advance(19, 11).unwrap_err(), 19);

    let raw = "// 😀\r\nR {})";
    let mut job = start(raw);
    assert_eq!(drive(&mut job, 4).parsed_to, 7);
    assert_eq!(drive(&mut job, 8).parsed_to, 9);
    error_at(job.advance(raw.len(), 11).unwrap_err(), raw.len() - 1);

    let raw = "// 😀\r)";
    let mut job = start(raw);
    assert!(errors(&drive(&mut job, 7).tree, raw).is_empty());
    error_at(job.advance(raw.len(), 11).unwrap_err(), 8);
}

#[test]
fn strict_viewport_retains_publication_on_failure_and_restarts_on_changes() {
    let marker = NodeProp::new();
    let configure = |label| {
        lezer_language()
            .configure(Some(true), Some(label))
            .with_node_props(&[marker.add(move |row| row.is("Literal").then_some(label))])
    };
    let literal_label = |out: &LezerParseOutput| {
        let mut pending = vec![out.tree.root()];
        while let Some(node) = pending.pop() {
            if node.name() == "Literal" {
                return node.node_type().unwrap().prop(&marker).copied();
            }
            pending.extend(node.children().rev());
        }
        None
    };
    let strict = configure("first");
    let other = configure("next");
    let raw = source();
    let slot = Compartment::new();
    let install = |lang: &LezerLanguage| lang.viewport_extension().unwrap();
    let state = EditorState::with_extensions(&raw, slot.of(install(&strict))).unwrap();
    assert!(state.lezer_syntax_tree(INITIAL).is_none());
    let mut view = EditorView::new(state);
    let viewport = Viewport::new(view.state().text(), 0..200, std::iter::once(0..200)).unwrap();
    view.set_viewport_geometry(viewport).unwrap();
    let initial = view.lezer_syntax_tree(INITIAL).unwrap();
    prefix(&initial, INITIAL, &raw);
    assert_eq!(literal_label(&initial), Some("first"));
    assert!(!view.lezer_syntax_tree_available(INITIAL + 1));
    let ticket = view.lezer_idle_work().unwrap();
    let old_state = view.state().clone();

    assert!(view.ensure_lezer_syntax_tree(MIDDLE, 1).unwrap().is_none());
    let stage = view
        .ensure_lezer_syntax_tree(MIDDLE, 2048)
        .unwrap()
        .unwrap();
    prefix(&stage, MIDDLE, &raw);
    assert!(view.lezer_syntax_tree_available(MIDDLE));
    assert!(!view.lezer_syntax_tree_available(MIDDLE + 1));
    assert!(view.lezer_syntax_tree(MIDDLE).is_none());
    assert!(view.force_lezer_parsing(MIDDLE, 0).unwrap());
    assert!(Arc::ptr_eq(
        &stage,
        &view.lezer_syntax_tree(MIDDLE).unwrap()
    ));
    let last = view.ensure_lezer_syntax_tree(CLOSE, 256).unwrap().unwrap();
    prefix(&last, CLOSE, &raw);
    let LezerViewportError::Parse(error) = view.force_lezer_parsing(TRAILING, 1).unwrap_err()
    else {
        panic!("expected a strict parse error");
    };
    error_at(error, TRAILING);
    error_at(view.lezer_parse_error().unwrap(), TRAILING);
    assert!(Arc::ptr_eq(
        &stage,
        &view.lezer_syntax_tree(MIDDLE).unwrap()
    ));
    assert!(view.lezer_syntax_tree(CLOSE).is_none());
    assert!(Arc::ptr_eq(
        &last,
        &view.ensure_lezer_syntax_tree(CLOSE, 0).unwrap().unwrap()
    ));
    assert!(!view.lezer_syntax_tree_available(TRAILING));
    assert!(view.lezer_idle_work().is_none());
    assert_eq!((view.revision(), view.undo_depth()), (0, 0));

    let reconfigure = |lang: &LezerLanguage| slot.reconfigure(install(lang));
    view.update(
        Transaction::default().with_effects([reconfigure(&other)]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert!(view.run_lezer_idle(&ticket, 1).unwrap().is_none());
    assert!(view.lezer_parse_error().is_none());
    let next = view.lezer_syntax_tree(INITIAL).unwrap();
    prefix(&next, INITIAL, &raw);
    assert_eq!(literal_label(&next), Some("next"));
    assert!(Arc::ptr_eq(
        &initial,
        &old_state.lezer_syntax_tree(INITIAL).unwrap()
    ));

    // The old job already reached the closing token. Change it so restoring
    // that cutoff must run strict parsing on the new text.
    assert!(
        view.ensure_lezer_syntax_tree(CLOSE, 2048)
            .unwrap()
            .is_some()
    );
    let txn =
        |range, insert| Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap());
    view.update(txn(CLOSE..TRAILING, "]"), (), |_| (None, ()))
        .unwrap();
    assert!(view.lezer_syntax_tree(CLOSE).is_none());
    error_at(
        view.ensure_lezer_syntax_tree(raw.len(), 2048).unwrap_err(),
        CLOSE,
    );

    let loose = other.configure(Some(false), None);
    view.update(
        txn(CLOSE..TRAILING, "}").with_effects([reconfigure(&loose)]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert!(view.lezer_parse_error().is_none());
    let recovered = view
        .ensure_lezer_syntax_tree(raw.len(), 2048)
        .unwrap()
        .unwrap();
    assert!(recovered.complete);
    assert_eq!(errors(&recovered.tree, &raw), [(TRAILING, raw.len())]);

    view.update(
        txn(TRAILING..raw.len(), "").with_effects([reconfigure(&strict)]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert!(view.force_lezer_parsing(TRAILING, 2048).unwrap());
    prefix(
        &view.lezer_syntax_tree(TRAILING).unwrap(),
        TRAILING,
        &raw[..TRAILING],
    );
}

#[test]
fn strict_viewport_bootstrap_failure_does_not_publish_a_recovered_tree() {
    let strict = lezer_language().configure(Some(true), None);
    let state =
        EditorState::with_extensions("R {})", strict.viewport_extension().unwrap()).unwrap();
    let mut view = EditorView::<()>::new(state);
    error_at(view.lezer_parse_error().unwrap(), 4);
    assert!(view.lezer_syntax_tree(0).is_none());
    assert!(!view.lezer_syntax_tree_available(0));
    error_at(view.ensure_lezer_syntax_tree(0, 0).unwrap_err(), 4);
}
