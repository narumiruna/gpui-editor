use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, ConfigurationError, EditorState, Extension, MapMode, Range, RangeSet, RangeSetUpdate,
    RangeValue, StateEffect, StateEffectType, StateField,
    language::line_projection::{configured_editor_lines, editor_line_extension},
};

static MARK: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|pos, changes| {
        changes.map_pos(*pos, Assoc::Before, MapMode::TrackAfter)
    })
});

#[derive(Clone)]
struct Close;

impl RangeValue for Close {
    fn start_side(&self) -> i32 {
        1
    }

    fn end_side(&self) -> i32 {
        -1
    }
}

#[derive(Default)]
pub(super) struct Tracked(RangeSet<Close>);

impl Tracked {
    pub(super) fn at(&self, pos: usize) -> bool {
        self.0
            .iter(pos)
            .take_while(|range| range.from <= pos)
            .any(|range| range.from == pos)
    }
}

static TRACKED: LazyLock<StateField<Tracked>> = LazyLock::new(|| {
    StateField::new(
        |_| Ok(Tracked::default()),
        |tracked: &Tracked, update| {
            let tr = update.transaction();
            let mapped = tracked.0.map(tr.changes());
            let text = update.state().text();
            let keep_range = if let Some(selection) = tr.selection().filter(|_| !mapped.is_empty())
            {
                let line_view = configured_editor_lines(update.state())?;
                let lines = line_view.as_ref().as_ref().unwrap_or(text);
                lines
                    .line_at(selection.main().head())
                    .map(|line| line.range)
            } else {
                None
            };
            let mut keep = |range: &Range<Close>| {
                keep_range
                    .as_ref()
                    .is_none_or(|line| (line.start..=line.end).contains(&range.from))
            };
            let add = tr
                .effects()
                .iter()
                .filter_map(|effect| {
                    let &pos = effect.get(&MARK)?;
                    // The mark spans the first scalar of a close, including a triple quote.
                    let ch = text.as_str().get(pos..)?.chars().next()?;
                    Some(Close.range(pos, pos + ch.len_utf8()))
                })
                .collect();
            let ranges = mapped
                .update(RangeSetUpdate {
                    add,
                    sort: true,
                    filter: Some(&mut keep),
                    filter_range: None,
                })
                .expect("close markers are valid ranges");
            Ok(Tracked(ranges))
        },
    )
});

pub(super) fn extension() -> Extension {
    Extension::group([editor_line_extension(), TRACKED.extension()])
}

pub(super) fn mark(pos: usize) -> StateEffect {
    MARK.of(pos)
}

pub(super) fn get(state: &EditorState) -> Result<Option<Arc<Tracked>>, ConfigurationError> {
    state.context().field(&TRACKED)
}
