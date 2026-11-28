use std::ops::Range;

use gpui::{
    App, Context, Div, Entity, KeyBinding, ScrollStrategy, UniformListScrollHandle, Window,
    actions, div, prelude::*, px, uniform_list,
};
use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Text, Transaction,
    merge::{Chunk, CollapseConfig, MergeProjection, ProjectionRow, Side},
};

use super::{Editor, Redo, Undo};
use crate::theme::Theme;

mod syntax;

use syntax::{ReviewSyntax, ReviewText};

actions!(editor, [CloseMergeReview]);

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseMergeReview, Some("MergeReview")),
        KeyBinding::new("secondary-z", Undo, Some("MergeReview")),
        KeyBinding::new("secondary-shift-z", Redo, Some("MergeReview")),
    ]);
}

pub(super) struct MergeReview {
    saved: Text,
    draft: Text,
    saved_syntax: ReviewSyntax,
    draft_syntax: ReviewSyntax,
    revision: u64,
    chunks: Vec<Chunk>,
    projection: MergeProjection,
    current: usize,
    scroll: UniformListScrollHandle,
    message: Option<&'static str>,
}

impl MergeReview {
    fn new(saved: Text, draft: Text, revision: u64) -> Self {
        let chunks = Chunk::build(&saved, &draft);
        let projection =
            MergeProjection::new(&saved, &draft, &chunks, Some(CollapseConfig::default()));
        Self {
            saved,
            draft,
            saved_syntax: ReviewSyntax::default(),
            draft_syntax: ReviewSyntax::default(),
            revision,
            chunks,
            projection,
            current: 0,
            scroll: UniformListScrollHandle::new(),
            message: None,
        }
    }

