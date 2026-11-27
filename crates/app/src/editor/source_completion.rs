//! Completion for editable source drafts in the document surface.

use std::{
    cell::Cell,
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Context, KeyBinding, KeyDownEvent, MouseButton,
    Pixels, Task, Window, actions, div, point, prelude::*, px, rgba, size,
};
use gpui_editor_codemirror::{
    Assoc, ChangeDesc, EditorState, MapMode,
    autocomplete::{
        insert_completion_text,
        session::{
            CompletionConfig, CompletionOptionId, CompletionQuery, CompletionSession,
            CompletionStatus, CompletionStep, SessionOption, SourceId, SourceResult,
            picked_completion,
        },
        start_snippet,
    },
    language::{
        markdown::{
            MarkdownCompletionSource, MarkdownDialect, MarkdownDocument, MarkdownOverlay,
            markdown_completion_source, markdown_completion_start, markdown_completions,
            markdown_snapshot,
        },
        sql::{NativeSqlSchemaExtension, SqlSchemaCompletionSource},
        xml::NativeXmlSchemaExtension,
    },
    language_data::{NativeLanguage, NativeSyntax},
    theme_one_dark::view_style,
    view::{ViewPlugin, ViewUpdate},
};

use super::{Editor, layout::Layout};
use crate::{
    document::{BlockPreview, Markdown, SectionKind},
    theme::Theme,
};

actions!(
    source_completion,
    [
        OpenCompletion,
        CloseCompletion,
        CompletionPageDown,
        CompletionPageUp
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-space", OpenCompletion, Some("Editor")),
        KeyBinding::new("escape", CloseCompletion, Some("Editor")),
        KeyBinding::new("pageup", CompletionPageUp, Some("Editor")),
        KeyBinding::new("pagedown", CompletionPageDown, Some("Editor")),
    ]);
}

const SOURCE: SourceId = SourceId(0);
const SECOND_SOURCE: SourceId = SourceId(1);
const MAX_VISIBLE: usize = 6;

#[derive(Clone, Copy, Eq, PartialEq)]
enum SourceLanguage {
    Prose,
    Fence(NativeLanguage),
}

#[derive(Clone)]
enum SourceSchema {
    Stock,
    Sql(Arc<SqlSchemaCompletionSource>),
    Xml(Arc<NativeXmlSchemaExtension>),
}

impl SourceSchema {
    fn for_language(language: SourceLanguage, state: &EditorState) -> Self {
        let registered = match language {
            SourceLanguage::Fence(NativeLanguage::Sql(dialect)) => {
                NativeSqlSchemaExtension::source_for(state, dialect).map(Self::Sql)
            }
            SourceLanguage::Fence(NativeLanguage::Xml) => {
                NativeXmlSchemaExtension::source_for(state).map(Self::Xml)
            }
            _ => None,
        };
        registered.unwrap_or(Self::Stock)
    }

    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Sql(first), Self::Sql(second)) => Arc::ptr_eq(first, second),
            (Self::Xml(first), Self::Xml(second)) => Arc::ptr_eq(first, second),
            (Self::Stock, Self::Stock) => true,
            _ => false,
        }
    }

    fn valid_boundary(&self, source: &str, position: usize) -> bool {
        matches!(self, Self::Stock) || valid_source_boundary(source, position)
    }
}

fn valid_source_boundary(source: &str, position: usize) -> bool {
    source.is_char_boundary(position)
        && !(source.as_bytes().get(position) == Some(&b'\n')
            && position > 0
            && source.as_bytes()[position - 1] == b'\r')
}

enum CompletionPick {
    Keyboard,
    Pointer(CompletionOptionId),
    Commit(char),
}

pub(super) struct SourceCompletion {
    session: CompletionSession,
    scope: Range<usize>,
    language: SourceLanguage,
    schema: SourceSchema,
    popup_id: Rc<()>,
    revision: u64,
    head: usize,
    started: Instant,
    timer: Option<(Duration, Task<()>)>,
}

impl SourceCompletion {
    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }

    fn has_current_schema(&self, state: &EditorState, head: usize) -> bool {
        self.schema
            .same_as(&SourceSchema::for_language(self.language, state))
            && self.schema.valid_boundary(state.text().as_str(), head)
    }

    fn maps_to(&self, changes: &ChangeDesc, scope: &Range<usize>) -> bool {
        changes.map_pos(self.scope.start, Assoc::Before, MapMode::Simple) == Some(scope.start)
            && changes.map_pos(self.scope.end, Assoc::After, MapMode::Simple) == Some(scope.end)
            && changes.changed_ranges().all(|(old, new)| {
                self.scope.start <= old.start
                    && old.end <= self.scope.end
                    && scope.start <= new.start
                    && new.end <= scope.end
            })
    }
}

struct AcceptedEdit {
    changes: ChangeDesc,
    selection_set: bool,
}

// Capture the accepted change after Markdown planning and transaction filtering.
#[derive(Clone, Default)]
struct CompletionEdit(Rc<Cell<Option<AcceptedEdit>>>);

impl ViewPlugin for CompletionEdit {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        if update.doc_changed() || update.selection_set {
            self.0.set(Some(AcceptedEdit {
                changes: update.changes.clone(),
                selection_set: update.selection_set,
            }));
        }
    }
}

impl Editor {
    fn completion_scope(&self) -> Option<(Range<usize>, SourceLanguage)> {
        if !self.input_enabled
            || self.is_composing()
            || !self.selection().is_empty()
            || self.has_source_snippet()
            || self.has_multiple_selections()
        {
            return None;
        }
        let head = self.cursor();
        let Some(fence) = self.source_fence() else {
            if self.active.is_some() {
                return None;
            }
            let state = self.view.state();
            let start = markdown_completion_start(state.text(), head)?;
            if markdown_snapshot(state).is_some()
                && state
                    .language_data_at::<MarkdownCompletionSource>(
                        "autocomplete",
                        head,
                        Assoc::Before,
                    )
                    .is_empty()
            {
                return None;
            }
            self.markdown()
                .plan_replacement(start..head, "<", None)
                .ok()?;
            return Some((0..self.source().len(), SourceLanguage::Prose));
        };
        let language = fence.language()?;
        (language.support().completion_source_capacity() != 0
            && (language != NativeLanguage::Xml || !self.view.state().read_only())
            && (fence.body.start..=fence.body.end).contains(&head))
        .then_some((fence.body, SourceLanguage::Fence(language)))
    }

