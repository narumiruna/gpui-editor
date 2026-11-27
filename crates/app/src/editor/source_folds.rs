use std::{ops::Range, sync::Arc};

use gpui::{Context, actions};
use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, StateEffect, Text, Transaction,
    language::{
        StreamSettings,
        fold::{
            FoldGutterClick, FoldGutterConfig, FoldGutterMarker, FoldGutterState, announce_fold,
            fold_effect, foldable, folded_ranges, unfold_effect,
        },
        markdown::MarkdownCodeParser,
    },
    language_data::{NativeLanguage, NativeSyntax},
    view::{SCROLL_SNAPSHOT, ScrollSnapshot},
};

use super::Editor;

actions!(editor, [ToggleSourceFold]);

pub(super) struct SourceFoldQuery<'a> {
    body: Range<usize>,
    syntax: Option<&'a NativeSyntax>,
    configured_fold: bool,
    state: EditorState,
}

impl SourceFoldQuery<'_> {
    pub(super) fn marker(
        &self,
        config: &FoldGutterConfig,
        line: Range<usize>,
    ) -> Option<FoldGutterMarker> {
        config.marker_if(
            &self.state,
            line.clone(),
            |_, from, to| self.syntax_fold(from, to),
            |kind, range| self.accepts(&line, kind, range),
        )
    }

    fn click(
        &self,
        config: &FoldGutterConfig,
        line: Range<usize>,
    ) -> Option<(FoldGutterState, Range<usize>, Transaction)> {
        let FoldGutterClick::Dispatch(transaction) =
            config.click(&self.state, line.clone(), |_, from, to| {
                self.syntax_fold(from, to)
            })
        else {
            return None;
        };
        let [effect] = transaction.effects() else {
            return None;
        };
        let (kind, range) = if let Some(range) = effect.get(fold_effect()) {
            (FoldGutterState::Open, range)
        } else {
            (FoldGutterState::Closed, effect.get(unfold_effect())?)
        };
        let range = range.clone();
        self.accepts(&line, kind, &range)
            .then_some((kind, range, transaction))
    }

    fn line_at(&self, start: usize) -> Option<Range<usize>> {
        if let Some(lines) = self.syntax.and_then(NativeSyntax::syntax_lines) {
            let origin = self.body.start;
            let local = lines.line_at(start.checked_sub(origin)?)?.range;
            Some(origin + local.start..origin + local.end)
        } else {
            Some(self.state.text().line_at(start)?.range)
        }
    }

    fn syntax_fold(&self, from: usize, to: usize) -> Option<Range<usize>> {
        syntax_fold_in(
            self.syntax?,
            &self.state,
            &self.body,
            from,
            to,
            self.configured_fold,
        )
    }

    fn accepts(&self, line: &Range<usize>, kind: FoldGutterState, range: &Range<usize>) -> bool {
        let text = self.state.text().as_str();
        if range.is_empty()
            || !inside(&self.body, range)
            || !(line.start..=line.end).contains(&range.start)
            || text.get(range.clone()).is_none()
            || [range.start, range.end]
                .into_iter()
                .any(|at| at > 0 && text.get(at - 1..at + 1) == Some("\r\n"))
        {
            return false;
        }
        let folds = folded_ranges(&self.state);
        let mut nearby = folds
            .iter(range.start)
            .take_while(|fold| fold.from < range.end);
        match kind {
            FoldGutterState::Open => {
                nearby.all(|old| old.to <= range.start || inside(range, &(old.from..old.to)))
            }
            FoldGutterState::Closed => {
                nearby.any(|old| old.from == range.start && old.to == range.end)
            }
        }
    }
}

fn has_fold_props(parser: &MarkdownCodeParser) -> bool {
    matches!(
        parser,
        MarkdownCodeParser::Wast(_) | MarkdownCodeParser::Json(_) | MarkdownCodeParser::Lezer(_)
    )
}

fn syntax_fold_in(
    syntax: &NativeSyntax,
    state: &EditorState,
    body: &Range<usize>,
    from: usize,
    to: usize,
    configured_fold: bool,
) -> Option<Range<usize>> {
    if configured_fold {
        return syntax.foldable_in_scope(state, body.clone(), from..to);
    }
    let origin = body.start;
    let local = syntax.foldable(from.checked_sub(origin)?, to.checked_sub(origin)?)?;
    Some(origin + local.start..origin + local.end)
}

impl Editor {
    pub(super) fn source_body_line_projection(&mut self) -> Option<Text> {
        let fence = self.source_fence()?;
        let parser = fence.parser?;
        if !parser.language().uses_projected_lines() {
            return None;
        }
        let body = fence.body;
        let settings = StreamSettings {
            tab_size: self.view.state().tab_size(),
            ..Default::default()
        };
        let document = self.source_document(&body, &parser, settings)?;
        document.syntax().projected_lines().cloned()
    }