    fn select(&mut self, index: usize) {
        let Some(chunk) = self.chunks.get(index) else {
            return;
        };
        self.current = index;
        self.message = None;
        if let Some(row) = [Side::B, Side::A].into_iter().find_map(|side| {
            let start = chunk.range(side).start;
            self.projection.rows(side).iter().position(
                |row| matches!(row, ProjectionRow::Line { range, .. } if range.start == start),
            )
        }) {
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    fn navigate(&mut self, forward: bool) {
        let count = self.chunks.len();
        if count == 0 {
            return;
        }
        let step = if forward { 1 } else { count - 1 };
        self.select((self.current + step) % count);
    }
}

impl Editor {
    pub(crate) fn is_reviewing_changes(&self) -> bool {
        self.merge_review.is_some()
    }

    pub(crate) fn review_changes(
        &mut self,
        saved: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled
            || !self.is_markdown()
            || self.is_composing()
            || self.source() == saved
        {
            return;
        }
        self.finish_typing();
        self.cancel_source_snippet(cx);
        self.dismiss_keyboard_diagnostic_hover();
        self.invalidate_edit_layout();
        self.merge_review = Some(MergeReview::new(
            Text::new(saved),
            self.view.state().text().clone(),
            self.view.revision(),
        ));
        self.merge_review
            .as_mut()
            .expect("review installed")
            .select(0);
        window.focus(&self.focus);
        cx.notify();
    }

    pub(crate) fn dismiss_merge_review(&mut self, cx: &mut Context<Self>) {
        if self.merge_review.take().is_some() {
            self.invalidate_edit_layout();
            cx.notify();
        }
    }

    fn restore_review_chunk(&mut self, revision: u64, index: usize, cx: &mut Context<Self>) {
        if !self.input_enabled || self.is_composing() || revision != self.view.revision() {
            return;
        }
        let Some(review) = self
            .merge_review
            .as_ref()
            .filter(|review| review.revision == revision)
        else {
            return;
        };
        let Some(chunk) = review.chunks.get(index) else {
            return;
        };
        let range = chunk.range(Side::B);
        let replacement = review
            .saved
            .slice(chunk.range(Side::A))
            .expect("valid saved chunk");
        let Ok((planned_range, text)) =
            self.markdown()
                .plan_replacement(range.clone(), replacement, None)
        else {
            self.merge_review
                .as_mut()
                .expect("review installed")
                .message = Some("This change touches a read-only section.");
            cx.notify();
            return;
        };
        if planned_range != range || text != replacement {
            self.merge_review
                .as_mut()
                .expect("review installed")
                .message = Some("This change cannot be restored without changing its boundaries.");
            cx.notify();
            return;
        }
        let changes =
            ChangeSet::single(Change::new(range, replacement)).expect("valid chunk replacement");
        let edit = Transaction::new(changes).with_user_event("revert");
        if self
            .view
            .update(edit, self.active.clone(), |_| (None, None))
            .is_ok()
        {
            self.active = None;
            self.invalidate_edit_layout();
            cx.notify();
        }
    }

    pub(super) fn render_merge_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let theme = Theme::for_editor(window, self.view.state());
        let review = self.merge_review.as_mut().expect("review installed");
        if review.revision != self.view.revision() {
            let mut next = MergeReview::new(
                review.saved.clone(),
                self.view.state().text().clone(),
                self.view.revision(),
            );
            next.saved_syntax = std::mem::take(&mut review.saved_syntax);
            next.current = review.current.min(next.chunks.len().saturating_sub(1));
            next.scroll = review.scroll.clone();
            next.select(next.current);
            *review = next;
        }
        let state = self.view.state();
        review.saved_syntax.prepare(&review.saved, state);
        review.draft_syntax.prepare(&review.draft, state);
        let count = review.chunks.len();
        let message = review.message;
        let revision = review.revision;
        let current = review.current;
        let enabled = self.input_enabled && count != 0;
        let handle = review.scroll.clone();
        let rows = review
            .projection
            .rows(Side::A)
            .len()
            .max(review.projection.rows(Side::B).len());
        let editor = cx.entity();
        let list = uniform_list("merge-review-rows", rows, move |visible, _, cx| {
            let editor_ref = editor.read(cx);
            let Some(review) = &editor_ref.merge_review else {
                return Vec::new();
            };
            let renderer = ReviewRows {
                review,
                editor: &editor,
                state: editor_ref.view.state(),
                theme,
            };
            visible.map(|row| renderer.row(row)).collect()
        });
        let prev = cx.listener(|editor, _, _, cx| {
            if let Some(review) = &mut editor.merge_review {
                review.navigate(false);
                cx.notify();
            }
        });
        let next = cx.listener(|editor, _, _, cx| {
            if let Some(review) = &mut editor.merge_review {
                review.navigate(true);
                cx.notify();
            }
        });
        div()
            .id("merge-review")
            .debug_selector(|| "merge-review".into())
            .size_full()
            .flex()
            .flex_col()
            .key_context("MergeReview")
            .track_focus(&self.focus)
            .bg(theme.surface)
            .text_color(theme.text)
            .on_action(
                cx.listener(|editor, _: &CloseMergeReview, _, cx| editor.dismiss_merge_review(cx)),
            )
            .on_action(cx.listener(|editor, _: &Undo, _, cx| editor.replay_history(false, cx)))
            .on_action(cx.listener(|editor, _: &Redo, _, cx| editor.replay_history(true, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .bg(theme.toolbar)
                    .child(div().flex_1().text_sm().child(if count == 0 {
                        "No changes".to_owned()
                    } else {
                        format!("Change {} of {count}", current + 1)
                    }))
                    .child(
                        review_button("Previous", "merge-previous", enabled, theme)
                            .when(enabled, |b| b.on_click(prev)),
                    )
                    .child(
                        review_button("Next", "merge-next", enabled, theme)
                            .when(enabled, |b| b.on_click(next)),
                    )
                    .child(
                        review_button("Restore saved change", "merge-restore", enabled, theme)
                            .when(enabled, |b| {
                                b.on_click(cx.listener(move |editor, _, _, cx| {
                                    editor.restore_review_chunk(revision, current, cx)
                                }))
                            }),
                    )
                    .child(
                        review_button("Close", "merge-close", true, theme).on_click(
                            cx.listener(|editor, _, _, cx| editor.dismiss_merge_review(cx)),
                        ),
                    ),
            )
            .when_some(message, |view, message| {
                view.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .bg(theme.error_bg)
                        .child(message),
                )
            })
            .child(
                div()
                    .flex()
                    .bg(theme.panel)
                    .text_sm()
                    .child(column_label("Saved"))
                    .child(column_label("Draft")),
            )
            .child(list.track_scroll(handle).flex_1().min_h_0().w_full())
    }
}

fn column_label(label: &'static str) -> Div {
    div().w_1_2().px_2().py_1().child(label)
}

fn review_button(
    label: &'static str,
    id: &'static str,
    enabled: bool,
    theme: Theme,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .rounded_md()
        .px_2()
        .py_1()
        .text_sm()
        .text_color(if enabled { theme.text } else { theme.muted })
        .when(enabled, |button| {
            button.hover(|style| style.bg(theme.selected))
        })
        .child(label)
}

struct ReviewRows<'a> {
    review: &'a MergeReview,
    editor: &'a Entity<Editor>,
    state: &'a EditorState,
    theme: Theme,
}

impl ReviewRows<'_> {
    fn row(&self, index: usize) -> Div {
        div()
            .flex()
            .w_full()
            .h(px(24.))
            .child(self.cell(Side::A, index))
            .child(self.cell(Side::B, index))
    }