    pub(super) fn edit_with_source_completion(
        &mut self,
        event: &str,
        edit: impl FnOnce(&mut Self, &mut Context<Self>) -> bool,
        cx: &mut Context<Self>,
    ) {
        let was_current = self.current_source_completion().is_some();
        let previous = self.completion.take().filter(|_| was_current);
        let capture = previous.as_ref().map(|_| {
            let capture = CompletionEdit::default();
            let plugin = self.view.add_plugin(capture.clone());
            (capture, plugin)
        });
        let revision = self.view.revision();
        let accepted = edit(self, cx);
        let report = capture.and_then(|(capture, plugin)| {
            self.view.remove_plugin(plugin);
            capture.0.take()
        });
        if !accepted && report.is_none() {
            self.completion = previous;
            return;
        }
        if let Some(mut current) = previous
            && self.view.revision() == revision.wrapping_add(1)
            && let Some(AcceptedEdit {
                changes,
                selection_set,
            }) = report
            && !changes.is_empty()
            && let Some((scope, language)) = self.completion_scope()
            && language == current.language
            && current.has_current_schema(self.view.state(), self.cursor())
            && current.maps_to(&changes, &scope)
        {
            let now = current.elapsed(cx.background_executor().now());
            if current
                .session
                .update(self.view.state(), &changes, selection_set, Some(event), now)
                .is_ok()
            {
                current.scope = scope;
                current.head = self.cursor();
                current.revision = self.view.revision();
                self.completion = Some(current);
                self.schedule_source_completion(cx);
                cx.notify();
                return;
            }
        }
        if accepted && self.view.revision() != revision && event == "input.type" {
            self.request_source_completion(false, cx);
        }
    }

    pub(super) fn request_source_completion(
        &mut self,
        explicit: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((scope, language)) = self.completion_scope() else {
            return false;
        };
        let now = cx.background_executor().now();
        if self.current_source_completion().is_none() {
            let schema = SourceSchema::for_language(language, self.view.state());
            if !schema.valid_boundary(self.source(), self.cursor()) {
                return false;
            }
            let count = match language {
                SourceLanguage::Fence(language) => language.support().completion_source_capacity(),
                SourceLanguage::Prose => 1,
            } + usize::from(matches!(schema, SourceSchema::Sql(_)));
            self.completion = Some(SourceCompletion {
                session: CompletionSession::new(
                    self.view.state(),
                    CompletionConfig::default(),
                    [SOURCE, SECOND_SOURCE].into_iter().take(count),
                ),
                scope,
                language,
                schema,
                popup_id: Rc::new(()),
                revision: self.view.revision(),
                head: self.cursor(),
                started: now,
                timer: None,
            });
        }
        let current = self.completion.as_mut().expect("source request");
        let elapsed = current.elapsed(now);
        current.session.start(explicit, elapsed);
        self.schedule_source_completion(cx);
        cx.notify();
        true
    }

    fn schedule_source_completion(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.completion.as_mut() else {
            return;
        };
        if current.session.status() == CompletionStatus::Inactive {
            self.completion = None;
            return;
        }
        let deadline = current.session.deadline();
        if deadline == current.timer.as_ref().map(|(due, _)| *due) {
            return;
        }
        current.timer = deadline.map(|due| {
            let executor = cx.background_executor();
            let timer = executor.timer(due.saturating_sub(current.elapsed(executor.now())));
            let task = cx.spawn(async move |editor, cx| {
                timer.await;
                let _ = editor.update(cx, Self::run_source_completion_timer);
            });
            (due, task)
        });
    }

    fn run_source_completion_timer(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.current_source_completion() else {
            self.completion = None;
            return;
        };
        let executor = cx.background_executor().clone();
        let elapsed = current.elapsed(executor.now());
        let (scope, language) = (current.scope.clone(), current.language);
        let schema = current.schema.clone();
        let current = self.completion.as_mut().expect("validated source request");
        current.timer = None;
        let queries = current.session.take_queries(elapsed);
        self.schedule_source_completion(cx);
        cx.notify();
        if queries.is_empty() {
            return;
        }
        // A rescheduled timer must not cancel other queries already in flight.
        cx.spawn(async move |editor, cx| {
            let results = executor
                .spawn(async move { source_options(queries, scope, language, &schema) })
                .await;
            let _ = editor.update(cx, |editor, cx| {
                editor.finish_source_completion_queries(results, cx);
            });
        })
        .detach();
    }

    fn finish_source_completion_queries(
        &mut self,
        results: Vec<(CompletionQuery, Option<SourceResult>)>,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.current_source_completion() else {
            if self.completion.as_ref().is_some_and(|current| {
                !current.has_current_schema(self.view.state(), self.cursor())
            }) {
                self.completion = None;
                cx.notify();
            }
            return;
        };
        let elapsed = current.elapsed(cx.background_executor().now());
        let current = self.completion.as_mut().expect("validated source request");
        let mut changed = false;
        for (query, result) in results {
            // The session rejects cancelled queries and results from older sessions.
            changed |= !matches!(
                current.session.finish_query(&query, result, elapsed),
                Ok(false)
            );
        }
        if changed {
            self.schedule_source_completion(cx);
            cx.notify();
        }
    }

    fn current_source_completion(&self) -> Option<&SourceCompletion> {
        let current = self.completion.as_ref()?;
        (current.head == self.cursor()
            && current.revision == self.view.revision()
            && current.has_current_schema(self.view.state(), current.head)
            && self.completion_scope().is_some_and(|(range, language)| {
                range == current.scope && language == current.language
            }))
        .then_some(current)
    }

