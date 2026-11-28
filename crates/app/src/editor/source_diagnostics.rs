//! Native lint presentation for editable source.

use std::{ops::Range, sync::Arc};

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, Context, FocusHandle, KeyBinding, MouseButton,
    Pixels, Rgba, Window, actions, div, fill, point, prelude::*, px, rgba, size,
};
use gpui_editor_codemirror::{
    Assoc, EditError, MapMode, Transaction,
    lint::{self, Diagnostic, LintState, LocatedDiagnostic, Severity},
    view::tooltips::{
        HoverSession, HoverSide, HoverTarget, Tooltip, TooltipId, close_hover_tooltip,
    },
};

use super::{Editor, Overlay, layout::Layout};
use crate::theme::Theme;

mod runner;
#[cfg(test)]
mod tests;

use runner::Runner;

actions!(
    source_diagnostics,
    [
        OpenDiagnostics,
        CloseDiagnostics,
        NextDiagnostic,
        PreviousDiagnostic
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    for context in ["Editor", "SourceDiagnostics"] {
        cx.bind_keys([
            KeyBinding::new("cmd-shift-m", OpenDiagnostics, Some(context)),
            KeyBinding::new("f8", NextDiagnostic, Some(context)),
            KeyBinding::new("shift-f8", PreviousDiagnostic, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("escape", CloseDiagnostics, Some("SourceDiagnostics")),
        KeyBinding::new("down", NextDiagnostic, Some("SourceDiagnostics")),
        KeyBinding::new("up", PreviousDiagnostic, Some("SourceDiagnostics")),
    ]);
}

pub(super) struct SourceDiagnostics {
    runner: Runner,
    focus: FocusHandle,
    focused_window: Option<AnyWindowHandle>,
    keyboard_hover: Option<(TooltipId, String)>,
}

impl SourceDiagnostics {
    pub(super) fn new(focus: FocusHandle) -> Self {
        Self {
            runner: Runner::default(),
            focus,
            focused_window: None,
            keyboard_hover: None,
        }
    }
}

impl Editor {
    fn apply_diagnostic_update(&mut self, transaction: Transaction) -> bool {
        let active = self.active.clone();
        let selection = transaction.selection().cloned();
        let effects = transaction.effects().len();
        self.view
            .update_checked(
                transaction.without_filtering().with_add_to_history(false),
                active.clone(),
                |resolved| {
                    if !resolved.changes().is_empty()
                        || resolved.selection() != selection.as_ref()
                        || resolved.effects().len() != effects
                        || resolved.add_to_history()
                    {
                        return Err(EditError::InvalidRange);
                    }
                    Ok(())
                },
                |_| (None, active),
            )
            .is_ok()
    }

    fn diagnostic_panel_open(&self) -> bool {
        LintState::get(self.view.state()).is_some_and(|lint| lint.panel_open())
    }

    fn restore_diagnostic_focus(&mut self, cx: &mut Context<Self>) {
        if self.diagnostic_panel_open() {
            return;
        }
        let Some(window) = self.diagnostics.focused_window.take() else {
            return;
        };
        let panel = self.diagnostics.focus.clone();
        let editor = self.focus.clone();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| {
                if panel.is_focused(window) {
                    window.focus(&editor);
                }
            });
        });
    }

    pub(super) fn source_diagnostic_marks(&self) -> Vec<LocatedDiagnostic> {
        let Some(scope) = self.current_diagnostic_scope() else {
            return Vec::new();
        };
        let source = self.source();
        let mut marks = Vec::new();
        lint::for_each_diagnostic(self.view.state(), |mark| {
            let valid = if scope.standalone {
                standalone_diagnostic_range_is_safe(source, &mark.range)
            } else {
                source.get(mark.range.clone()).is_some()
            };
            if scope.range.start <= mark.range.start && mark.range.end <= scope.range.end && valid {
                marks.push(mark);
            }
        });
        marks
    }

    #[cfg(target_os = "macos")]
    fn source_lsp_diagnostic_marks(&self) -> impl Iterator<Item = LocatedDiagnostic> + '_ {
        self.source_lsp_diagnostics()
            .iter()
            .map(|diagnostic| LocatedDiagnostic {
                range: diagnostic.range.clone(),
                diagnostic: Arc::new(diagnostic.clone()),
            })
    }

    #[cfg(not(target_os = "macos"))]
    fn source_lsp_diagnostic_marks(&self) -> impl Iterator<Item = LocatedDiagnostic> {
        std::iter::empty()
    }

    pub(super) fn source_diagnostic_paint_marks(&self) -> Vec<LocatedDiagnostic> {
        let mut marks = self.source_diagnostic_marks();
        marks.extend(self.source_lsp_diagnostic_marks());
        marks
    }

    pub(super) fn has_source_diagnostics(&self) -> bool {
        self.live_diagnostic_scope().is_some()
    }

    pub(super) fn open_source_diagnostics(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_source_diagnostics() {
            return false;
        }
        self.sync_source_diagnostics(cx);
        if let Some(open) = lint::open_lint_panel(self.view.state())
            && !self.apply_diagnostic_update(open)
        {
            return false;
        }
        self.run_diagnostics(true, cx);
        self.completion = None;
        self.diagnostics.focused_window = Some(window.window_handle());
        window.focus(&self.diagnostics.focus);
        cx.notify();
        true
    }

    fn close_source_diagnostics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(close) = lint::close_lint_panel(self.view.state()) {
            self.apply_diagnostic_update(close);
        }
        self.diagnostics.focused_window = None;
        window.focus(&self.focus);
        cx.notify();
    }

    pub(super) fn jump_source_diagnostic(
        &mut self,
        backward: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let marks = self.source_diagnostic_marks();
        if marks.is_empty() {
            return false;
        }
        let transaction = if backward {
            lint::previous_diagnostic
        } else {
            lint::next_diagnostic
        };
        let Some(edit) = transaction(self.view.state()) else {
            return false;
        };
        let Some(range) = edit.selection().map(|selection| selection.main().range()) else {
            return false;
        };
        let accepted = if let Some(mark) = marks.iter().find(|mark| mark.range == range) {
            self.pick_source_diagnostic(&mark.diagnostic, cx)
        } else {
            // Native navigation can target a segment of overlapping diagnostics.
            self.select_source_diagnostic(edit, range.clone(), cx)
        };
        if accepted && window.is_window_active() && self.focus.is_focused(window) {
            self.activate_keyboard_diagnostic_hover(range);
        }
        accepted
    }

    fn activate_keyboard_diagnostic_hover(&mut self, range: Range<usize>) {
        let state = self.view.state();
        let Ok(Some(content)) = lint::lint_tooltip_at(state, range.start, Assoc::After) else {
            return;
        };
        let Some(first) = content.diagnostics.into_iter().next() else {
            return;
        };
        let source = lint::lint_hover_tooltip();
        if source.active(state).is_none() {
            return;
        }
        let mut session = HoverSession::default();
        let Some(request) = session
            .activate_hover(
                state,
                HoverTarget {
                    pos: range.start,
                    side: HoverSide::After,
                },
                Some(source),
                Some(Arc::new(move |update| {
                    !update.changes().is_empty()
                        || !(range.start..=range.end)
                            .contains(&update.state().selection().main().head())
                })),
            )
            .pop()
        else {
            return;
        };
        let mut tip = Tooltip::new(content.range.start);
        tip.above = true;
        tip.end = Some(content.range.end);
        let id = tip.id();
        if let Some(effect) = session.complete(state, request, vec![tip])
            && self.apply_diagnostic_update(Transaction::default().with_effects([effect]))
        {
            self.diagnostics.keyboard_hover = Some((id, first.message));
        }
    }

    pub(super) fn dismiss_keyboard_diagnostic_hover(&mut self) -> bool {
        let Some((id, _)) = self.diagnostics.keyboard_hover.take() else {
            return false;
        };
        let source = lint::lint_hover_tooltip();
        source
            .active(self.view.state())
            .is_some_and(|active| active.tooltips().iter().any(|tip| tip.id() == id))
            && self.apply_diagnostic_update(
                Transaction::default().with_effects([close_hover_tooltip(source)]),
            )
    }

    fn keyboard_diagnostic_hover(
        &self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        window: &Window,
    ) -> Option<Overlay> {
        if !window.is_window_active() || !self.focus.is_focused(window) {
            return None;
        }
        let scope = self.current_diagnostic_scope()?;
        let (id, text) = self.diagnostics.keyboard_hover.as_ref()?;
        let state = self.view.state();
        let active = lint::lint_hover_tooltip().active(state)?;
        let tip = active.tooltips().iter().find(|tip| tip.id() == *id)?;
        if tip.pos < scope.range.start
            || tip.end.unwrap_or(tip.pos) > scope.range.end
            || layout.fold_hides(tip.pos)
        {
            return None;
        }
        let anchor = layout.bounds_at(tip.pos, false)?;
        viewport.intersects(&anchor).then(|| Overlay {
            text: text.clone(),
            anchor: Some(anchor),
            viewport,
            caret: layout.bounds_at_selection(state.selection().main(), self.upstream),
            prefer_above: tip.above,
        })
    }

    fn pick_source_diagnostic(
        &mut self,
        diagnostic: &Arc<Diagnostic>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(transaction) = lint::select_diagnostic(self.view.state(), diagnostic) else {
            return false;
        };
        let range = transaction
            .selection()
            .expect("diagnostic selection")
            .main()
            .range();
        self.select_source_diagnostic(transaction, range, cx)
    }

    fn select_source_diagnostic(
        &mut self,
        transaction: Transaction,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scope) = self.current_diagnostic_scope() else {
            return false;
        };
        if range.start < scope.range.start
            || range.end > scope.range.end
            || scope.standalone && !standalone_diagnostic_range_is_safe(self.source(), &range)
            || !self.apply_diagnostic_update(transaction)
        {
            return false;
        }
        self.completion = None;
        self.upstream = false;
        self.preferred_x = None;
        self.reveal_source(range.start, cx);
        cx.notify();
        true
    }

    fn run_source_diagnostic_action(
        &mut self,
        diagnostic: &Arc<Diagnostic>,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self
            .current_diagnostic_scope()
            .filter(|scope| !scope.standalone)
            .map(|scope| scope.range.clone())
        else {
            return;
        };
        if !self
            .source_diagnostic_marks()
            .iter()
            .any(|mark| Arc::ptr_eq(diagnostic, &mark.diagnostic))
        {
            return;
        }
        if let Ok(Some(transaction)) = lint::run_action(self.view.state(), diagnostic, index)
            && transaction.effects().is_empty()
            && let Ok(changes) = transaction.changes().desc(self.source().len())
            && let Some(end) = changes.map_pos(scope.end, Assoc::After, MapMode::Simple)
            && transaction.selection().is_none_or(|selection| {
                selection.ranges().iter().all(|range| {
                    let range = range.range();
                    scope.start <= range.start && range.end <= end
                })
            })
        {
            self.apply_source_transaction(transaction, scope, cx);
        }
    }

    pub(super) fn render_source_diagnostics(
        &self,
        window: &Window,
        cx: &Context<Self>,
        themed: bool,
    ) -> Option<AnyElement> {
        let scope = self.current_diagnostic_scope()?;
        if !self.diagnostic_panel_open() {
            return None;
        }
        let theme = Theme::for_editor(window, self.view.state());
        let dark = theme.view.is_some_and(|style| style.dark);
        let marks = self.source_diagnostic_marks();
        let selected = LintState::get(self.view.state()).and_then(|lint| lint.selected().cloned());
        let close = div()
            .debug_selector(|| "source-diagnostics-close".into())
            .cursor_pointer()
            .rounded_sm()
            .px_2()
            .py_1()
            .text_xs()
            .hover(|it| it.bg(theme.selected))
            .child("Close")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|editor, _, window, cx| editor.close_source_diagnostics(window, cx)),
            );
        #[cfg(test)]
        let close = super::one_dark_tests::panel_menu::probe(
            close,
            super::one_dark_tests::panel_menu::Role::DiagnosticsClose,
        );
        let panel = div()
            .id("source-diagnostics")
            .debug_selector(|| "source-diagnostics-panel".into())
            .key_context("SourceDiagnostics")
            .track_focus(&self.diagnostics.focus)
            .on_action(cx.listener(|editor, _: &CloseDiagnostics, window, cx| {
                editor.close_source_diagnostics(window, cx)
            }))
            .on_action(cx.listener(|editor, _: &NextDiagnostic, window, cx| {
                editor.jump_source_diagnostic(false, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &PreviousDiagnostic, window, cx| {
                editor.jump_source_diagnostic(true, window, cx);
            }))
            .flex()
            .flex_col()
            .flex_none()
            .when(!themed, |panel| {
                panel
                    .border_b(px(1.))
                    .border_color(theme.border)
                    .bg(theme.panel)
                    .text_color(theme.panel_text())
            })
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child("Diagnostics")
                    .child(close),
            )
            .child(
                div()
                    .id("source-diagnostics-results")
                    .overflow_y_scroll()
                    .max_h(px(160.))
                    .px_3()
                    .pb_2()
                    .when(marks.is_empty(), |panel| {
                        let message = match (self.diagnostics.runner.pending(), scope.stock_json) {
                            (true, true) => "Checking JSON…",
                            (true, false) => "Checking source…",
                            (false, _) => "No diagnostics",
                        };
                        panel.child(div().text_sm().text_color(theme.muted).child(message))
                    })
                    .children(marks.into_iter().enumerate().map(|(i, mark)| {
                        let diagnostic = mark.diagnostic;
                        let selected = selected
                            .as_ref()
                            .is_some_and(|mark| Arc::ptr_eq(&mark.diagnostic, &diagnostic));
                        let message = diagnostic.message.clone();
                        let severity = severity_name(diagnostic.severity);
                        let color = severity_color(diagnostic.severity, dark, theme);
                        let pick = diagnostic.clone();
                        div()
                            .flex()
                            .flex_col()
                            .rounded_sm()
                            .when(selected, |row| row.bg(theme.selected))
                            .child(
                                div()
                                    .debug_selector(move || format!("source-diagnostic-{i}"))
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .hover(|it| it.bg(theme.selected))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |editor, _, _, cx| {
                                            editor.pick_source_diagnostic(&pick, cx);
                                        }),
                                    )
                                    .child(div().text_xs().text_color(color).child(severity))
                                    .child(message),
                            )
                            .when(!scope.standalone && !diagnostic.actions.is_empty(), |row| {
                                row.child(
                                    div().flex().gap_1().px_2().children(
                                        diagnostic.actions.iter().enumerate().map(
                                            |(index, action)| {
                                                let diagnostic = diagnostic.clone();
                                                div()
                                                    .cursor_pointer()
                                                    .px_2()
                                                    .py_1()
                                                    .text_xs()
                                                    .text_color(theme.accent)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |editor, _, _, cx| {
                                                            editor.run_source_diagnostic_action(
                                                                &diagnostic,
                                                                index,
                                                                cx,
                                                            )
                                                        }),
                                                    )
                                                    .child(action.name.clone())
                                            },
                                        ),
                                    ),
                                )
                            })
                    })),
            );
        #[cfg(test)]
        let panel = super::one_dark_tests::panel_menu::probe(
            panel,
            super::one_dark_tests::panel_menu::Role::Diagnostics,
        );
        Some(panel.into_any_element())
    }

    pub(super) fn source_diagnostic_tooltip(
        &self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        window: &Window,
    ) -> Option<Overlay> {
        let pointer = window.mouse_position();
        // Leave the lint lane to its own delayed tooltip, even if a keyboard
        // tooltip is active in the document body.
        if layout.standalone_lint_lane_contains(pointer) {
            return None;
        }
        if let Some(hover) = self.keyboard_diagnostic_hover(layout, viewport, window) {
            return Some(hover);
        }
        if self.selecting || !viewport.contains(&pointer) {
            return None;
        }
        let at = layout.index_at(pointer);
        let state = self.view.state();
        let tooltip = |marks: Vec<LocatedDiagnostic>| {
            let mut hits = marks.into_iter().filter_map(|mark| {
                let anchor = marker_anchor(layout, mark.range.start)?;
                let point_hit = anchor.inset(px(-4.)).contains(&pointer);
                let range_hit = !mark.range.is_empty() && mark.range.contains(&at);
                (point_hit || range_hit).then_some((anchor, mark.diagnostic))
            });
            let (anchor, first) = hits.next()?;
            let candidates = std::iter::once(first)
                .chain(hits.map(|(_, diagnostic)| diagnostic))
                .map(|diagnostic| (*diagnostic).clone())
                .collect();
            let first = lint::filter_tooltip_diagnostics(state, candidates)
                .into_iter()
                .next()?;
            Some(Overlay {
                text: first.message,
                anchor: Some(anchor),
                viewport,
                caret: layout.bounds_at_selection(state.selection().main(), self.upstream),
                prefer_above: false,
            })
        };
        tooltip(self.source_diagnostic_marks()).or_else(|| {
            if !window.is_window_active() || !self.focus.is_focused(window) {
                return None;
            }
            tooltip(self.source_lsp_diagnostic_marks().collect())
        })
    }
}