    fn cell(&self, side: Side, index: usize) -> Div {
        let theme = self.theme;
        let cell = div()
            .w_1_2()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .border_r_1()
            .border_color(theme.border);
        let Some(row) = self.review.projection.rows(side).get(index) else {
            return cell;
        };
        match row {
            ProjectionRow::Spacer { .. } => cell.bg(theme.panel),
            ProjectionRow::Collapsed { lines, range } => {
                let editor = self.editor.clone();
                let start = range.start;
                let revision = self.review.revision;
                cell.bg(theme.panel).child(
                    div()
                        .id((text_side_id(side), index))
                        .debug_selector(move || format!("merge-expand-{side:?}-{index}"))
                        .px_2()
                        .text_sm()
                        .text_color(theme.accent)
                        .on_click(move |_, _, cx| {
                            editor.update(cx, |editor, cx| {
                                if let Some(review) = &mut editor.merge_review
                                    && review.revision == revision
                                    && review.projection.expand(side, start)
                                {
                                    cx.notify();
                                }
                            });
                        })
                        .child(format!("Show {} unchanged lines", lines.len())),
                )
            }
            ProjectionRow::Line { number, range } => self.line(cell, side, *number, range.clone()),
        }
    }

    fn line(&self, cell: Div, side: Side, number: usize, range: Range<usize>) -> Div {
        let theme = self.theme;
        let (source, syntax) = if side == Side::A {
            (&self.review.saved, &self.review.saved_syntax)
        } else {
            (&self.review.draft, &self.review.draft_syntax)
        };
        let text = source.slice(range.clone()).expect("valid projected line");
        let text = ReviewText::new(text, syntax.styles(&range, self.state));
        #[cfg(test)]
        let text = text.probe(side, number);
        let changed = self
            .review
            .chunks
            .partition_point(|chunk| chunk.range(side).end <= range.start);
        let changed = self.review.chunks.get(changed).is_some_and(|chunk| {
            let chunk = chunk.range(side);
            chunk.start <= range.start && range.start < chunk.end
        });
        cell.flex()
            .items_center()
            .text_sm()
            .whitespace_nowrap()
            .when(changed, |cell| {
                cell.bg(if side == Side::A {
                    theme.error_bg
                } else {
                    theme.selected
                })
            })
            .child(
                div()
                    .w(px(54.))
                    .flex_none()
                    .px_2()
                    .text_color(theme.muted)
                    .child(number.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .font_family(super::layout::CODE_FONT_FAMILY)
                    .child(text),
            )
    }
}

fn text_side_id(side: Side) -> &'static str {
    if side == Side::A {
        "merge-expand-saved"
    } else {
        "merge-expand-draft"
    }
}

#[cfg(test)]
mod tests;