    pub(super) fn close_source_completion(&mut self, cx: &mut Context<Self>) -> bool {
        if self.completion.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(super) fn move_source_completion(
        &mut self,
        down: bool,
        page: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(current) = self.current_source_completion() else {
            return false;
        };
        let elapsed = current.elapsed(cx.background_executor().now());
        let session = &mut self
            .completion
            .as_mut()
            .expect("validated source request")
            .session;
        let step = if page {
            CompletionStep::Page(MAX_VISIBLE)
        } else {
            CompletionStep::Option
        };
        if session.move_selection(down, step, elapsed) == Ok(true) {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(super) fn accept_source_completion(
        &mut self,
        option: Option<CompletionOptionId>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.pick_source_completion(
            option.map_or(CompletionPick::Keyboard, CompletionPick::Pointer),
            cx,
        )
    }

    pub(super) fn commit_source_completion(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        let mods = key.modifiers;
        if mods.platform || mods.function || mods.control && !(cfg!(windows) && mods.alt) {
            return;
        }
        let Some(text) = &key.key_char else {
            return;
        };
        let mut chars = text.chars();
        if let (Some(character), None) = (chars.next(), chars.next()) {
            self.pick_source_completion(CompletionPick::Commit(character), cx);
        }
    }

    fn pick_source_completion(&mut self, pick: CompletionPick, cx: &mut Context<Self>) -> bool {
        let Some(current) = self.current_source_completion() else {
            return false;
        };
        let option = match pick {
            CompletionPick::Pointer(option) => Some(option),
            _ => current.session.selected(),
        };
        let Some(option) = option else {
            return false;
        };
        let scope = current.scope.clone();
        let line_ending = if self.source()[..scope.start].ends_with("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let language = current.language;
        let apply = |state: &EditorState, choice: SessionOption<'_>| {
            let insert = choice.item.insert.as_deref().unwrap_or(&choice.item.label);
            if !current
                .schema
                .valid_boundary(state.text().as_str(), choice.range.start)
                || !current
                    .schema
                    .valid_boundary(state.text().as_str(), choice.range.end)
                || matches!(current.schema, SourceSchema::Xml(_))
                    && !self.preserves_completion_fence(&scope, &choice.range, insert)
            {
                return Err(gpui_editor_codemirror::EditError::InvalidRange);
            }
            let transaction = if let SourceLanguage::Fence(language) = language
                && let Some(text) = state.text().slice_owned(scope.clone())
                && let Ok(document) = language.support().parse(text)
                && let Some(snippet) =
                    document.completion_snippet(current.head - scope.start, choice.item)
            {
                let (mut transaction, session) =
                    snippet.insert_with_line_ending(state, choice.range, "  ", line_ending)?;
                if let Some(session) = session {
                    transaction = transaction.with_effects([start_snippet(session)]);
                }
                transaction.with_user_event("input.complete")
            } else {
                insert_completion_text(state, choice.range, insert)?
            };
            Ok(transaction.with_annotation(picked_completion().of(choice.item.clone())))
        };
        let result = match pick {
            CompletionPick::Keyboard => current.session.accept_with(
                option,
                current.elapsed(cx.background_executor().now()),
                apply,
            ),
            CompletionPick::Pointer(_) => current.session.accept_pointer_with(option, apply),
            CompletionPick::Commit(character) => {
                current.session.commit_with(option, character, apply)
            }
        };
        let Ok(Some(transaction)) = result else {
            return false;
        };
        let mut current = self.completion.take().expect("validated source completion");
        let length = self.view.state().text().len();
        let mut receipt = None;
        let applied = if language == SourceLanguage::Prose {
            self.apply_prose_transaction_with_receipt(transaction, Some(&mut receipt), cx)
        } else {
            self.apply_source_transaction_with_receipt(transaction, scope, Some(&mut receipt), cx)
        };
        if !applied {
            self.completion = Some(current);
            return false;
        }
        if let Some(receipt) = receipt
            && let Some((scope, language)) = self.completion_scope()
            && current.language == language
            && current.has_current_schema(self.view.state(), self.cursor())
            && let Ok(changes) = receipt.resolved.changes().desc(length)
            && current.maps_to(&changes, &scope)
        {
            let now = current.elapsed(cx.background_executor().now());
            if current.session.update_after_apply(
                self.view.state(),
                &receipt.proposed,
                &receipt.resolved,
                now,
            ) == Ok(true)
                && current.session.status() != CompletionStatus::Inactive
            {
                current.scope = scope;
                current.head = self.cursor();
                current.revision = self.view.revision();
                self.completion = Some(current);
                self.schedule_source_completion(cx);
            }
        }
        true
    }

    // Schema text or a custom insertion can contain a Markdown closing fence.
    // Require the same code block to span all its original and edited bytes.
    fn preserves_completion_fence(
        &self,
        scope: &Range<usize>,
        range: &Range<usize>,
        insert: &str,
    ) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        if active.source.start > scope.start
            || scope.end > active.source.end
            || range.start < scope.start
            || range.end > scope.end
            || self.source().get(range.clone()).is_none()
        {
            return false;
        }
        let mut draft = self.source().to_owned();
        draft.replace_range(range.clone(), insert);
        let expected = active.source.start..active.source.end - range.len() + insert.len();
        let closed = scope.end < active.source.end;
        Markdown::new(&draft).sections().iter().any(|section| {
            section.source == expected
                && if closed {
                    matches!(section.preview, Some(BlockPreview::Code { .. }))
                } else {
                    section.kind == SectionKind::DraftCode
                }
        })
    }

    pub(super) fn prepaint_source_completion(
        &self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let current = self.current_source_completion()?;
        if !self.focus.is_focused(window) {
            return None;
        }
        let anchor =
            layout.bounds_at_selection(self.view.state().selection().main(), self.upstream)?;
        let viewport = viewport.inset(px(6.));
        if !viewport.intersects(&anchor)
            || viewport.size.width < px(80.)
            || viewport.size.height < px(32.)
        {
            return None;
        }
        let options = current.session.options().ok()?;
        if options.is_empty() {
            return None;
        }
        let selected = current.session.selected();
        let first = selected
            .and_then(|id| options.iter().position(|option| option.id == id))
            .map_or(0, |index| (index + 1).saturating_sub(MAX_VISIBLE));
        let theme = Theme::for_editor(window, self.view.state());
        let surface = theme.view.map_or(theme.preview, |style| {
            rgba(style.tooltip.background.rgba32())
        });
        let active = theme.view.map_or(theme.selection, |style| {
            rgba(style.selected_completion.background.rgba32())
        });
        let installed = view_style(self.view.state());
        let foreground = if installed.is_some() {
            theme.on_tooltip
        } else {
            theme.text
        };
        let border = installed.map_or(Some((px(1.), theme.border)), |style| {
            style
                .tooltip
                .border
                .map(|stroke| (px(f32::from(stroke.width_px)), rgba(stroke.color.rgba32())))
        });
        let popup = div()
            .w(viewport.size.width.min(px(260.)))
            .max_h(viewport.size.height)
            .overflow_hidden()
            .py_1()
            .rounded_md()
            .when_some(border, |popup, (width, color)| {
                popup.border(width).border_color(color)
            })
            .text_sm()
            .text_color(foreground)
            .bg(surface)
            .children(options.iter().skip(first).take(MAX_VISIBLE).map(|option| {
                let id = option.id;
                let popup_id = current.popup_id.clone();
                let label = option
                    .item
                    .display_label
                    .as_ref()
                    .unwrap_or(&option.item.label);
                let row = div()
                    .debug_selector(|| format!("source-completion: {label}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |editor, _, _, cx| {
                            // A row can still receive a click before the new list is painted.
                            if editor
                                .completion
                                .as_ref()
                                .is_some_and(|current| Rc::ptr_eq(&popup_id, &current.popup_id))
                            {
                                editor.accept_source_completion(Some(id), cx);
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(Some(id) == selected, |row| {
                        row.bg(active).when_some(theme.view, |row, style| {
                            row.text_color(rgba(style.selected_completion.foreground.rgba32()))
                        })
                    })
                    .child(label.clone())
                    .when_some(option.item.detail.clone(), |row, detail| {
                        row.child(div().text_xs().child(detail))
                    });
                #[cfg(test)]
                let row = row.children(super::one_dark_tests::overlay_text_probe(
                    if Some(id) == selected {
                        super::one_dark_tests::OverlayText::SelectedCompletion
                    } else {
                        super::one_dark_tests::OverlayText::Completion
                    },
                ));
                #[cfg(test)]
                let row = super::one_dark_tests::panel_menu::probe(
                    row,
                    super::one_dark_tests::panel_menu::Role::CompletionOption {
                        label: label.clone(),
                        selected: Some(id) == selected,
                    },
                );
                row
            }));
        #[cfg(test)]
        let popup = super::one_dark_tests::panel_menu::probe(
            popup.debug_selector(|| "source-completion-menu".into()),
            super::one_dark_tests::panel_menu::Role::CompletionMenu,
        );
        let mut popup = popup.into_any_element();
        let size = popup.layout_as_root(
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            cx,
        );
        let below = anchor.bottom() + px(3.);
        let y = if below + size.height <= viewport.bottom() {
            below
        } else {
            anchor.top() - size.height - px(3.)
        };
        let origin = point(
            anchor.left().clamp(
                viewport.left(),
                (viewport.right() - size.width).max(viewport.left()),
            ),
            y.clamp(
                viewport.top(),
                (viewport.bottom() - size.height).max(viewport.top()),
            ),
        );
        popup.prepaint_at(origin, window, cx);
        Some(popup)
    }
}

fn source_options(
    queries: Vec<CompletionQuery>,
    scope: Range<usize>,
    language: SourceLanguage,
    schema: &SourceSchema,
) -> Vec<(CompletionQuery, Option<SourceResult>)> {
    let mut results =
        completion_results(&queries, scope.clone(), language, schema).unwrap_or_default();
    queries
        .into_iter()
        .map(|query| {
            let result = if query.is_cancelled() {
                None
            } else {
                let slot = usize::from(query.source() == SECOND_SOURCE);
                results[slot].take().map(|mut result| {
                    result.from += scope.start;
                    result.to = result.to.map(|offset| scope.start + offset);
                    result
                })
            };
            (query, result)
        })
        .collect()
}

fn completion_results(
    queries: &[CompletionQuery],
    scope: Range<usize>,
    language: SourceLanguage,
    schema: &SourceSchema,
) -> Option<[Option<SourceResult>; 2]> {
    let query = queries.iter().find(|query| !query.is_cancelled())?;
    let position = query.position.checked_sub(scope.start)?;
    let text = query.state().text().slice_owned(scope)?;
    match language {
        SourceLanguage::Fence(language) => {
            let document = language.support().parse(text).ok()?;
            if let (SourceSchema::Xml(source), NativeSyntax::Xml(xml)) = (schema, document.syntax())
            {
                let result = source
                    .complete(xml, position, query.explicit)
                    .filter(|result| {
                        valid_schema_result(schema, xml.text().as_str(), position, result)
                    });
                return Some([result, None]);
            }
            if let (SourceSchema::Sql(source), NativeSyntax::Sql(sql)) = (schema, document.syntax())
            {
                let explicit = |source| {
                    queries.iter().find_map(|query| {
                        (!query.is_cancelled() && query.source() == source)
                            .then_some(query.explicit)
                    })
                };
                let schema = explicit(SOURCE)
                    .and_then(|explicit| source.complete(sql, position, explicit))
                    .filter(|result| {
                        valid_schema_result(schema, sql.text().as_str(), position, result)
                    });
                let keywords = explicit(SECOND_SOURCE).and_then(|explicit| {
                    document
                        .indexed_completions(position, explicit)
                        .next()
                        .map(|(_, result)| result)
                });
                return Some([schema, keywords]);
            }
            let mut results = [None, None];
            for (slot, result) in document.indexed_completions(position, query.explicit) {
                results[slot] = Some(result);
            }
            Some(results)
        }
        SourceLanguage::Prose => {
            if markdown_snapshot(query.state()).is_some() {
                let result = markdown_completions(query.state(), position, query.explicit)
                    .into_iter()
                    .next();
                return Some([result, None]);
            }
            let doc = MarkdownDocument::parse_with_dialect(text, MarkdownDialect::Extended);
            if doc.overlays().any(|overlay| {
                matches!(overlay, MarkdownOverlay::Html { ranges } if ranges.iter().any(|range| {
                    range.start < position && position <= range.end
                }))
            }) {
                return None;
            }
            Some([
                markdown_completion_source(&doc, position, query.explicit),
                None,
            ])
        }
    }
}

fn valid_schema_result(
    schema: &SourceSchema,
    source: &str,
    position: usize,
    result: &SourceResult,
) -> bool {
    let to = result.to.unwrap_or(position);
    result.from <= to
        && schema.valid_boundary(source, result.from)
        && schema.valid_boundary(source, to)
}

#[cfg(test)]
mod sql_tests;
#[cfg(test)]
mod xml_tests;

#[cfg(test)]
mod tests {
    use gpui::{EntityInputHandler, Modifiers, TestAppContext};

    use super::*;
    use crate::{
        document::{BlockPreview, Section, SectionKind},
        editor::bind_keys,
    };

    const CSS: &str = "```css\r\np {\r\n    col\r\n}\r\n```\r\n\r\nUntouched.";

    fn activate_css(editor: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
        let position = editor.source().find("col").unwrap() + 3;
        activate_draft(editor, position, window, cx);
    }

    pub(super) fn activate_draft(
        editor: &mut Editor,
        position: usize,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) {
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
        editor.move_to(position, false, cx);
        window.focus(&editor.focus);
    }

    fn accept_named(
        editor: &mut Editor,
        label: &str,
        detail: Option<&str>,
        cx: &mut Context<Editor>,
    ) {
        let source = editor.current_source_completion().unwrap();
        let choice = source
            .session
            .options()
            .unwrap()
            .into_iter()
            .find(|option| option.item.label == label && option.item.detail.as_deref() == detail)
            .unwrap_or_else(|| panic!("missing completion {label:?} {detail:?}"));
        assert!(editor.accept_source_completion(Some(choice.id), cx));
    }

    #[gpui::test]
    fn css_completion_recovers_unclosed_calls_in_stylesheets_and_html(cx: &mut TestAppContext) {
        for (language, body) in [
            ("css", "a { --tone: red; color: var( /*😀*/ ; width: 4px; }"),
            (
                "html",
                "<div style=\"--tone: red; color: var( /*😀*/ ; width: 4px;\"></div>",
            ),
        ] {
            let source = format!("\u{732b}\r\n\r\n```{language}\r\n{body}\r\n```\r\n\r\nOutside.");
            let at = source.find("; width").unwrap();
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
            let clock = cx.update(|window, cx| {
                bind_keys(cx);
                editor.update(cx, |editor, cx| activate_draft(editor, at, window, cx));
                cx.background_executor().clone()
            });
            cx.simulate_keystrokes("ctrl-space");
            cx.run_until_parked();
            clock.advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            clock.advance_clock(Duration::from_millis(75));
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    accept_named(editor, "--tone", None, cx);
                    let mut updated = source.clone();
                    updated.insert_str(at, "--tone");
                    assert_eq!(editor.source(), updated, "{language}");
                    assert_eq!(editor.view.undo_depth(), 1);
                });
            });
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.source(), source, "{language}");
                assert_eq!(editor.cursor(), at);
            });
        }
    }

    #[gpui::test]
    fn a_commit_key_picks_before_typing_and_never_swallows_the_character(cx: &mut TestAppContext) {
        use gpui_editor_codemirror::autocomplete::session::CompletionItem;
        use gpui_editor_codemirror::{EditError, Transaction, transaction_filter};

        let draft = "```css\r\np { col }\r\n```\r\n";
        let from = draft.find("col").unwrap();
        let head = from + 3;
        for (override_chars, reject_pick, expected) in [
            (None, false, "color: red;"),
            (Some(Vec::new()), false, "col;"),
            (Some(vec![';']), true, "col;"),
        ] {
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
            cx.update(|window, cx| {
                bind_keys(cx);
                editor.update(cx, |editor, cx| {
                    activate_draft(editor, head, window, cx);
                    let filter = transaction_filter(move |_, tx| {
                        if reject_pick && tx.annotation(picked_completion()).is_some() {
                            Err(EditError::InvalidRange)
                        } else {
                            Ok(tx)
                        }
                    });
                    let active = editor.active.clone();
                    editor
                        .view
                        .update(
                            Transaction::default().with_effects([filter.append()]),
                            active.clone(),
                            |_| (None, active),
                        )
                        .unwrap();
                    let (scope, language) = editor.completion_scope().unwrap();
                    let mut session = CompletionSession::new(
                        editor.view.state(),
                        CompletionConfig {
                            activate_on_typing: false,
                            ..CompletionConfig::default()
                        },
                        [SOURCE],
                    );
                    session.start_completion(Duration::ZERO);
                    let now = Duration::from_millis(50);
                    let query = session.take_queries(now).pop().unwrap();
                    let mut item = CompletionItem::new("color");
                    item.insert = Some("color: red".into());
                    item.commit_characters = override_chars;
                    let mut result = SourceResult::new(from, vec![item]);
                    result.commit_characters = vec![';'];
                    session.finish_query(&query, Some(result), now).unwrap();
                    assert!(session.selected().is_some());
                    assert!(session.accept_selected(now).unwrap().is_none());
                    editor.completion = Some(SourceCompletion {
                        session,
                        scope,
                        language,
                        schema: SourceSchema::Stock,
                        popup_id: Rc::new(()),
                        revision: editor.view.revision(),
                        head,
                        started: cx.background_executor().now(),
                        timer: None,
                    });
                });
            });
            cx.simulate_keystrokes(";");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft.replace("col", expected)));
        }
    }

    #[gpui::test]
    fn a_picked_completion_restarts_queries_only_if_the_filter_preserves_it(
        cx: &mut TestAppContext,
    ) {
        use gpui_editor_codemirror::{
            Extension, Transaction, autocomplete::session::activate_on_completion,
            transaction_filter,
        };

        let draft = "```css\r\np { dis }\r\n```\r\n";
        let applied = draft.replace("dis", "display: ");
        let position = draft.find("dis").unwrap() + 3;
        for remove_pick in [false, true] {
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
            let executor = cx.update(|window, cx| {
                bind_keys(cx);
                editor.update(cx, |editor, cx| {
                    activate_draft(editor, position, window, cx);
                    let options = Extension::group([
                        activate_on_completion(|item| item.label == "display"),
                        transaction_filter(move |_, transaction| {
                            if remove_pick && transaction.annotation(picked_completion()).is_some()
                            {
                                Ok(Transaction::new(transaction.changes().clone())
                                    .with_selection(transaction.selection().unwrap().clone())
                                    .with_user_event("input.complete"))
                            } else {
                                Ok(transaction)
                            }
                        }),
                    ]);
                    let active = editor.active.clone();
                    editor
                        .view
                        .update(
                            Transaction::default().with_effects([options.append()]),
                            active.clone(),
                            |_| (None, active),
                        )
                        .unwrap();
                });
                cx.background_executor().clone()
            });
            cx.simulate_keystrokes("ctrl-space");
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(75));
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    accept_named(editor, "display", None, cx);
                    assert_eq!(editor.source(), applied);
                    assert_eq!(editor.view.undo_depth(), 1);
                    if remove_pick {
                        assert!(editor.current_source_completion().is_none());
                    } else {
                        let session = editor.current_source_completion().unwrap();
                        assert_eq!(session.session.status(), CompletionStatus::Pending);
                        assert!(session.timer.is_some());
                    }
                })
            });
            if !remove_pick {
                cx.run_until_parked();
                executor.advance_clock(Duration::from_millis(99));
                cx.run_until_parked();
                cx.update(|_, cx| {
                    let editor = editor.read(cx);
                    let session = editor.current_source_completion().unwrap();
                    assert_eq!(session.session.status(), CompletionStatus::Pending);
                });
                executor.advance_clock(Duration::from_millis(1));
                cx.run_until_parked();
                cx.update(|_, cx| {
                    let editor = editor.read(cx);
                    // The implicit query ran at the new caret and found no matches.
                    assert!(editor.current_source_completion().is_none());
                    assert_eq!(editor.source(), applied);
                });
            }
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
        }
    }

    #[gpui::test]
    fn go_and_python_snippets_and_embedded_completions_stay_inside_draft(cx: &mut TestAppContext) {
        for (draft, prefix, label, detail, text, selection) in [
            (
                "```go\npackage main\nfunc f() {\n  cas\n}\n```\n\nOutside.",
                "cas",
                "case",
                None,
                "case :\n  ",
                "",
            ),
            (
                "```py\ncla\n```\n\nOutside.",
                "cla",
                "class",
                Some("definition"),
                "class name:\n  def __init__(self, params):\n      ",
                "name",
            ),
            (
                "```html\n<script>const prior = 1; pri</script>\n```\n\nOutside.",
                "pri",
                "prior",
                None,
                "prior",
                "",
            ),
            (
                "```liquid\n{% end\n```\n\nOutside.",
                "end",
                "endif",
                None,
                "endif",
                "",
            ),
        ] {
            let start = draft.rfind(prefix).unwrap();
            let position = start + prefix.len();
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
            let executor = cx.update(|window, cx| {
                bind_keys(cx);
                editor.update(cx, |editor, cx| {
                    activate_draft(editor, position, window, cx)
                });
                cx.background_executor().clone()
            });
            cx.simulate_keystrokes("ctrl-space");
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(75));
            let mut expected = draft.to_owned();
            expected.replace_range(start..position, text);
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    accept_named(editor, label, detail, cx);
                    assert_eq!(editor.source(), expected);
                    assert_eq!(&editor.source()[editor.selection()], selection);
                })
            });
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
        }
    }

    #[gpui::test]
    fn typescript_snippet_preserves_crlf_and_selects_fields(cx: &mut TestAppContext) {
        let draft = "字\r\n\r\n```ts\r\n  inte\r\n```\r\n\r\nOutside.";
        let position = draft.find("inte").unwrap() + 4;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, position, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(75));
        let inserted = draft.replacen("inte", "interface name {\r\n    \r\n  }", 1);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                accept_named(editor, "interface", Some("definition"), cx);
                assert_eq!(editor.source(), inserted);
                assert_eq!(&editor.source()[editor.selection()], "name");
                assert!(editor.has_source_snippet());
            });
        });
        cx.simulate_input("模");
        cx.simulate_keystrokes("tab");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(!editor.has_source_snippet());
            assert_eq!(editor.source(), inserted.replacen("name", "模", 1));
            assert_eq!(
                editor.cursor(),
                editor.source().find("    \r\n").unwrap() + 4
            );
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), inserted));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), draft);
            assert!(!editor.view.can_undo());
        });
    }

    #[gpui::test]
    fn javascript_snippet_edits_and_deletes_linked_fields(cx: &mut TestAppContext) {
        let draft = "```js\nfo\n```\n\nOutside.";
        let position = draft.find("fo").unwrap() + 2;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, position, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(75));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                accept_named(editor, "for", Some("loop"), cx);
                let ranges = editor.view.state().selection().ranges();
                assert_eq!(ranges.len(), 3);
                assert!(
                    ranges
                        .iter()
                        .all(|range| &editor.source()[range.range()] == "index")
                );
            });
        });
        cx.simulate_input("éx");
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.source(),
                "```js\nfor (let é = 0; é < bound; é++) {\n  \n}\n```\n\nOutside."
            );
            assert_eq!(editor.view.state().selection().ranges().len(), 3);
        });
        cx.simulate_keystrokes("tab");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(&editor.source()[editor.selection()], "bound");
        });
        cx.simulate_keystrokes("shift-tab");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let ranges = editor.view.state().selection().ranges();
            assert_eq!(ranges.len(), 3);
            assert!(
                ranges
                    .iter()
                    .all(|range| &editor.source()[range.range()] == "é")
            );
        });
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(!editor.has_source_snippet());
            assert_eq!(editor.view.state().selection().ranges().len(), 1);
        });
    }

    #[gpui::test]
    fn markdown_tag_completion_uses_the_prose_range_and_configuration(cx: &mut TestAppContext) {
        use gpui_editor_codemirror::language::markdown::{MarkdownConfig, markdown};

        let source = "字 <di\r\n\r\nAfter\r\n";
        let position = source.find("<di").unwrap() + 3;
        for setting in [None, Some(true), Some(false)] {
            let (editor, cx) = cx.add_window_view(|window, cx| match setting {
                None => Editor::new(source.into(), window, cx),
                Some(complete_html_tags) => Editor::with_native_extensions(
                    source.into(),
                    markdown(MarkdownConfig {
                        complete_html_tags,
                        ..Default::default()
                    }),
                    window,
                    cx,
                )
                .unwrap(),
            });
            let executor = cx.update(|window, cx| {
                bind_keys(cx);
                editor.update(cx, |editor, _| {
                    editor.set_selection(position, position);
                    window.focus(&editor.focus);
                });
                cx.background_executor().clone()
            });
            cx.simulate_keystrokes("ctrl-space");
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(75));
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    if setting != Some(false) {
                        accept_named(editor, "<div", None, cx);
                        assert_eq!(editor.source(), "字 <div\r\n\r\nAfter\r\n");
                    } else {
                        assert!(editor.current_source_completion().is_none());
                    }
                });
            });
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        }
    }

    #[gpui::test]
    fn xml_completion_replaces_the_closing_name_without_duplicating_the_bracket(
        cx: &mut TestAppContext,
    ) {
        let draft = "```xml\r\n<item>字</it>\r\n```\r\n\r\nOutside.";
        let start = draft.find("</it>").unwrap() + 2;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, start + 2, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(75));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                accept_named(editor, "item>", None, cx);
                assert_eq!(editor.source(), draft.replace("</it>", "</item>"));
            });
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
    }

    #[gpui::test]
    fn javascript_local_names_share_the_native_completion_list(cx: &mut TestAppContext) {
        let draft = "```jsx\nconst needle = <Box/>;\nnee\n```\n\nOutside.";
        let position = draft.rfind("nee").unwrap() + 3;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, position, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(75));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                accept_named(editor, "needle", None, cx);
                assert_eq!(editor.source(), draft.replace("\nnee\n", "\nneedle\n"));
                assert!(!editor.has_source_snippet());
            });
        });
    }

    #[gpui::test]
    fn sql_keyword_completion_keeps_the_original_fence_and_undo(cx: &mut TestAppContext) {
        let source = "```sql\r\nselect name from items wher\r\n```\r\n\r\n字";
        let start = source.find("wher").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, start + 4, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(75));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let current = editor.current_source_completion().unwrap();
                let option = current
                    .session
                    .options()
                    .unwrap()
                    .into_iter()
                    .find(|option| option.item.label == "where")
                    .unwrap();
                assert_eq!(option.range, start..start + 4);
                assert!(editor.accept_source_completion(Some(option.id), cx));
                assert_eq!(editor.source(), source.replacen("wher", "where", 1));
                assert_eq!(editor.cursor(), start + 5);
            });
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }

    #[gpui::test]
    fn css_completion_accepts_only_the_draft_body_and_records_one_edit(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CSS.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                editor.move_to(CSS.find("col").unwrap() + 3, false, cx);
                assert!(!editor.request_source_completion(true, cx));
                activate_css(editor, window, cx);
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert!(
                editor
                    .read(cx)
                    .completion
                    .as_ref()
                    .unwrap()
                    .session
                    .options()
                    .unwrap()
                    .is_empty()
            )
        });
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        let color_index = cx.update(|_, cx| {
            let editor = editor.read(cx);
            let completion = editor.current_source_completion().unwrap();
            assert_eq!(editor.source(), CSS);
            completion
                .session
                .options()
                .unwrap()
                .iter()
                .position(|option| option.item.label == "color")
                .unwrap()
        });
        executor.advance_clock(Duration::from_millis(75));
        for _ in 0..color_index {
            cx.simulate_keystrokes("down");
        }
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), CSS.replacen("col", "color: ", 1));
            assert!(editor.completion.is_none());
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), CSS));
    }

    #[gpui::test]
    fn a_new_css_draft_offers_completions_and_escape_preserves_text(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
            cx.background_executor().clone()
        });
        cx.run_until_parked();
        cx.simulate_input("```");
        cx.simulate_keystrokes("left");
        cx.simulate_input("css");
        cx.simulate_keystrokes("right");
        cx.simulate_input("a { col }");
        cx.simulate_keystrokes("left left ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), "```css\na { col }\n```");
            let current = editor.current_source_completion().unwrap();
            assert_eq!(&editor.source()[current.scope.clone()], "a { col }");
            assert!(
                current
                    .session
                    .options()
                    .unwrap()
                    .iter()
                    .any(|option| option.item.label == "color")
            );
        });
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.completion.is_none());
            assert_eq!(editor.source(), "```css\na { col }\n```");
        });
    }

    #[gpui::test]
    fn a_click_on_a_visible_css_option_applies_it_without_moving_the_caret_elsewhere(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CSS.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_css(editor, window, cx));
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let current = editor.current_source_completion().unwrap();
            let now = current.elapsed(executor.now());
            assert!(current.session.accept_selected(now).unwrap().is_none());
        });
        let bounds = cx
            .debug_bounds("source-completion: color")
            .expect("visible completion row");
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), CSS.replacen("col", "color: ", 1));
            assert_eq!(editor.cursor(), CSS.find("col").unwrap() + "color: ".len());
            assert!(editor.completion.is_none());
        });
    }

    #[gpui::test]
    fn each_stylesheet_fence_uses_its_native_completion_source(cx: &mut TestAppContext) {
        for (info, body) in [
            ("less trailing-info", "a {\r\n  col\r\n}"),
            ("scss", "a {\r\n  col\r\n}"),
            ("Sass", "a\r\n  col"),
        ] {
            let source = format!("```{info}\r\n{body}\r\n```\r\n\r\nUnchanged.");
            let initial = source.clone();
            let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(initial, window, cx));
            let executor = cx.update(|window, cx| {
                bind_keys(cx);
                editor.update(cx, |editor, cx| activate_css(editor, window, cx));
                cx.background_executor().clone()
            });
            cx.simulate_keystrokes("ctrl-space");
            cx.run_until_parked();
            executor.advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            let first = cx.update(|_, cx| {
                let editor = editor.read(cx);
                let completion = editor.current_source_completion().expect(info);
                assert_eq!(&editor.source()[completion.scope.clone()], body, "{info}");
                completion
                    .session
                    .options()
                    .unwrap()
                    .iter()
                    .position(|option| option.item.label == "color")
                    .expect(info)
            });
            executor.advance_clock(Duration::from_millis(75));
            for _ in 0..first {
                cx.simulate_keystrokes("down");
            }
            cx.simulate_keystrokes("enter");
            cx.update(|_, cx| {
                assert_eq!(
                    editor.read(cx).source(),
                    source.replacen("col", "color: ", 1),
                    "{info}"
                )
            });
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source, "{info}"));
        }
    }

    #[gpui::test]
    fn html_completes_an_unfinished_tag_without_changing_the_closing_fence(
        cx: &mut TestAppContext,
    ) {
        let source = "```html\r\n<but\r\n```\r\n\r\nOutside";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, source.find("<but").unwrap() + 4, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        let index = cx.update(|_, cx| {
            editor
                .read(cx)
                .current_source_completion()
                .unwrap()
                .session
                .options()
                .unwrap()
                .iter()
                .position(|option| option.item.label == "button")
                .expect("button option")
        });
        executor.advance_clock(Duration::from_millis(75));
        for _ in 0..index {
            cx.simulate_keystrokes("down");
        }
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source.replace("<but", "<button")));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
    }

    #[gpui::test]
    fn css_valid_for_keeps_selection_and_maps_the_range_on_typing_and_backspace(
        cx: &mut TestAppContext,
    ) {
        let start = CSS.find("col").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CSS.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_css(editor, window, cx));
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        executor.advance_clock(Duration::from_millis(75));
        let selected = cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                let session = &mut editor.completion.as_mut().unwrap().session;
                let index = session
                    .options()
                    .unwrap()
                    .iter()
                    .position(|option| option.item.label == "background-color")
                    .unwrap();
                assert_ne!(index, 0);
                assert!(session.set_selected_completion(Some(index)).unwrap());
                session.selected().unwrap()
            })
        });
        cx.simulate_input("o");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let current = editor.current_source_completion().unwrap();
            assert_eq!(current.session.status(), CompletionStatus::Active);
            assert_eq!(current.session.deadline(), None);
            let option = current.session.selected_completion().unwrap().unwrap();
            assert_eq!(option.id, selected);
            assert_eq!(option.range, start..start + 4);
            assert_eq!(
                &editor.source()[current.scope.clone()],
                "p {\r\n    colo\r\n}"
            );
        });
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let current = editor.current_source_completion().unwrap();
            assert_eq!(current.session.deadline(), None);
            let option = current.session.selected_completion().unwrap().unwrap();
            assert_eq!(option.id, selected);
            assert_eq!(option.range, start..start + 3);
        });
        // No new interaction delay: the same list is ready for Enter.
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.source(),
                CSS.replacen("col", "background-color: ", 1)
            );
            assert!(editor.completion.is_none());
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), CSS));
    }

    #[gpui::test]
    fn invalid_css_prefix_schedules_an_explicit_refresh_in_the_same_session(
        cx: &mut TestAppContext,
    ) {
        let source = "```css\r\nbutton {}\r\n```\r\n\r\nOutside.";
        let head = source.find("button").unwrap() + "button".len();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_draft(editor, head, window, cx));
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        let (started, stale) = cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            let option = current.session.selected_completion().unwrap().unwrap();
            assert_eq!(option.item.label, "button");
            (current.started, option.id)
        });
        cx.simulate_input(":");
        executor.advance_clock(Duration::from_millis(49));
        cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            assert_eq!(current.started, started);
            assert_eq!(current.session.status(), CompletionStatus::Pending);
            assert!(current.session.options().unwrap().is_empty());
        });
        executor.advance_clock(Duration::from_millis(1));
        executor.advance_clock(Duration::from_millis(75));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let current = editor.current_source_completion().unwrap();
                assert_eq!(current.started, started);
                assert_eq!(current.session.status(), CompletionStatus::Active);
                assert!(!editor.accept_source_completion(Some(stale), cx));
                accept_named(editor, "hover", None, cx);
                assert_eq!(editor.source(), source.replace("button", "button:hover"));
            });
        });
    }

    #[gpui::test]
    fn a_late_result_that_fails_valid_for_runs_the_pending_refresh(cx: &mut TestAppContext) {
        let source = "```css\np {}\n```\n\nOutside.";
        let head = source.find("p {").unwrap() + 1;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, head, window, cx);
                assert!(editor.request_source_completion(true, cx));
                editor.completion.as_mut().unwrap().timer = None;
            });
            cx.background_executor().clone()
        });
        executor.advance_clock(Duration::from_millis(50));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.run_source_completion_timer(cx);
                editor.replace_text_in_range(None, ":", window, cx);
                let current = editor.current_source_completion().unwrap();
                assert_eq!(current.session.status(), CompletionStatus::Pending);
                assert_eq!(current.session.deadline(), None);
            });
        });
        // The initial result is stale. Replaying the edit must wake the host
        // again without another keystroke or a new explicit request.
        cx.run_until_parked();
        cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            assert_eq!(current.session.status(), CompletionStatus::Active);
            assert!(current.session.options().unwrap().iter().any(|option| {
                option.item.label == "hover" && option.range == (head + 1..head + 1)
            }));
        });
    }

    #[gpui::test]
    fn an_in_flight_request_cannot_publish_into_a_reopened_completion(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CSS.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_css(editor, window, cx);
                assert!(editor.request_source_completion(true, cx));
                // Hold the first deadline so the worker starts in the same UI
                // update that closes the session, before the worker can finish.
                editor.completion.as_mut().unwrap().timer = None;
            });
            cx.background_executor().clone()
        });
        executor.advance_clock(Duration::from_millis(50));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.run_source_completion_timer(cx);
                assert_eq!(editor.completion.as_ref().unwrap().session.deadline(), None);
                assert!(editor.close_source_completion(cx));
                assert!(editor.request_source_completion(true, cx));
            });
        });
        executor.advance_clock(Duration::from_millis(49));
        cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            assert_eq!(current.session.status(), CompletionStatus::Pending);
            assert!(current.session.options().unwrap().is_empty());
        });
        executor.advance_clock(Duration::from_millis(1));
        cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            assert_eq!(current.session.status(), CompletionStatus::Active);
        });
    }

    #[gpui::test]
    fn typed_css_completion_is_debounced_and_motion_and_ime_cancel(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CSS.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_css(editor, window, cx);
                editor.replace_text_in_range(None, "o", window, cx);
                assert!(editor.completion.is_some());
            });
            cx.background_executor().clone()
        });
        executor.advance_clock(Duration::from_millis(50));
        cx.simulate_input("r");
        executor.advance_clock(Duration::from_millis(99));
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert!(
                editor
                    .read(cx)
                    .completion
                    .as_ref()
                    .unwrap()
                    .session
                    .options()
                    .unwrap()
                    .is_empty()
            )
        });
        cx.simulate_keystrokes("left");
        executor.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert!(editor.completion.is_none());
                editor.replace_and_mark_text_in_range(None, "中", None, window, cx);
                assert!(editor.is_composing());
                assert!(!editor.request_source_completion(true, cx));
                assert!(editor.completion.is_none());
            });
        });
    }
}
