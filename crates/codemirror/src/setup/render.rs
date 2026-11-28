use std::{
    ops::RangeInclusive,
    sync::{Arc, LazyLock},
};

use crate::{
    EditError, EditorState, Extension, Line, RangeSet,
    language::{
        brackets::state_bracket_match_decorations,
        fold::{
            FOLD_GUTTER_CLASS, FoldGutterClick, FoldGutterConfig, FoldPlaceholder,
            fold_placeholder_decorations, folded_ranges, syntax_fallback,
        },
        line_projection::configured_editor_lines,
    },
    language_data::NativeSyntax,
    search::{highlight_search_matches, selection_match_decorations},
    view::{
        MarkDecoration, SpecialChar, ViewHighlight,
        decorations::{Decoration, DecorationSet, ReplaceSpec},
        gutters::{
            GutterConfig, GutterMarker, GutterSlot, gutter_installed, gutter_slots,
            gutter_slots_with_line_marker, gutter_with_identity, highlight_active_line_gutter,
            preset_line_numbers,
        },
        special_char_decorations,
    },
};

use super::{SetupEditor, current_syntax, milliseconds};

static FOLD_CONFIG: LazyLock<FoldGutterConfig> = LazyLock::new(FoldGutterConfig::default);
static FOLD_GUTTER: LazyLock<Arc<GutterConfig>> = LazyLock::new(|| {
    Arc::new(GutterConfig {
        class: FOLD_GUTTER_CLASS.into(),
        line_marker: Some(Arc::new(|state, line, _| fold_marker(state, line, None))),
        spacer: Some(Arc::new(|state| FOLD_CONFIG.spacer(state).marker)),
        ..GutterConfig::default()
    })
});
static FOLD_GUTTER_EXTENSION: LazyLock<Extension> =
    LazyLock::new(|| gutter_with_identity(FOLD_GUTTER.clone()));

fn fold_marker(
    state: &EditorState,
    line: &Line<'_>,
    syntax: Option<&NativeSyntax>,
) -> Option<GutterMarker> {
    FOLD_CONFIG
        .marker(state, line.range.clone(), syntax_fallback(syntax))
        .map(|marker| marker.marker)
}

pub(super) fn gutter_extensions() -> [Extension; 3] {
    [
        preset_line_numbers(),
        highlight_active_line_gutter(),
        FOLD_GUTTER_EXTENSION.clone(),
    ]
}

/// Current decorations for the host to draw. All coordinates are UTF-8 bytes.
/// Fold widgets replace their ranges; use the same ranges as atomic spans.
#[derive(Debug)]
pub struct SetupDecorations {
    pub special_chars: DecorationSet<SpecialChar>,
    pub active_lines: DecorationSet,
    pub selection_matches: RangeSet<MarkDecoration>,
    pub search_matches: RangeSet<MarkDecoration>,
    /// Stock or configured marks from the current bracket matches.
    pub bracket_matches: RangeSet<MarkDecoration>,
    /// Stock labels. Use [`SetupEditor::configured_fold_placeholders`] to
    /// render installed placeholder settings and prepared labels.
    pub fold_placeholders: DecorationSet<&'static str>,
}

impl<M: Clone> SetupEditor<M> {
    /// Project the native presets' decorations from the current view state
    /// and viewport. The fallback syntax style is separate.
    pub fn decorations(&self) -> Result<SetupDecorations, EditError> {
        let state = self.view.state();
        let text = state.text();
        let viewport = self.view.viewport();
        let mut output = SetupDecorations {
            special_chars: special_char_decorations(state, viewport)?,
            active_lines: ViewHighlight::ActiveLine.project(state, viewport)?,
            selection_matches: selection_match_decorations(state, viewport)?,
            search_matches: highlight_search_matches(state, viewport)?,
            bracket_matches: state_bracket_match_decorations(state)?,
            fold_placeholders: DecorationSet::empty(),
        };
        if self.basic {
            let placeholder = Decoration::replace(Some("…"), ReplaceSpec::default());
            output.fold_placeholders = DecorationSet::new(
                text,
                folded_ranges(state)
                    .iter(0)
                    .map(|range| placeholder.try_range(range.from..range.to))
                    .collect::<Result<Vec<_>, _>>()?,
                true,
            )
            .map_err(|_| EditError::InvalidRange)?;
        }
        Ok(output)
    }

    /// Project the current fold widgets with their configured labels and
    /// stock accessibility metadata.
    pub fn configured_fold_placeholders(
        &self,
    ) -> Result<DecorationSet<FoldPlaceholder>, EditError> {
        fold_placeholder_decorations(self.view.state())
    }

    /// Project gutters for the given one-based logical lines. The host can
    /// override attached syntax and decides which rows to draw after wrapping
    /// or folding.
    pub fn gutters(
        &self,
        lines: RangeInclusive<usize>,
        syntax: Option<&NativeSyntax>,
    ) -> Result<Vec<GutterSlot>, EditError> {
        let state = self.view.state();
        let Some(syntax) = current_syntax(state, syntax).filter(|_| self.basic) else {
            return gutter_slots(state, lines);
        };
        gutter_slots_with_line_marker(state, lines, &FOLD_GUTTER, &|state, line, _| {
            fold_marker(state, line, Some(syntax))
        })
    }

    /// Act on a basic preset fold marker. The existing fold takes precedence
    /// over a foldable syntax range. Does not move the selection.
    pub fn toggle_line_fold(
        &mut self,
        line_number: usize,
        syntax: Option<&NativeSyntax>,
        now: std::time::Duration,
        metadata: M,
    ) -> Result<bool, EditError> {
        let state = self.view.state();
        if !self.basic || !gutter_installed(state, &FOLD_GUTTER)? {
            return Ok(false);
        }
        let context = state.context();
        let projected = configured_editor_lines(&context)?;
        let text = projected.as_ref().as_ref().unwrap_or(context.text());
        let line = text.line(line_number).ok_or(EditError::InvalidRange)?;
        let syntax = current_syntax(state, syntax);
        let click = FOLD_CONFIG.click(state, line.range, syntax_fallback(syntax));
        let FoldGutterClick::Dispatch(transaction) = click else {
            return Ok(false);
        };
        self.view.update(
            transaction.with_time(milliseconds(now)),
            metadata.clone(),
            |_| (None, metadata),
        )?;
        Ok(true)
    }
}