fn standalone_diagnostic_range_is_safe(source: &str, range: &Range<usize>) -> bool {
    source.get(range.clone()).is_some()
        && [range.start, range.end]
            .into_iter()
            .all(|at| at == 0 || source.as_bytes().get(at - 1..=at) != Some(b"\r\n"))
}

fn marker_anchor(layout: &Layout, offset: usize) -> Option<Bounds<Pixels>> {
    if layout.fold_hides(offset) {
        return None;
    }
    let caret = layout.bounds_at(offset, false)?;
    Some(Bounds::new(
        point(caret.left() - px(2.), caret.bottom() - px(4.)),
        size(px(5.), px(3.)),
    ))
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
        Severity::Info => "Info",
        Severity::Hint => "Hint",
    }
}

fn severity_color(severity: Severity, dark: bool, theme: Theme) -> Rgba {
    match severity {
        Severity::Error => rgba(if dark { 0xe06c75ff } else { 0xb12537ff }),
        Severity::Warning => rgba(if dark { 0xe5c07bff } else { 0x795100ff }),
        Severity::Info => theme.accent,
        Severity::Hint => theme.muted,
    }
}

pub(super) fn paint_point_marks(
    layout: &Layout,
    marks: &[LocatedDiagnostic],
    theme: Theme,
    dark: bool,
    window: &mut Window,
) {
    let clip = window.content_mask().bounds;
    for mark in marks {
        let Some(anchor) = marker_anchor(layout, mark.range.start) else {
            continue;
        };
        if !clip.intersects(&anchor) {
            continue;
        }
        let color = severity_color(mark.diagnostic.severity, dark, theme);
        if mark.range.is_empty() {
            window.paint_quad(fill(anchor, color));
        }
        if let Some(frame) = &layout.source_frame {
            let origin = point(
                layout.origin.x + frame.bounds.left() + px(5.),
                anchor.top() - px(5.),
            );
            window.paint_quad(fill(Bounds::new(origin, size(px(3.), px(8.))), color));
        }
    }
}

pub(super) fn paint_range_marks(
    marks: &[LocatedDiagnostic],
    item: &super::layout::Item,
    row: Bounds<Pixels>,
    theme: Theme,
    dark: bool,
    window: &mut Window,
) {
    for mark in marks {
        for x in item.x_spans(mark.range.clone()) {
            let bounds = Bounds::new(
                point(row.left() + x.start, row.bottom() - px(3.)),
                size(x.end - x.start, px(2.)),
            );
            window.paint_quad(fill(
                bounds,
                severity_color(mark.diagnostic.severity, dark, theme),
            ));
        }
    }
}
