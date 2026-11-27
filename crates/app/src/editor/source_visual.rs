use std::ops::Range;

use gpui::{Bounds, Context, Pixels, px};
use gpui_editor_codemirror::{
    Assoc, EditorSelection, Text,
    commands::visual::{
        self, PageScrollGeometry, TextDirection, VerticalDistance, VisualCommandResult,
        VisualCursor, VisualDirection, VisualGeometry, VisualSelection,
    },
    view::ScrollStrategy,
};
use unicode_segmentation::GraphemeCursor;

use super::{Editor, RevealAlignment, SourceReveal, layout::Layout};

type Command = fn(
    &gpui_editor_codemirror::EditorState,
    &dyn VisualGeometry,
    &VisualSelection,
) -> VisualCommandResult;

struct DraftGeometry<'a> {
    layout: &'a Layout,
    text: &'a Text,
    scope: Range<usize>,
    folds: [Vec<Range<usize>>; 1],
    viewport: Bounds<Pixels>,
    scrolls: bool,
}

impl DraftGeometry<'_> {
    fn move_group_clusters(
        &self,
        start: VisualCursor,
        forward: bool,
        accept: &mut dyn FnMut(&str) -> bool,
    ) -> VisualCursor {
        let source = self.text.as_str();
        let mut scan = GraphemeCursor::new(start.head, source.len(), true);
        let mut at = start;
        // Each successful step crosses a nonempty cluster. The bound also
        // protects against inconsistent visual stops across adjacent rows.
        for index in 0..self.scope.len() {
            let (next, crossed) = if let Some((next, crossed)) =
                self.layout.bidi_step(self.text, at.as_caret(), forward)
            {
                scan.set_cursor(next.head());
                (VisualCursor::from_range(next), crossed)
            } else if self.layout.uses_bidi_carets(at.head) {
                break;
            } else {
                let boundary = if forward {
                    scan.next_boundary(source, 0)
                } else {
                    scan.prev_boundary(source, 0)
                };
                let Ok(Some(head)) = boundary else {
                    break;
                };
                let cluster = &source[head.min(at.head)..head.max(at.head)];
                (
                    VisualCursor {
                        assoc: Some(side(!forward)),
                        ..VisualCursor::new(head)
                    },
                    if cluster == "\r\n" { "\n" } else { cluster },
                )
            };
            if crossed.is_empty()
                || !(self.scope.start..=self.scope.end).contains(&next.head)
                || (!accept(crossed) && index > 0)
            {
                break;
            }
            at = next;
        }
        at
    }
}