    pub(super) fn source_fold_query(&mut self, body: &Range<usize>) -> Option<SourceFoldQuery<'_>> {
        let fence = self.source_fence().filter(|fence| &fence.body == body)?;
        let state = self.view.state().clone();
        let settings = StreamSettings {
            tab_size: state.tab_size(),
            ..Default::default()
        };
        let configured_fold = fence.parser.as_ref().is_some_and(has_fold_props);
        let syntax = fence.parser.and_then(|parser| {
            self.source_document(body, &parser, settings)
                .map(|doc| doc.syntax())
        });
        Some(SourceFoldQuery {
            body: body.clone(),
            syntax,
            configured_fold,
            state,
        })
    }

    pub(super) fn click_source_fold_gutter(
        &mut self,
        body: &Range<usize>,
        line: Range<usize>,
        config: &Arc<FoldGutterConfig>,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled || self.is_composing() {
            return;
        }
        let Some(current) = FoldGutterConfig::configured(self.view.state()) else {
            return;
        };
        if !Arc::ptr_eq(&current, config) {
            return;
        }
        let Some(query) = self.source_fold_query(body) else {
            return;
        };
        if !inside(body, &line) || query.line_at(line.start).as_ref() != Some(&line) {
            return;
        }
        let Some((kind, range, transaction)) = query.click(&current, line) else {
            return;
        };
        let effect = match kind {
            FoldGutterState::Open => fold_effect(),
            FoldGutterState::Closed => unfold_effect(),
        };
        self.apply_source_fold_change(
            transaction,
            None,
            |accepted, snapshot| {
                let effects = accepted.effects();
                accepted.changes().is_empty()
                    && accepted.selection().is_none()
                    && !accepted.add_to_history()
                    && !accepted.scroll_into_view()
                    && effects.len() == 1 + usize::from(snapshot.is_some())
                    && effects
                        .iter()
                        .filter(|e| e.get(effect) == Some(&range))
                        .count()
                        == 1
                    && snapshot.is_none_or(|snapshot| {
                        effects
                            .iter()
                            .any(|e| e.get(&SCROLL_SNAPSHOT) == Some(&snapshot))
                    })
            },
            cx,
        );
    }

    pub(super) fn can_toggle_source_fold(&self) -> bool {
        self.input_enabled
            && !self.is_composing()
            && self.source_fence().is_some_and(|fence| {
                fence.body.start <= self.cursor()
                    && self.cursor() <= fence.body.end
                    && fence
                        .language()
                        .is_some_and(|language| !matches!(language, NativeLanguage::Legacy(_)))
            })
    }

    pub(super) fn toggle_source_fold(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_toggle_source_fold() {
            return false;
        }
        let fence = self.source_fence().expect("checked source fence");
        let head = self.cursor();
        let parser = fence.parser.expect("checked source parser");
        let configured_fold = has_fold_props(&parser);
        let body = fence.body;
        let origin = body.start;
        let folded = self.source_fold_ranges();
        let state = self.view.state().clone();
        let text = state.text().slice_owned(body.clone()).expect("valid fence");
        let syntax = self
            .source_document(&body, &parser, StreamSettings::default())
            .map(|document| document.syntax());
        let lines = syntax.and_then(NativeSyntax::syntax_lines).unwrap_or(&text);
        let Some(line) = lines.line_at(head - origin) else {
            return false;
        };
        if let Some(range) = folded.iter().find(|range| {
            range.start <= origin + line.range.end && range.end > origin + line.range.start
        }) {
            let notice = fold_notice(&state, lines, origin, range, false);
            return self.toggle_fold_range(range.clone(), false, notice, None, cx);
        }
        for number in (1..=line.number).rev() {
            let candidate = lines.line(number).expect("valid line").range;
            let Some(range) = foldable(
                &state,
                origin + candidate.start,
                origin + candidate.end,
                |state, start, end| {
                    syntax_fold_in(syntax?, state, &body, start, end, configured_fold)
                },
            ) else {
                continue;
            };
            if range.end <= origin + line.range.start
                || !inside(&body, &range)
                || folded.iter().any(|old| {
                    old.start < range.end && range.start < old.end && !inside(&range, old)
                })
            {
                continue;
            }
            let selection = (range.start < head && head < range.end).then_some(range.start);
            let notice = fold_notice(&state, lines, origin, &range, true);
            return self.toggle_fold_range(range, true, notice, selection, cx);
        }
        false
    }

    fn toggle_fold_range(
        &mut self,
        range: Range<usize>,
        folding: bool,
        notice: StateEffect,
        collapse_to: Option<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        let effect = if folding {
            fold_effect()
        } else {
            unfold_effect()
        };
        let expected_selection = collapse_to.map(|at| EditorSelection::single(at, at));
        self.apply_source_fold_change(
            Transaction::default().with_effects([effect.of(range.clone()), notice]),
            collapse_to,
            |accepted, snapshot| {
                let mut folds = accepted.effects().iter().filter_map(|effect| {
                    effect
                        .get(fold_effect())
                        .map(|range| (true, range))
                        .or_else(|| effect.get(unfold_effect()).map(|range| (false, range)))
                });
                let mut scroll = accepted
                    .effects()
                    .iter()
                    .filter_map(|effect| effect.get(&SCROLL_SNAPSHOT));
                accepted.changes().is_empty()
                    && accepted.selection() == expected_selection.as_ref()
                    && !accepted.add_to_history()
                    && !accepted.scroll_into_view()
                    && folds.next() == Some((folding, &range))
                    && folds.next().is_none()
                    && scroll.next() == snapshot.as_ref()
                    && scroll.next().is_none()
            },
            cx,
        )
    }

    pub(super) fn source_fold_ranges(&self) -> Vec<Range<usize>> {
        let folds = folded_ranges(self.view.state());
        if folds.is_empty() {
            return Vec::new();
        }
        let Some(fence) = self.source_fence() else {
            return Vec::new();
        };
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for fold in folds.iter(fence.body.start) {
            if fold.from >= fence.body.end {
                break;
            }
            let range = fold.from..fold.to;
            if !inside(&fence.body, &range) {
                continue;
            }
            if let Some(last) = ranges.last_mut()
                && range.start < last.end
            {
                if last.start == range.start {
                    last.end = last.end.max(range.end);
                }
                continue;
            }
            ranges.push(range);
        }
        ranges
    }

    pub(super) fn open_source_fold(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled || self.is_composing() || !self.source_fold_ranges().contains(&range)
        {
            return false;
        }
        self.change_source_folds([unfold_effect().of(range)], None, cx)
    }

    pub(super) fn reveal_folded_range(&mut self, revealed: &Range<usize>, cx: &mut Context<Self>) {
        let Some(fence) = self
            .source_fence()
            .filter(|fence| inside(&fence.body, revealed))
        else {
            return;
        };
        let folds = folded_ranges(self.view.state());
        let effects: Vec<_> = folds
            .iter(revealed.start)
            .take_while(|fold| fold.from < revealed.end)
            .filter_map(|fold| {
                let range = fold.from..fold.to;
                (inside(&fence.body, &range) && revealed.start < range.end)
                    .then(|| unfold_effect().of(range))
            })
            .collect();
        if !effects.is_empty() {
            self.change_source_folds(effects, None, cx);
        }
    }

    pub(super) fn unfold_at_deletion_edge(&mut self, right: bool, cx: &mut Context<Self>) -> bool {
        if !self.selection().is_empty() {
            return false;
        }
        self.source_fold_ranges()
            .into_iter()
            .find(|range| self.cursor() == if right { range.start } else { range.end })
            .is_some_and(|range| self.open_source_fold(range, cx))
    }

    pub(super) fn unfold_before_source_deletion(
        &mut self,
        transaction: &Transaction,
        cx: &mut Context<Self>,
    ) -> bool {
        let ranges: Vec<_> = transaction
            .changes()
            .changes()
            .map(|(range, _)| range)
            .collect();
        let effects: Vec<_> = self
            .source_fold_ranges()
            .into_iter()
            .filter_map(|fold| {
                let first = ranges.partition_point(|edit| edit.end <= fold.start);
                ranges
                    .get(first)
                    .is_some_and(|edit| edit.start < fold.end)
                    .then(|| unfold_effect().of(fold))
            })
            .collect();
        !effects.is_empty() && self.change_source_folds(effects, None, cx)
    }

    fn change_source_folds(
        &mut self,
        effects: impl IntoIterator<Item = StateEffect>,
        collapse_to: Option<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_source_fold_change(
            Transaction::default().with_effects(effects),
            collapse_to,
            |_, _| true,
            cx,
        )
    }

    fn apply_source_fold_change(
        &mut self,
        transaction: Transaction,
        collapse_to: Option<usize>,
        check: impl FnOnce(&Transaction, Option<ScrollSnapshot>) -> bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let snapshot = collapse_to
            .is_none()
            .then(|| self.capture_source_scroll())
            .flatten();
        let mut transaction = transaction
            .with_effects(snapshot.map(|snapshot| SCROLL_SNAPSHOT.of(snapshot)))
            .with_add_to_history(false);
        if let Some(position) = collapse_to {
            transaction = transaction.with_selection(EditorSelection::single(position, position));
        }
        let active = self.active.clone();
        if self
            .view
            .update_checked(
                transaction,
                active.clone(),
                |accepted| {
                    check(accepted, snapshot)
                        .then_some(())
                        .ok_or(EditError::InvalidRange)
                },
                |_| (None, active),
            )
            .is_err()
        {
            return false;
        }
        self.invalidate_edit_layout();
        self.reveal_caret = collapse_to.is_some();
        cx.notify();
        true
    }
}

fn fold_notice(
    state: &EditorState,
    lines: &Text,
    origin: usize,
    range: &Range<usize>,
    folding: bool,
) -> StateEffect {
    let from = lines
        .line_at(range.start - origin)
        .expect("fold start")
        .number;
    let to = lines.line_at(range.end - origin).expect("fold end").number;
    announce_fold(state, from..=to, folding)
}

fn inside(bounds: &Range<usize>, range: &Range<usize>) -> bool {
    bounds.start <= range.start && range.end <= bounds.end
}

#[cfg(test)]
mod tests;
