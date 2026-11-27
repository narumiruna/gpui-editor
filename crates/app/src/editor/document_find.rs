//! Find and replace in the current Markdown draft.

use std::{ops::Range, sync::Arc};

use gpui::{
    App, Bounds, Context, CursorStyle, Div, Edges, Entity, KeyBinding, PaintQuad, Pixels, Stateful,
    Subscription, Window, actions, div, fill, point, prelude::*, px, quad, rgba, size,
    transparent_black,
};
use gpui_editor_codemirror::{
    Change, ChangeDesc, ChangeSet, EditorSelection, HistoryIsolation, RangeSet, SelectionRange,
    StateEffect, Transaction,
    language::markdown::{MarkdownCodeLanguages, MarkdownCodeParser, configured_markdown_parser},
    search::{
        HighlightSelectionOptions, SCROLL_TO_OCCURRENCE, SearchOptions, SearchQuery,
        highlight_selection_matches, match_announcement, replacement_announcement,
        replacements_announcement, select_next_occurrence, select_selection_matches,
        selection_match_options,
    },
    view::{MarkDecoration, Viewport},
};

use super::{
    Editor,
    goto_line::OpenGotoLine,
    layout::{Item, ItemPaint},
};
use crate::{
    app::developer::{FindInput, TextField},
    theme::Theme,
};