impl VisualGeometry for DraftGeometry<'_> {
    fn move_vertical(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        distance: VerticalDistance,
    ) -> VisualCursor {
        let down = direction == VisualDirection::Forward;
        let page = (distance == VerticalDistance::Page).then(|| self.viewport.size.height - px(5.));
        let Some((target, goal)) = self.layout.vertical_caret(
            start.as_caret(),
            start.goal_column.map(Pixels::from),
            page,
            down,
        ) else {
            return start;
        };
        if !(self.scope.start..=self.scope.end).contains(&target.head()) {
            return if distance == VerticalDistance::Page {
                VisualCursor::new(target.head().clamp(self.scope.start, self.scope.end))
            } else {
                start
            };
        }
        VisualCursor {
            goal_column: Some(f64::from(goal)),
            ..VisualCursor::from_range(target)
        }
    }

    fn line_boundary(&self, start: VisualCursor, direction: VisualDirection) -> VisualCursor {
        let Some((head, upstream)) = self.layout.row_edge(
            start.head,
            start.assoc == Some(Assoc::Before),
            direction == VisualDirection::Forward,
        ) else {
            return start;
        };
        VisualCursor {
            assoc: Some(side(upstream)),
            ..VisualCursor::new(head.clamp(self.scope.start, self.scope.end))
        }
    }

    fn line_block(&self, head: usize) -> Option<Range<usize>> {
        let line = self.layout.line_block(head)?;
        Some(line.start.max(self.scope.start)..line.end.min(self.scope.end))
    }

    fn unwrapped_line_boundary(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
    ) -> Option<VisualCursor> {
        let block = self.line_block(start.head)?;
        let (head, assoc) = match direction {
            VisualDirection::Backward => (block.start, Assoc::Before),
            VisualDirection::Forward => (block.end, Assoc::After),
        };
        Some(VisualCursor {
            assoc: Some(assoc),
            ..VisualCursor::new(head)
        })
    }

    fn text_direction_at(&self, _: usize) -> Option<TextDirection> {
        Some(TextDirection::LeftToRight)
    }

    fn move_by_char(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        accept: Option<&mut dyn FnMut(&str) -> bool>,
    ) -> Option<VisualCursor> {
        let forward = direction == VisualDirection::Forward;
        if let Some(accept) = accept {
            return Some(self.move_group_clusters(start, forward, accept));
        }
        let next = self
            .layout
            .bidi_step(self.text, start.as_caret(), forward)
            .map(|(next, _)| VisualCursor::from_range(next))
            .unwrap_or_else(|| {
                let head = self.layout.step(start.head, forward);
                if head == start.head {
                    start
                } else {
                    VisualCursor {
                        assoc: Some(side(!forward)),
                        ..VisualCursor::new(head)
                    }
                }
            });
        Some(
            if (self.scope.start..=self.scope.end).contains(&next.head) {
                next
            } else {
                start
            },
        )
    }

    fn page_scroll_geometry(&self, head: usize) -> Option<PageScrollGeometry> {
        if !self.scrolls {
            return None;
        }
        let caret = self.layout.bounds_at(head, false)?;
        Some(PageScrollGeometry {
            scroll_top: self.viewport.top().into(),
            scroll_bottom: self.viewport.bottom().into(),
            caret_top: caret.top().into(),
            caret_bottom: caret.bottom().into(),
        })
    }

    fn atomic_ranges(&self) -> &[Vec<Range<usize>>] {
        &self.folds
    }
}

fn side(upstream: bool) -> Assoc {
    if upstream {
        Assoc::Before
    } else {
        Assoc::After
    }
}

impl Editor {
    pub(super) fn move_in_source_columns(
        &mut self,
        right: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let command = match (right, extend) {
            (false, false) => visual::cursor_char_left,
            (true, false) => visual::cursor_char_right,
            (false, true) => visual::select_char_left,
            (true, true) => visual::select_char_right,
        };
        self.run_source_visual(command, false, cx)
    }

    pub(super) fn move_to_source_row_edge(
        &mut self,
        right: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let command = match (right, extend) {
            (false, false) => visual::cursor_line_boundary_left,
            (true, false) => visual::cursor_line_boundary_right,
            (false, true) => visual::select_line_boundary_left,
            (true, true) => visual::select_line_boundary_right,
        };
        self.run_source_visual(command, false, cx)
    }

    pub(super) fn move_in_source_groups(
        &mut self,
        right: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let command = match (right, extend) {
            (false, false) => visual::cursor_group_left,
            (true, false) => visual::cursor_group_right,
            (false, true) => visual::select_group_left,
            (true, true) => visual::select_group_right,
        };
        self.run_source_visual(command, true, cx)
    }

    pub(super) fn move_in_source_rows(
        &mut self,
        down: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let command = match (down, extend) {
            (false, false) => visual::cursor_line_up,
            (true, false) => visual::cursor_line_down,
            (false, true) => visual::select_line_up,
            (true, true) => visual::select_line_down,
        };
        self.run_source_visual(command, false, cx)
    }

    pub(super) fn page_in_source_rows(
        &mut self,
        down: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let command = match (down, extend) {
            (false, false) => visual::cursor_page_up,
            (true, false) => visual::cursor_page_down,
            (false, true) => visual::select_page_up,
            (true, true) => visual::select_page_down,
        };
        self.run_source_visual(command, false, cx)
    }

