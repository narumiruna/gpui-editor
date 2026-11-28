use std::cell::Cell;

use serde_json::{Value, json};

use super::*;
use crate::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Text, Transaction,
    merge::{
        Chunk, MergeView, MergeViewConfig, Side, UnifiedMergeConfig, build,
        original_doc_change_effect, unified_merge_state, unified_merge_view_with_timeout,
    },
};

const FIXTURE: &str = include_str!("../../../tests/fixtures/merge_timeout.jsonl");

fn rows() -> impl Iterator<Item = Value> {
    FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn config(row: &Value) -> DiffConfig {
    DiffConfig {
        scan_limit: number(&row["scanLimit"]),
    }
}

fn millis(value: &Value) -> Duration {
    Duration::from_millis(value.as_u64().unwrap())
}

fn replay<T>(row: &Value, run: impl FnOnce(Duration, &dyn Fn() -> Instant) -> T) -> T {
    let times = row["times"].as_array().unwrap();
    let epoch = Instant::now();
    let reads = Cell::new(0_usize);
    let now = || {
        let i = reads.get();
        reads.set(i + 1);
        epoch + millis(&times[i.min(times.len() - 1)])
    };
    let result = run(millis(&row["timeout"]), &now);
    assert_eq!(reads.get(), number(&row["reads"]), "{}", row["name"]);
    result
}

#[test]
fn pinned_diff_timeout() {
    for row in rows().filter(|row| row["raw"].is_array()) {
        let (a, b) = (string(&row["a"]), string(&row["b"]));
        let config = config(&row);
        for (kind, presentable) in [("raw", false), ("display", true)] {
            let diff = replay(&row, |timeout, now| {
                config.compute(a, b, presentable, timeout, now)
            });
            assert_eq!(
                json!(diff.precise),
                row["precise"],
                "{} {kind}",
                row["name"]
            );
            assert_eq!(
                json!(ranges(&diff.changes)),
                row[kind],
                "{} {kind}",
                row["name"]
            );
            assert_gaps(a, b, diff.changes);
        }
        let chunks = replay(&row, |timeout, now| {
            build(a, b, config.compute(a, b, true, timeout, now))
        });
        assert_chunks(&row, &Text::new(a), &Text::new(b), &chunks);
    }
}

#[test]
fn pinned_diff_timeout_restarts_for_each_update_window() {
    let row = rows().find(|row| row["name"] == "update-windows").unwrap();
    let separator = format!(
        "\n{}",
        string(&row["pad"]).repeat(number(&row["repetitions"]))
    );
    let doc = |key: &str| {
        Text::new(
            row[key]
                .as_array()
                .unwrap()
                .iter()
                .map(string)
                .collect::<Vec<_>>()
                .join(&separator),
        )
    };
    let (a, mut b) = (doc("linesA"), doc("linesB"));
    let old = Chunk::build(&a, &b);
    let changes = ChangeSet::new(
        row["edits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edit| Change::new(number(&edit[0])..number(&edit[1]), string(&edit[2])))
            .collect(),
    )
    .unwrap();
    let desc = changes.desc(b.len()).unwrap();
    changes.apply_to(&mut b).unwrap();
    let config = config(&row);
    let chunks = replay(&row, |timeout, now| {
        Chunk::update_with(&old, &a, &b, Side::B, &desc, |a, b| {
            Ok::<_, std::convert::Infallible>(config.compute(a, b, true, timeout, now))
        })
        .unwrap()
    });
    assert_chunks(&row, &a, &b, &chunks);
    assert_eq!(chunks.first(), old.first());
    assert_eq!(chunks.last(), old.last());
}

#[test]
fn real_clock_handles_zero_and_large_timeouts() {
    let config = DiffConfig { scan_limit: 500 };
    let (a, mut b) = (Text::new("abcd"), Text::new("abdc"));
    let old = Chunk::build(&a, &b);
    for timeout in [Duration::ZERO, Duration::MAX] {
        assert_eq!(
            config.diff_with_timeout(a.as_str(), b.as_str(), timeout),
            config.diff(a.as_str(), b.as_str())
        );
        assert_eq!(
            config.presentable_diff_with_timeout(a.as_str(), b.as_str(), timeout),
            config.presentable_diff(a.as_str(), b.as_str())
        );
        assert_eq!(Chunk::build_with_timeout(&a, &b, config, timeout), old);
    }
    let edit = ChangeSet::single(Change::new(0..1, "x")).unwrap();
    let desc = edit.desc(b.len()).unwrap();
    edit.apply_to(&mut b).unwrap();
    assert_eq!(
        Chunk::update_with_timeout(&old, &a, &b, Side::B, &desc, config, Duration::MAX),
        Chunk::build(&a, &b),
    );
}

#[test]
fn side_by_side_timeout_reconfiguration_keeps_chunks_until_an_edit() {
    let config = MergeViewConfig::default();
    let mut view = MergeView::new_with_timeout(
        EditorState::new("abcd"),
        EditorState::new("abdc"),
        config,
        Duration::MAX,
    )
    .unwrap();
    let initial = view.chunks().to_vec();
    let projection = view.projection().clone();
    view.reconfigure_diff_timeout(Duration::ZERO);
    view.dispatch(
        Side::B,
        Transaction::default().with_selection(EditorSelection::single(3, 3)),
    )
    .unwrap();
    assert_eq!(view.chunks(), initial);
    for side in [Side::A, Side::B] {
        assert_eq!(view.projection().rows(side), projection.rows(side));
    }

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    view.reconfigure_diff_override(Some(DiffOverride::new({
        let calls = Arc::clone(&calls);
        move |a, b| {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            vec![DiffChange {
                a: 0..a.len(),
                b: 0..b.len(),
            }]
        }
    })));
    view.reconfigure_diff_timeout(Duration::MAX);
    assert_eq!(view.chunks(), initial);
    let change = ChangeSet::single(Change::new(2..3, "é")).unwrap();
    view.dispatch(Side::B, Transaction::new(change)).unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(
        view.chunks()[0].changes(),
        &[DiffChange { a: 0..4, b: 0..5 }]
    );

    // Removing the override restores the timed built-in diff for new windows.
    view.reconfigure_diff_override(None);
    let change = ChangeSet::single(Change::new(2..4, "c")).unwrap();
    view.dispatch(Side::B, Transaction::new(change)).unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(
        view.chunks(),
        Chunk::build(view.state(Side::A).text(), view.state(Side::B).text())
    );
}

#[test]
fn unified_merge_timeout_updates_original_then_editor_and_keeps_old_snapshot() {
    let slot = Compartment::new();
    let original = Text::new("α\r\ncat\r\nend");
    let editor = "α\r\ncut\r\nend";
    let extension =
        unified_merge_view_with_timeout(UnifiedMergeConfig::new(original.clone()), Duration::MAX);
    let mut state = EditorState::with_extensions(editor, slot.of(extension.clone())).unwrap();
    let before = unified_merge_state(&state).unwrap().unwrap();
    assert_eq!(before.chunks(), Chunk::build(&original, state.text()));

    let change_a = ChangeSet::single(Change::new(5..6, "o")).unwrap();
    let change_b = ChangeSet::single(Change::new(5..6, "é")).unwrap();
    let desc_a = change_a.desc(original.len()).unwrap();
    let desc_b = change_b.desc(state.text().len()).unwrap();
    let effect = original_doc_change_effect(&state, change_a.clone()).unwrap();
    let mut after_a = original.clone();
    change_a.apply_to(&mut after_a).unwrap();
    let after_first = Chunk::update_with_timeout(
        before.chunks(),
        &after_a,
        state.text(),
        Side::A,
        &desc_a,
        DiffConfig::default(),
        Duration::MAX,
    );
    state
        .apply(Transaction::new(change_b).with_effects([effect, slot.reconfigure(extension)]))
        .unwrap();
    let after = unified_merge_state(&state).unwrap().unwrap();
    assert_eq!(after.original(), &after_a);
    assert_eq!(state.text().as_str(), "α\r\ncét\r\nend");
    assert_eq!(
        after.chunks(),
        Chunk::update_with_timeout(
            &after_first,
            &after_a,
            state.text(),
            Side::B,
            &desc_b,
            DiffConfig::default(),
            Duration::MAX,
        )
    );
    assert_eq!(before.original(), &original);
    assert_eq!(before.chunks(), Chunk::build(&original, &Text::new(editor)));
}

fn bounds(a: Range<usize>, b: Range<usize>) -> [usize; 4] {
    [a.start, a.end, b.start, b.end]
}

fn ranges(changes: &[DiffChange]) -> Vec<[usize; 4]> {
    changes
        .iter()
        .map(|ch| bounds(ch.a.clone(), ch.b.clone()))
        .collect()
}

fn assert_chunks(row: &Value, a: &Text, b: &Text, chunks: &[Chunk]) {
    let actual: Vec<_> = chunks
        .iter()
        .map(|ch| {
            json!([
                bounds(ch.range(Side::A), ch.range(Side::B)),
                ch.precise(),
                ranges(ch.changes())
            ])
        })
        .collect();
    assert_eq!(json!(actual), row["chunks"], "{}", row["name"]);
    assert_gaps(
        a.as_str(),
        b.as_str(),
        chunks.iter().flat_map(Chunk::absolute_changes),
    );
    let (mut into_a, mut into_b) = (a.clone(), b.clone());
    for chunk in chunks.iter().rev() {
        for (text, side) in [(a, Side::A), (b, Side::B)] {
            let range = chunk.range(side);
            let text = text.as_str();
            assert!(text.get(range.clone()).is_some());
            assert!(range.start == 0 || text.as_bytes()[range.start - 1] == b'\n');
            assert!(
                range.end == 0
                    || range.end == text.len()
                    || text.as_bytes()[range.end - 1] == b'\n'
            );
        }
        into_a = chunk.apply(&into_a, b, Side::A).unwrap();
        into_b = chunk.apply(a, &into_b, Side::B).unwrap();
    }
    assert_eq!(&into_a, b);
    assert_eq!(&into_b, a);
}

fn assert_gaps(a: &str, b: &str, changes: impl IntoIterator<Item = DiffChange>) {
    let (mut last_a, mut last_b) = (0, 0);
    for change in changes {
        assert_eq!(&a[last_a..change.a.start], &b[last_b..change.b.start]);
        assert!(a.get(change.a.clone()).is_some() && b.get(change.b.clone()).is_some());
        (last_a, last_b) = (change.a.end, change.b.end);
    }
    assert_eq!(&a[last_a..], &b[last_b..]);
}