actions!(
    document_find,
    [
        OpenFind,
        CloseFind,
        FindNext,
        FindPrevious,
        SelectNextOccurrence,
        SelectSelectionMatches
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    for context in ["Editor", "DocumentFind"] {
        cx.bind_keys([
            KeyBinding::new("cmd-f", OpenFind, Some(context)),
            KeyBinding::new("cmd-g", FindNext, Some(context)),
            KeyBinding::new("cmd-shift-g", FindPrevious, Some(context)),
            KeyBinding::new("f3", FindNext, Some(context)),
            KeyBinding::new("shift-f3", FindPrevious, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("escape", CloseFind, Some("DocumentFind")),
        KeyBinding::new("cmd-d", SelectNextOccurrence, Some("Editor")),
        KeyBinding::new("cmd-shift-l", SelectSelectionMatches, Some("Editor")),
    ]);
}

pub(super) struct FindPanel {
    query: Entity<TextField>,
    replacement: Entity<TextField>,
    options: SearchOptions,
    message: Option<String>,
    visible: bool,
    show_replacement: bool,
    marks: Option<FindMarks>,
    _subscriptions: [Subscription; 3],
}

#[derive(Clone, Copy)]
enum FindOption {
    Case,
    Word,
    Regex,
}

impl FindOption {
    fn value(self, options: &mut SearchOptions) -> &mut bool {
        match self {
            Self::Case => &mut options.case_sensitive,
            Self::Word => &mut options.whole_word,
            Self::Regex => &mut options.regexp,
        }
    }
}

impl FindPanel {
    fn new(cx: &mut Context<Editor>) -> Self {
        let colors = Editor::panel_input_colors(cx);
        let query = cx.new(|cx| {
            TextField::document_find("document-find-input", String::new(), colors.clone(), cx)
        });
        let replacement = cx.new(|cx| {
            TextField::document_find("document-replacement-input", String::new(), colors, cx)
        });
        let changed = cx.observe(&query, |editor, _, cx| {
            if let Some(find) = &mut editor.find {
                find.message = None;
                cx.notify();
            }
        });
        let submitted = cx.subscribe(&query, |editor, _, input, cx| {
            editor.find_match(matches!(input, FindInput::Previous), cx);
        });
        let replace = cx.subscribe(&replacement, |editor, _, input, cx| {
            if matches!(input, FindInput::Next) {
                editor.replace_match(cx);
            }
        });
        Self {
            query,
            replacement,
            options: SearchOptions::default(),
            message: None,
            visible: true,
            show_replacement: false,
            marks: None,
            _subscriptions: [changed, submitted, replace],
        }
    }

    fn is_composing(&self, cx: &App) -> bool {
        self.query.read(cx).is_composing() || self.replacement.read(cx).is_composing()
    }

    pub(super) fn set_editable(&self, enabled: bool, cx: &mut Context<Editor>) {
        self.query
            .update(cx, |field, cx| field.set_editable(enabled, cx));
        self.replacement
            .update(cx, |field, cx| field.set_editable(enabled, cx));
    }
}

impl Editor {
    #[cfg(test)]
    pub(crate) fn find_is_open(&self) -> bool {
        self.find.as_ref().is_some_and(|find| find.visible)
    }

    pub(crate) fn find_is_composing(&self, cx: &App) -> bool {
        self.find
            .as_ref()
            .is_some_and(|find| find.visible && find.is_composing(cx))
    }

    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled || self.is_composing() || self.find_is_composing(cx) {
            return;
        }
        let find = self.find.get_or_insert_with(|| FindPanel::new(cx));
        find.visible = true;
        find.query.read(cx).focus(window);
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_is_composing(cx) {
            return;
        }
        if let Some(find) = &mut self.find {
            find.visible = false;
            window.focus(&self.focus);
            cx.notify();
        }
    }

    pub(super) fn repeat_find(
        &mut self,
        previous: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .find
            .as_ref()
            .is_none_or(|find| find.query.read(cx).text().is_empty())
        {
            self.open_find(window, cx);
        } else {
            self.find_match(previous, cx);
        }
    }

    fn set_find_message(&mut self, message: Option<String>, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            find.message = message;
        }
        cx.notify();
    }

    fn search_query(&mut self, cx: &mut Context<Self>) -> Option<SearchQuery> {
        if !self.input_enabled || self.is_composing() || self.find_is_composing(cx) {
            return None;
        }
        let find = self.find.as_ref()?;
        let query = find.query.read(cx).text();
        if query.is_empty() {
            self.set_find_message(Some("Enter search text".into()), cx);
            return None;
        }
        match SearchQuery::new(query, find.options) {
            Ok(query) => Some(query),
            Err(error) => {
                self.set_find_message(Some(error.to_string().replace('\n', " ")), cx);
                None
            }
        }
    }

    fn find_match(&mut self, previous: bool, cx: &mut Context<Self>) {
        let Some(query) = self.search_query(cx) else {
            return;
        };
        self.find_with_query(&query, previous, None, cx);
    }

    fn find_with_query(
        &mut self,
        query: &SearchQuery,
        previous: bool,
        before_edit: Option<(Range<usize>, StateEffect)>,
        cx: &mut Context<Self>,
    ) {
        let state = self.state_with_source_words(query.options().whole_word);
        let cursor = query.in_state(&state);
        let selection = self.selection();
        let matched = if previous {
            cursor.find_previous(selection)
        } else {
            cursor.find_next(selection)
        };
        if let Ok(Some(matched)) = matched {
            let range = matched.range;
            let announcement = before_edit.filter(|(next, _)| next == &range).map_or_else(
                || match_announcement(self.view.state(), &range),
                |(_, effect)| effect,
            );
            self.select_match(range, announcement, cx);
        } else {
            self.set_find_message(Some("No other matches".into()), cx);
        }
    }

    pub(super) fn select_search_occurrences(&mut self, all: bool, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled || self.is_composing() || self.has_source_snippet() {
            return false;
        }
        let transaction = if all {
            select_selection_matches(self.view.state())
        } else {
            select_next_occurrence(&self.state_with_source_words(true))
        };
        transaction.is_some_and(|transaction| self.select_search_transaction(transaction, cx))
    }

    fn select_find_matches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.search_query(cx) else {
            return;
        };
        let Some(transaction) =
            query.select_matches(&self.state_with_source_words(query.options().whole_word))
        else {
            self.set_find_message(Some("No matches, or more than 1000 matches".into()), cx);
            return;
        };
        self.select_search_transaction(transaction, cx);
        self.set_find_message(None, cx);
        window.focus(&self.focus);
    }

    fn select_search_transaction(
        &mut self,
        mut transaction: Transaction,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(selection) = transaction.selection() else {
            return false;
        };
        if selection == self.view.state().selection() {
            return false;
        }
        let active = self
            .active
            .as_ref()
            .filter(|section| {
                selection.ranges().iter().all(|selected| {
                    let range = selected.range();
                    section.content.start <= range.start && range.end <= section.content.end
                })
            })
            .cloned();
        let occurrence = transaction
            .effects()
            .iter()
            .find_map(|effect| effect.get(&SCROLL_TO_OCCURRENCE).copied());
        let range = occurrence
            .and_then(|offset| {
                selection
                    .ranges()
                    .iter()
                    .find(|selected| selected.range().end == offset)
            })
            .map_or_else(|| selection.main().range(), |selected| selected.range());
        if active.is_some() {
            self.reveal_folded_range(&range, cx);
        } else if self.active.is_some() {
            self.layout = None;
        }
        if transaction.user_event().is_none() {
            transaction = transaction.with_user_event("select.search.occurrence");
        }
        self.record_selection_transaction(transaction, active, cx);
        self.upstream = false;
        self.preferred_x = None;
        self.reveal_source(occurrence.unwrap_or(range.start), cx);
        true
    }

    fn select_match(
        &mut self,
        range: Range<usize>,
        announcement: StateEffect,
        cx: &mut Context<Self>,
    ) {
        let leave_active = self.active.as_ref().is_some_and(|section| {
            range.start < section.content.start || range.end > section.content.end
        });
        let transaction = Transaction::default()
            .with_selection(EditorSelection::single(range.start, range.end))
            .with_effects([announcement])
            .with_user_event("select.search")
            .with_add_to_history(false)
            .isolate_history(HistoryIsolation::Before);
        let check = super::preserve_markdown_edit(&transaction, None);
        let active = (!leave_active).then(|| self.active.clone()).flatten();
        let Ok(active) = self
            .view
            .update_checked(transaction, self.active.clone(), check, |_| (None, active))
        else {
            self.set_find_message(Some("Could not select the match".into()), cx);
            return;
        };
        self.completion = None;
        self.active = active;
        if leave_active {
            // Clear source editing state without snapping the new Find selection.
            self.finish_active();
            self.layout = None;
        } else {
            self.reveal_folded_range(&range, cx);
        }
        self.upstream = false;
        self.preferred_x = None;
        self.reveal_source(range.start, cx);
        self.set_find_message(None, cx);
    }

    fn replace_match(&mut self, cx: &mut Context<Self>) {
        if self.view.state().read_only() {
            return;
        }
        let Some(query) = self.search_query(cx) else {
            return;
        };
        let selected = self.selection();
        let template = self.find.as_ref().unwrap().replacement.read(cx).text();
        let state = self.state_with_source_words(query.options().whole_word);
        let cursor = query.in_state(&state);
        let matched = cursor
            .cursor(selected.start..state.text().len())
            .ok()
            .and_then(|mut cursor| cursor.next())
            .filter(|matched| matched.range == selected && matched.precise);
        let before_edit = if let Some(matched) = matched {
            let replacement = query.replacement(&matched, template);
            let announcement = replacement_announcement(&state, selected.start);
            // Stock Replace Next describes the following match in the start state.
            let next = cursor
                .find_next(selected.clone())
                .ok()
                .flatten()
                .map(|next| {
                    let range = SelectionRange::new(next.range.start, next.range.end);
                    (range, match_announcement(&state, &next.range))
                });
            let Some(changes) =
                self.replace_find_selection(selected, &replacement, announcement, cx)
            else {
                self.set_find_message(Some("Cannot replace this match".into()), cx);
                return;
            };
            // Keep the app's post-edit Find navigation. Use the start-state
            // phrase only if it describes the match that navigation selects.
            next.and_then(|(range, effect)| Some((range.map(&changes).ok()?.range(), effect)))
        } else {
            None
        };
        self.find_with_query(&query, false, before_edit, cx);
    }

    fn replace_find_selection(
        &mut self,
        range: Range<usize>,
        text: &str,
        effect: StateEffect,
        cx: &mut Context<Self>,
    ) -> Option<ChangeDesc> {
        if !self.has_multiple_selections() {
            return self.replace_with_effect(range, text, effect, cx);
        }
        let state = self.view.state();
        let transaction = state
            .replace_selection(text)
            .ok()?
            .with_user_event("input")
            .with_scroll_into_view()
            .with_effects([effect]);
        let old_len = state.text().len();
        let mut receipt = None;
        if !self.apply_multi_edit_with_receipt(transaction, Some(&mut receipt), cx) {
            return None;
        }
        receipt?.resolved.changes().desc(old_len).ok()
    }

    fn replace_all_matches(&mut self, cx: &mut Context<Self>) {
        if self.view.state().read_only() {
            return;
        }
        let Some(query) = self.search_query(cx) else {
            return;
        };
        let replacement = self.find.as_ref().unwrap().replacement.read(cx).text();
        let replacements = query
            .in_state(&self.state_with_source_words(query.options().whole_word))
            .replacements(replacement);
        let Ok((changes, count)) = replacements else {
            self.set_find_message(Some("Cannot build the replacement".into()), cx);
            return;
        };
        if changes.is_empty() {
            self.set_find_message(Some("No replaceable matches".into()), cx);
            return;
        }
        let changes = if self.is_markdown() {
            let planned = self
                .markdown()
                .plan_replacements(changes.changes().map(|(range, text)| (range.clone(), text)));
            let Some(changes) = planned.ok().and_then(|planned| {
                ChangeSet::new(
                    planned
                        .into_iter()
                        .map(|(range, text)| Change::new(range, text))
                        .collect(),
                )
                .ok()
            }) else {
                self.set_find_message(
                    Some("Cannot replace all matches in this Markdown document".into()),
                    cx,
                );
                return;
            };
            changes
        } else {
            changes
        };
        self.finish_typing();
        let transaction = Transaction::new(changes)
            .with_effects([replacements_announcement(self.view.state(), count)])
            .with_user_event("input.replace.all");
        let check = super::preserve_markdown_edit(&transaction, None);
        if self
            .view
            .update_checked(transaction, self.active.clone(), check, |_| (None, None))
            .is_err()
        {
            self.set_find_message(Some("Could not apply the replacements".into()), cx);
            return;
        }
        self.active = None;
        self.invalidate_edit_layout();
        self.message = None;
        self.set_find_message(Some(format!("Replaced {count} matches")), cx);
    }

    fn toggle_find_option(
        &mut self,
        option: FindOption,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled || self.find_is_composing(cx) {
            return;
        }
        if let Some(find) = &mut self.find {
            let selected = option.value(&mut find.options);
            *selected = !*selected;
            find.message = None;
            find.query.read(cx).focus(window);
            cx.notify();
        }
    }

    pub(super) fn selection_match_marks(&self, focused: bool) -> Option<RangeSet<MarkDecoration>> {
        let state = self.view.state();
        let (options, use_word_scope) = match selection_match_options(state).ok()? {
            Some(options) if self.input_enabled => {
                let selection = state.selection();
                let word_match = if selection.main().is_empty() {
                    options.highlight_word_around_cursor
                } else {
                    options.whole_words
                };
                (options, word_match && selection.ranges().len() == 1)
            }
            // Without the extension, keep the editor's focused stock marks.
            None if focused => (HighlightSelectionOptions::default(), false),
            _ => return None,
        };
        let state = self.state_with_source_words(use_word_scope);
        highlight_selection_matches(&state, self.view.viewport(), options).ok()
    }

    pub(super) fn find_marks(&mut self, cx: &App) -> Option<Arc<RangeSet<MarkDecoration>>> {
        let find = self.find.as_ref().filter(|find| find.visible)?;
        let input = find.query.read(cx);
        if input.text().is_empty() || input.is_composing() {
            return None;
        }
        let query = input.text();
        let options = find.options;
        let word_scope = options.whole_word.then(|| self.word_scope()).flatten();
        let revision = self.view.revision();
        let viewport = self.view.viewport();
        // Facet-only changes can update word characters without changing the parser handle.
        let positional_wast = options.whole_word
            && configured_markdown_parser(self.view.state()).is_some_and(|config| {
                matches!(
                    config.code_languages,
                    Some(MarkdownCodeLanguages::Configured { .. })
                )
            });
        if !positional_wast
            && let Some(cache) = &find.marks
            && cache.query == query
            && cache.options == options
            && cache.word_scope == word_scope
            && cache.revision == revision
            && cache.viewport == *viewport
        {
            return Some(cache.marks.clone());
        }
        let mut ranges = Vec::new();
        let state = self.state_with_source_words(word_scope.is_some());
        if let Ok(query) = SearchQuery::new(query, options)
            && let Ok(matches) = query.in_state(&state).cursor(0..state.text().len())
        {
            let mark = MarkDecoration::new("cm-searchMatch");
            let end = viewport.interval().end;
            let visible = viewport.visible_ranges();
            for matched in matches.take_while(|matched| matched.range.start < end) {
                let range = matched.range;
                if !visible
                    .iter()
                    .any(|visible| visible.start < range.end && range.start < visible.end)
                    || ranges.last().is_some_and(
                        |last: &gpui_editor_codemirror::Range<MarkDecoration>| {
                            last.from == range.start && last.to == range.end
                        },
                    )
                {
                    continue;
                }
                if let Ok(range) = mark.clone().range(range) {
                    ranges.push(range);
                }
            }
        }
        let marks =
            Arc::new(RangeSet::of(ranges, true).expect("search produces ordered byte ranges"));
        self.find.as_mut()?.marks = Some(FindMarks {
            query: query.to_owned(),
            options,
            word_scope,
            revision,
            viewport: viewport.clone(),
            marks: marks.clone(),
        });
        Some(marks)
    }

    pub(super) fn render_find(&self, window: &Window, cx: &Context<Self>) -> Option<Div> {
        let find = self.find.as_ref().filter(|find| find.visible)?;
        let theme = Theme::for_editor(window, self.view.state());
        let enabled = self.input_enabled;
        let options = [
            ("document-find-case", "Match case", FindOption::Case),
            ("document-find-word", "Whole word", FindOption::Word),
            ("document-find-regex", "Regex", FindOption::Regex),
        ];
        let row = || div().flex().items_center().gap_2().min_w_0();
        Some(
            div()
                .key_context("DocumentFind")
                .debug_selector(|| "document-find-panel".into())
                .flex_none()
                .w_full()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .bg(theme.panel)
                .border_b(px(theme
                    .view
                    .map_or(1., |style| f32::from(style.panels.top_border.width_px))))
                .border_color(theme.border)
                .text_color(theme.panel_text())
                .text_sm()
                .on_action(
                    cx.listener(|editor, _: &OpenFind, window, cx| editor.open_find(window, cx)),
                )
                .on_action(
                    cx.listener(|editor, _: &CloseFind, window, cx| editor.close_find(window, cx)),
                )
                .on_action(cx.listener(|editor, _: &FindNext, _, cx| editor.find_match(false, cx)))
                .on_action(
                    cx.listener(|editor, _: &FindPrevious, _, cx| editor.find_match(true, cx)),
                )
                .on_action(cx.listener(|editor, _: &OpenGotoLine, window, cx| {
                    editor.open_goto_line(window, cx)
                }))
                .child(
                    row()
                        .child("Find")
                        .child(div().flex_1().min_w_0().child(find.query.clone()))
                        .child(
                            button("document-find-previous", "Previous", false, enabled, theme)
                                .on_click(
                                    cx.listener(|editor, _, _, cx| editor.find_match(true, cx)),
                                ),
                        )
                        .child(
                            button("document-find-next", "Next", false, enabled, theme).on_click(
                                cx.listener(|editor, _, _, cx| editor.find_match(false, cx)),
                            ),
                        )
                        .child(
                            button("document-find-close", "Close", false, true, theme).on_click(
                                cx.listener(|editor, _, window, cx| editor.close_find(window, cx)),
                            ),
                        ),
                )
                .child(
                    row()
                        .flex_wrap()
                        .child(
                            button(
                                "document-find-select-matches",
                                "Select all matches",
                                false,
                                enabled,
                                theme,
                            )
                            .on_click(cx.listener(
                                |editor, _, window, cx| editor.select_find_matches(window, cx),
                            )),
                        )
                        .children(options.map(|(id, label, option)| {
                            let mut values = find.options;
                            button(id, label, *option.value(&mut values), enabled, theme).on_click(
                                cx.listener(move |editor, _, window, cx| {
                                    editor.toggle_find_option(option, window, cx)
                                }),
                            )
                        }))
                        .child(
                            button(
                                "document-find-show-replacement",
                                "Replace…",
                                find.show_replacement,
                                enabled,
                                theme,
                            )
                            .on_click(cx.listener(
                                |editor, _, window, cx| {
                                    if !editor.input_enabled || editor.find_is_composing(cx) {
                                        return;
                                    }
                                    if let Some(find) = &mut editor.find {
                                        find.show_replacement = !find.show_replacement;
                                        if find.show_replacement {
                                            find.replacement.read(cx).focus(window);
                                        } else {
                                            find.query.read(cx).focus(window);
                                        }
                                        cx.notify();
                                    }
                                },
                            )),
                        ),
                )
                .when(find.show_replacement, |panel| {
                    panel.child(
                        row()
                            .child("Replace")
                            .child(div().flex_1().min_w_0().child(find.replacement.clone()))
                            .child(
                                button("document-find-replace", "Replace", false, enabled, theme)
                                    .on_click(
                                        cx.listener(|editor, _, _, cx| editor.replace_match(cx)),
                                    ),
                            )
                            .child(
                                button(
                                    "document-find-replace-all",
                                    "Replace All",
                                    false,
                                    enabled,
                                    theme,
                                )
                                .on_click(
                                    cx.listener(|editor, _, _, cx| editor.replace_all_matches(cx)),
                                ),
                            ),
                    )
                })
                .when_some(find.message.clone(), |panel, message| {
                    panel.child(
                        div()
                            .debug_selector(|| "document-find-message".into())
                            .text_color(theme.muted)
                            .truncate()
                            .child(message),
                    )
                }),
        )
    }
}