    pub(super) fn add_source_cursor(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        self.run_source_visual(
            if down {
                visual::add_cursor_below
            } else {
                visual::add_cursor_above
            },
            false,
            cx,
        )
    }

    fn source_visual_context(&self) -> Option<(DraftGeometry<'_>, VisualSelection)> {
        if !self.input_enabled || self.is_composing() || self.has_source_snippet() {
            return None;
        }
        let scope = self.source_fence()?.body;
        let layout = self.layout.as_ref()?;
        let state = self.view.state();
        let selection = state.selection();
        if selection.ranges().iter().any(|range| {
            let span = range.range();
            span.start < scope.start || span.end > scope.end
        }) {
            return None;
        }
        let cursors = selection
            .ranges()
            .iter()
            .enumerate()
            .map(|(index, &range)| {
                let main = index == selection.main_index();
                VisualCursor {
                    head: range.head(),
                    assoc: if main && !layout.uses_bidi_carets(range.head()) {
                        Some(side(self.upstream))
                    } else {
                        range.assoc()
                    },
                    bidi_level: range.bidi_level(),
                    goal_column: self
                        .preferred_x
                        .and_then(|x| range.goal_column().or_else(|| main.then(|| f64::from(x)))),
                }
            })
            .collect();
        let selection = VisualSelection::new(selection, cursors).ok()?;
        let geometry = DraftGeometry {
            layout,
            text: state.text(),
            scope,
            folds: [self.source_fold_ranges()],
            viewport: self.scroll.bounds(),
            scrolls: self.scroll.max_offset().height > px(2.),
        };
        Some((geometry, selection))
    }

    pub(super) fn delete_to_source_row_edge(
        &mut self,
        right: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((geometry, selection)) = self.source_visual_context() else {
            return false;
        };
        let delete = if right {
            visual::delete_line_boundary_forward
        } else {
            visual::delete_line_boundary_backward
        };
        let Ok(Some(transaction)) = delete(self.view.state(), &geometry, &selection) else {
            return false;
        };
        let scope = geometry.scope;
        if transaction
            .changes()
            .changes()
            .any(|(range, _)| range.start < scope.start || range.end > scope.end)
        {
            return true;
        }
        if self.unfold_before_source_deletion(&transaction, cx) {
            return true;
        }
        if self.apply_source_transaction(transaction, scope, cx) {
            self.reveal_caret = true;
        }
        true
    }

    fn run_source_visual(&mut self, command: Command, words: bool, cx: &mut Context<Self>) -> bool {
        let Some((geometry, selection)) = self.source_visual_context() else {
            return false;
        };
        let state = self.state_with_source_words(words);
        let update = match command(&state, &geometry, &selection) {
            Ok(Some(update)) => update,
            Ok(None) => return words,
            Err(_) => return false,
        };
        let Some(proposed) = update.transaction.selection() else {
            return false;
        };
        let main = update.selection.cursors()[proposed.main_index()];
        let ranges = proposed
            .ranges()
            .iter()
            .zip(update.selection.cursors())
            .map(|(&range, cursor)| {
                EditorSelection::range(
                    range.anchor(),
                    range.head(),
                    cursor.goal_column,
                    cursor.bidi_level,
                    cursor.assoc,
                )
            })
            .collect();
        let Ok(next) = EditorSelection::new(ranges, proposed.main_index()) else {
            return false;
        };
        self.record_selection_transaction(
            update.transaction.with_selection(next),
            self.active.clone(),
            cx,
        );
        self.upstream = main.assoc == Some(Assoc::Before) && self.cursor() == main.head;
        self.preferred_x = main.goal_column.map(Pixels::from);
        self.reveal_caret = true;
        if let Some(target) = self.view.take_scroll_target()
            && target.y == ScrollStrategy::Start
        {
            self.reveal_source = Some(SourceReveal {
                offset: target.range.head(),
                alignment: RevealAlignment::Start(Pixels::from(target.y_margin)),
            });
        }
        cx.notify();
        true
    }
}
