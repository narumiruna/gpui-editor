use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditError, EditorState, RangeTouch, StateUpdate,
    language::line_projection::configured_editor_lines,
    view::tooltips::{HoverOptions, HoverTooltip, Tooltip, hover_tooltip},
};

use super::{Diagnostic, LINT_STATE, LintState, providers, published_diagnostics};

/// Return whether to hide a lint tooltip for an accepted update. The range
/// uses old-document bytes. Return `None` to defer to the default policy.
pub type LintTooltipHide =
    Arc<dyn Fn(&StateUpdate<'_>, Range<usize>) -> Option<bool> + Send + Sync>;

static LINT_HOVER: LazyLock<HoverTooltip> = LazyLock::new(|| {
    hover_tooltip(HoverOptions {
        hide_on: Some(Arc::new(should_hide)),
        ..Default::default()
    })
});

/// The native lint hover source installed with lint state. Use
/// [`lint_tooltip_at`] to resolve its content. The host supplies pointer
/// timing, layout, and rendering.
pub fn lint_hover_tooltip() -> &'static HoverTooltip {
    &LINT_HOVER
}

fn should_hide(update: &StateUpdate<'_>, tooltip: &Tooltip) -> bool {
    let range = tooltip.pos..tooltip.end.unwrap_or(tooltip.pos);
    let configs = update
        .state()
        .facet(&providers::LINTERS)
        .expect("valid lint configuration");
    let mut decision = None;
    for callback in providers::combine_callbacks(
        configs
            .iter()
            .filter_map(|input| input.config.hide_on.as_ref()),
    ) {
        decision = callback(update, range.clone());
        if decision == Some(true) {
            break;
        }
    }
    if let Some(hide) = decision {
        return hide;
    }
    let previous = update.start().field(&LINT_STATE).expect("valid lint state");
    if update
        .transaction()
        .effects()
        .iter()
        .any(|effect| published_diagnostics(effect, update, previous.as_deref()).is_some())
    {
        return true;
    }
    let projection = configured_editor_lines(update.start()).expect("valid editor line projection");
    let lines = projection
        .as_ref()
        .as_ref()
        .unwrap_or(update.start().text());
    lines.line_at(range.start).is_none_or(|line| {
        update
            .changes()
            .touches_range(line.range.start..line.range.end.max(range.end))
            != RangeTouch::None
    })
}

/// The source segment and display list for a lint tooltip. The segment is in
/// current UTF-8 byte coordinates. It does not change when filters modify the
/// display list. Use live diagnostic identities for editor actions.
#[derive(Clone, Debug)]
pub struct LintTooltip {
    pub range: Range<usize>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Find the first matching diagnostic segment in marker layer order. At a
/// segment edge, use the requested side; a point can match on either side.
/// A filter may produce an empty list without changing the selected segment.
pub fn lint_tooltip_at(
    state: &EditorState,
    position: usize,
    side: Assoc,
) -> Result<Option<LintTooltip>, EditError> {
    if !state.text().is_char_boundary(position) {
        return Err(EditError::InvalidRange);
    }
    let Some(lint) = LintState::get(state) else {
        return Ok(None);
    };
    let mut found = None;
    lint.markers().between(position, position, |mark| {
        if mark.from == mark.to
            || ((position > mark.from || side == Assoc::After)
                && (position < mark.to || side == Assoc::Before))
        {
            found = Some(LintTooltip {
                range: mark.from..mark.to,
                diagnostics: filter_tooltip_diagnostics(
                    state,
                    mark.value
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| (**diagnostic).clone())
                        .collect(),
                ),
            });
            false
        } else {
            true
        }
    });
    Ok(found)
}

/// Apply the installed tooltip filters in extension order to the diagnostics
/// already selected by the host. This does not change markers or panel rows.
pub fn filter_tooltip_diagnostics(
    state: &EditorState,
    mut diagnostics: Vec<Diagnostic>,
) -> Vec<Diagnostic> {
    let context = state.context();
    let configs = context
        .facet(&providers::LINTERS)
        .expect("valid lint configuration");
    for filter in providers::combine_callbacks(
        configs
            .iter()
            .filter_map(|input| input.config.tooltip_filter.as_ref()),
    ) {
        diagnostics = filter(diagnostics, &context);
    }
    diagnostics
}