struct FindMarks {
    query: String,
    options: SearchOptions,
    word_scope: Option<(Range<usize>, MarkdownCodeParser)>,
    revision: u64,
    viewport: Viewport,
    marks: Arc<RangeSet<MarkDecoration>>,
}

/// Paint Find marks and the selections not already painted by those marks.
pub(super) fn paint_find_and_selection(
    marks: Option<&RangeSet<MarkDecoration>>,
    (primary, secondary): (&Range<usize>, &[Range<usize>]),
    item: &Item,
    row: Bounds<Pixels>,
    theme: Theme,
    window: &mut Window,
    #[cfg(test)] submitted: &mut Vec<PaintQuad>,
) {
    let mut paint = |quad: PaintQuad| {
        #[cfg(test)]
        let recorded = quad.clone();
        window.paint_quad(quad);
        #[cfg(test)]
        submitted.push(recorded);
    };

    let coverage = item.selection_source();
    let first = secondary.partition_point(|range| range.end <= coverage.start);
    let covered = secondary[first..].partition_point(|range| range.start < coverage.end);
    let visible_secondary = &secondary[first..first + covered];
    let mut painted_primary = false;
    let mut painted_secondary = Vec::new();
    if let Some(marks) = marks
        && !matches!(item.paint, ItemPaint::Fold(_))
    {
        marks.between(item.source.start, item.source.end, |matched| {
            let range = matched.from..matched.to;
            let is_primary = *primary == range;
            // The installed theme uses exact matches against all selection ranges.
            let secondary_index = theme.view.and_then(|_| {
                visible_secondary
                    .binary_search_by_key(&(matched.from, matched.to), |range| {
                        (range.start, range.end)
                    })
                    .ok()
            });
            let selected = is_primary || secondary_index.is_some();
            let mut painted = false;
            for x in item.x_spans(range).filter(|x| x.start < x.end) {
                let bounds = Bounds::new(
                    row.origin + point(x.start, px(0.)),
                    size(x.end - x.start, row.size.height),
                );
                if let Some(style) = theme.view {
                    let style = if selected {
                        style.selected_search_match
                    } else {
                        style.search_match
                    };
                    paint(fill(bounds, rgba(style.background.rgba32())));
                    let width = px(f32::from(style.outline.width_px));
                    // GPUI draws the border inside its quad. Expand only the outline quad.
                    paint(quad(
                        bounds.dilate(width),
                        px(0.),
                        transparent_black(),
                        Edges::all(width),
                        rgba(style.outline.color.rgba32()),
                        Default::default(),
                    ));
                } else {
                    let (background, stroke) = if is_primary {
                        (theme.selection, theme.accent)
                    } else {
                        (rgba(0xffce5a66), rgba(0x956000ff))
                    };
                    paint(quad(
                        bounds,
                        px(0.),
                        background,
                        Edges::all(px(1.)),
                        stroke,
                        Default::default(),
                    ));
                }
                painted = true;
            }
            if painted {
                painted_primary |= is_primary;
                painted_secondary.extend(secondary_index);
            }
            true
        });
    }
    for selected in (!painted_primary).then_some(primary).into_iter().chain(
        visible_secondary
            .iter()
            .enumerate()
            .filter_map(|(index, range)| (!painted_secondary.contains(&index)).then_some(range)),
    ) {
        for x in item.x_spans(selected.clone()) {
            paint(fill(
                Bounds::new(
                    row.origin + point(x.start, px(0.)),
                    size(x.end - x.start, row.size.height),
                ),
                theme.selection,
            ));
        }
    }
}

pub(super) fn paint_selection_matches(
    marks: &RangeSet<MarkDecoration>,
    find: Option<&RangeSet<MarkDecoration>>,
    item: &Item,
    row: Bounds<Pixels>,
    theme: Theme,
    window: &mut Window,
    #[cfg(test)] submitted: &mut Vec<PaintQuad>,
) {
    let background = theme
        .view
        .map_or(theme.selected, |style| rgba(style.selection_match.rgba32()));
    let mut paint = |range| {
        for x in item.x_spans(range).filter(|span| span.start < span.end) {
            let quad = fill(
                Bounds::new(
                    row.origin + point(x.start, px(0.)),
                    size(x.end - x.start, row.size.height),
                ),
                background,
            );
            #[cfg(test)]
            submitted.push(quad.clone());
            window.paint_quad(quad);
        }
    };
    marks.between(item.source.start, item.source.end, |matched| {
        let mut start = matched.from.max(item.source.start);
        let end = matched.to.min(item.source.end);
        if let Some(find) = find {
            // Find matches are disjoint and in source order. Only their
            // overlap makes a selection-match segment transparent upstream.
            find.between(start, end, |hit| {
                let gap_end = hit.from.min(end);
                if start < gap_end {
                    paint(start..gap_end);
                }
                start = start.max(hit.to);
                start < end
            });
        }
        if start < end {
            paint(start..end);
        }
        true
    });
}

fn button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    enabled: bool,
    theme: Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .px_2()
        .py_1()
        .rounded_md()
        .text_color(if enabled { theme.text } else { theme.muted })
        .bg(if selected {
            theme.selected
        } else {
            theme.toolbar
        })
        .when(enabled, |button| {
            button
                .cursor(CursorStyle::PointingHand)
                .hover(|style| style.bg(theme.selected))
        })
        .child(label)
}

#[cfg(test)]
mod announcements;
#[cfg(test)]
mod paint_tests;

#[cfg(test)]
mod tests {
    use gpui::{EntityInputHandler, TestAppContext};

    use crate::document::{BlockPreview, Section, SectionKind};

    use super::*;

    fn marks(editor: &Editor) -> Vec<Range<usize>> {
        editor
            .find
            .as_ref()
            .and_then(|find| find.marks.as_ref())
            .map(|cache| cache.marks.iter(0).map(|mark| mark.from..mark.to).collect())
            .unwrap_or_default()
    }

    #[gpui::test]
    fn search_marks_follow_projected_text_edits_and_undo(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("**word** and word".into(), window, cx));
        cx.update(|window, cx| {
            super::super::bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        cx.simulate_input("word");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(marks(editor), [2..6, 13..17]);
            assert!(
                editor
                    .view
                    .viewport()
                    .visible_ranges()
                    .iter()
                    .all(|range| range.start >= 2)
            );
        });

        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(Some(2..6), "other", window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert!(
                marks(editor.read(cx))
                    .into_iter()
                    .eq(std::iter::once(14..18))
            );
        });
        cx.simulate_keystrokes("escape secondary-z cmd-f");
        cx.run_until_parked();
        cx.update(|_, cx| assert_eq!(marks(editor.read(cx)), [2..6, 13..17]));
    }

    #[gpui::test]
    fn replace_all_treats_fences_as_literal_text_in_plain_files(cx: &mut TestAppContext) {
        let source = "```\r\nhit\r\n```\r\nhit\r\n";
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::new(source.into(), window, cx);
            editor.set_path(Some("sample.txt".into()), None);
            editor
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.open_find(window, cx);
                let find = editor.find.as_ref().unwrap();
                find.query.update(cx, |field, cx| {
                    field.replace_text_in_range(None, "hit", window, cx);
                });
                find.replacement.update(cx, |field, cx| {
                    field.replace_text_in_range(None, "new", window, cx);
                });
                editor.replace_all_matches(cx);
                assert_eq!(editor.source(), "```\r\nnew\r\n```\r\nnew\r\n");
                editor.replay_history(false, cx);
                assert_eq!(editor.source(), source);
            });
        });
    }

    #[gpui::test]
    fn whole_word_find_uses_the_active_language_and_refreshes_marks_on_scope_exit(
        cx: &mut TestAppContext,
    ) {
        let source = "alpha-beta\r\n\r\n```css\r\np { alpha-beta: x; content: alpha }\r\n```\r\n";
        let target = source.rfind("alpha").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            super::super::bind_keys(cx);
            editor.update(cx, |editor, cx| {
                let fence = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap()
                    .source;
                let head = source.find("p {").unwrap();
                editor.activate(
                    Section::new(SectionKind::DraftCode, fence.clone(), fence),
                    false,
                    cx,
                );
                editor.set_selection(head, head);
                editor.open_find(window, cx);
                editor.find.as_mut().unwrap().options.whole_word = true;
            });
        });
        cx.run_until_parked();
        cx.simulate_input("alpha");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let revision = cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.selection(), target..target + 5);
            assert!(
                marks(editor)
                    .into_iter()
                    .eq(std::iter::once(target..target + 5))
            );
            editor.view.revision()
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.finish_active();
                editor.layout = None;
                editor.set_selection(0, 0);
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.view.revision(), revision);
            assert!(marks(editor).contains(&(0..5)));
        });
    }

    #[gpui::test]
    fn whole_word_find_uses_embedded_language_at_each_match(cx: &mut TestAppContext) {
        let source =
            "字\r\n\r\n```html\r\n<script>\r\nconst $alpha = alpha;\r\n</script>\r\n```\r\n";
        let target = source.rfind("alpha").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            super::super::bind_keys(cx);
            editor.update(cx, |editor, cx| {
                let fence = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap()
                    .source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, fence.clone(), fence),
                    false,
                    cx,
                );
                let head = source.find("const").unwrap();
                editor.set_selection(head, head);
                editor.open_find(window, cx);
                editor.find.as_mut().unwrap().options.whole_word = true;
            });
        });
        cx.run_until_parked();
        cx.simulate_input("alpha");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.selection(), target..target + 5);
            assert_eq!(marks(editor), vec![target..target + 5]);
        });
    }

    #[gpui::test]
    fn only_matches_that_intersect_the_visible_source_ranges_are_kept(cx: &mut TestAppContext) {
        let source = "hit\n\n".repeat(80);
        let end = source.rfind("hit").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
        cx.simulate_resize(gpui::size(px(560.), px(360.)));
        cx.update(|window, cx| {
            super::super::bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        cx.simulate_input("hit");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let first = marks(editor.read(cx));
            assert_eq!(first.first(), Some(&(0..3)));
            assert!(!first.contains(&(end..end + 3)));
        });
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.reveal_source(end, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(editor.read(cx).scroll.offset().y < px(0.));
            window.refresh();
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let last = marks(editor.read(cx));
            assert!(last.contains(&(end..end + 3)));
            assert!(!last.contains(&(0..3)));
        });
    }

    #[gpui::test]
    fn find_field_leaves_enter_and_escape_to_active_composition(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("raw 中文".into(), window, cx));
        cx.update(|window, cx| {
            super::super::bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();

        let query = cx.update(|window, cx| {
            let field = editor.read(cx).find.as_ref().unwrap().query.clone();
            field.update(cx, |field, cx| {
                field.replace_and_mark_text_in_range(None, "中", None, window, cx)
            });
            assert!(editor.read(cx).find_is_composing(cx));
            field
        });
        cx.simulate_keystrokes("enter escape");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.find_is_open());
            assert!(editor.find_is_composing(cx));
            assert!(editor.selection().is_empty());
            assert_eq!(editor.source(), "raw 中文");
        });

        cx.update(|window, cx| {
            query.update(cx, |field, cx| {
                field.replace_text_in_range(None, "中", window, cx)
            });
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 4..7));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|_, cx| assert!(!editor.read(cx).find_is_open()));
    }
}
