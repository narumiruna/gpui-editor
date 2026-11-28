//! One GPUI input surface backed by offsets in the Markdown source.

use std::{
    borrow::Cow,
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    app::developer::FieldColors,
    document::{Heading, Markdown, Section, SectionKind},
    theme::Theme,
    workspace::{Workspace, is_markdown},
};
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Context, CursorStyle, ElementInputHandler,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollHandle, Size, Task, UTF16Selection,
    Window, actions, canvas, div, fill, point, prelude::*, px, quad, rgb, rgba, size,
};

use gpui_editor_codemirror::{
    Change, ChangeDesc, ChangeSet, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, SelectionRange, StateEffect, Transaction, TransactionCoordinates,
    autocomplete::snippet_state_extension,
    commands::bindings::TabFocusMode,
    language::{brackets::bracket_matching, fold::code_folding},
    lint,
    theme_one_dark::view_style,
    view::{
        EditorView, EndScrollPadding, ViewException, Viewport, filter_clipboard_input,
        input::{InputChange, InputHandling, run_input_handlers},
        log_exception,
        selection_layers::rectangular_selection,
    },
};

#[cfg(target_os = "macos")]
pub(crate) mod announcements;
mod commands;
mod document_find;
mod draw_selection;
mod drop_cursor;
mod goto_line;
mod highlight;
mod layout;
mod merge_review;
#[cfg(test)]
mod one_dark_tests;
#[cfg(test)]
mod placeholder_tests;
#[cfg(test)]
mod scroll_past_end_tests;
mod source_brackets;
pub mod source_collab;
mod source_completion;
mod source_diagnostics;
mod source_fence;
mod source_folds;
mod source_keys;
#[cfg(target_os = "macos")]
pub mod source_lsp;
mod source_markdown;
mod source_multi;
mod source_rectangle;
mod source_scroll;
mod source_snippets;
mod source_tab_focus;
pub mod source_tooltip;
mod source_visual;
use source_completion::{
    CloseCompletion, CompletionPageDown, CompletionPageUp, OpenCompletion, SourceCompletion,
};
use source_diagnostics::{NextDiagnostic, OpenDiagnostics, PreviousDiagnostic, SourceDiagnostics};
use source_fence::SourceSyntax;
use source_folds::ToggleSourceFold;
use source_keys::{
    IndentSourceSelection, InsertSourceBlankLine, SelectSourceParent, SourceIndent,
    ToggleSourceComment,
};
pub(crate) mod toolbar;
#[cfg(test)]
use commands::Command;
pub(crate) use document_find::OpenFind;
use document_find::{
    FindNext, FindPanel, FindPrevious, SelectNextOccurrence, SelectSelectionMatches,
};
use goto_line::{GotoLinePanel, OpenGotoLine};
use layout::{DEFAULT_LINE_HEIGHT, Graphic, ItemPaint, Layout};
use merge_review::MergeReview;
pub(crate) use toolbar::ToolbarAssets;

actions!(
    editor,
    [
        Left,
        Right,
        SelectLeft,
        SelectRight,
        Up,
        Down,
        SelectUp,
        SelectDown,
        AddSourceCursorAbove,
        AddSourceCursorBelow,
        SelectSourcePageUp,
        SelectSourcePageDown,
        SelectAll,
        Backspace,
        Delete,
        Newline,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        UndoSelection,
        RedoSelection,
        ToggleEditorTabFocus
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-[", ToggleSourceFold, Some("Editor")),
        KeyBinding::new("cmd-i", SelectSourceParent, Some("Editor")),
        KeyBinding::new("cmd-alt-\\", IndentSourceSelection, Some("Editor")),
        KeyBinding::new("alt-shift-m", ToggleEditorTabFocus, Some("Editor")),
        KeyBinding::new("left", Left, Some("Editor")),
        KeyBinding::new("right", Right, Some("Editor")),
        KeyBinding::new("shift-left", SelectLeft, Some("Editor")),
        KeyBinding::new("shift-right", SelectRight, Some("Editor")),
        KeyBinding::new("up", Up, Some("Editor")),
        KeyBinding::new("down", Down, Some("Editor")),
        KeyBinding::new("shift-up", SelectUp, Some("Editor")),
        KeyBinding::new("shift-down", SelectDown, Some("Editor")),
        KeyBinding::new("cmd-alt-up", AddSourceCursorAbove, Some("Editor")),
        KeyBinding::new("cmd-alt-down", AddSourceCursorBelow, Some("Editor")),
        KeyBinding::new("shift-pageup", SelectSourcePageUp, Some("Editor")),
        KeyBinding::new("shift-pagedown", SelectSourcePageDown, Some("Editor")),
        KeyBinding::new("secondary-a", SelectAll, Some("Editor")),
        KeyBinding::new("backspace", Backspace, Some("Editor")),
        KeyBinding::new("delete", Delete, Some("Editor")),
        KeyBinding::new("enter", Newline, Some("Editor")),
        KeyBinding::new("cmd-enter", InsertSourceBlankLine, Some("Editor")),
        KeyBinding::new("cmd-/", ToggleSourceComment, Some("Editor")),
        KeyBinding::new("secondary-c", Copy, Some("Editor")),
        KeyBinding::new("secondary-x", Cut, Some("Editor")),
        KeyBinding::new("secondary-v", Paste, Some("Editor")),
        KeyBinding::new("secondary-z", Undo, Some("Editor")),
        KeyBinding::new("secondary-shift-z", Redo, Some("Editor")),
        KeyBinding::new("cmd-u", UndoSelection, Some("Editor")),
        KeyBinding::new("cmd-shift-u", RedoSelection, Some("Editor")),
    ]);
    document_find::bind_keys(cx);
    goto_line::bind_keys(cx);
    source_completion::bind_keys(cx);
    source_diagnostics::bind_keys(cx);
    #[cfg(target_os = "macos")]
    source_lsp::bind_keys(cx);
    merge_review::bind_keys(cx);
}

const TYPING_PAUSE: Duration = Duration::from_secs(1);

pub struct Editor {
    focus: FocusHandle,
    view: EditorView<Option<Section>>,
    last_typed_at: Option<Instant>,
    find: Option<FindPanel>,
    goto_line: Option<GotoLinePanel>,
    completion: Option<SourceCompletion>,
    source_collab: Option<source_collab::SourceCollab>,
    diagnostics: SourceDiagnostics,
    #[cfg(target_os = "macos")]
    source_lsp: Option<source_lsp::SourceLsp>,
    #[cfg(target_os = "macos")]
    reference_panel_ui: source_lsp::ReferencePanelUi,
    source_tooltip: Option<source_tooltip::SourceTooltip>,
    #[cfg(test)]
    painted_source_tooltip: Option<source_tooltip::PaintedTooltip>,
    merge_review: Option<MergeReview>,
    caret_blink: draw_selection::CaretBlink,
    selection_epoch: std::time::Instant,
    tab_focus: TabFocusMode,
    // A soft break shares a byte offset between the end of one row and the start of the next.
    upstream: bool,
    marked: Option<Range<usize>>,
    active: Option<Section>,
    source_syntax: Option<SourceSyntax>,
    selecting: bool,
    rectangle: Option<source_rectangle::RectangleDrag>,
    layout: Option<Layout>,
    highlighting: highlight::Highlighting,
    input_enabled: bool,
    scroll: ScrollHandle,
    end_scroll_padding: EndScrollPadding,
    reveal_caret: bool,
    reveal_source: Option<SourceReveal>,
    preferred_x: Option<Pixels>,
    graphics: HashMap<(String, SectionKind, u32), Result<Graphic, String>>,
    images: HashMap<String, ImageEntry>,
    next_image_request: u64,
    image_requests: usize,
    allow_remote_images: bool,
    path: Option<PathBuf>,
    workspace: Option<Workspace>,
    hover: Option<Range<usize>>,
    #[cfg(test)]
    prepainted_overlay: Option<String>,
    message: Option<String>,
    message_timeout: Option<(String, Task<()>)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceReveal {
    offset: usize,
    alignment: RevealAlignment,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RevealAlignment {
    Nearest,
    Center,
    Start(Pixels),
}

#[derive(Clone, Copy)]
enum EditScope {
    Active,
    Document,
}

enum ImageEntry {
    Loading(u64),
    Ready(Result<Graphic, String>),
    BudgetExceeded,
}

impl Editor {
    pub fn new(source: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_native_extensions(source, Extension::default(), window, cx)
            .expect("editor extensions")
    }

    /// Create a Markdown editor with trusted native CodeMirror extensions.
    /// The editor retains its stock extensions and validates the initial
    /// configuration before it registers window callbacks. Source bytes are
    /// not normalized. Extensions do not bypass the Markdown edit rules.
    pub fn with_native_extensions(
        source: String,
        extensions: Extension,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, ConfigurationError> {
        let state = EditorState::with_extensions(
            source,
            Extension::group([
                lint::extension(),
                bracket_matching(Default::default()),
                code_folding(),
                snippet_state_extension(),
                rectangular_selection(Default::default()),
                extensions,
            ]),
        )?;
        let source_collab = source_collab::SourceCollab::install(&state)?;
        let focus = cx.focus_handle().tab_index(0).tab_stop(true);
        cx.on_focus(&focus, window, |editor, window, cx| {
            editor.report_native_focus(true);
            editor.sync_caret_blink(window, cx);
            cx.notify();
        })
        .detach();
        cx.on_blur(&focus, window, |editor, _, cx| {
            editor.caret_blink.cancel();
            editor.finish_pointer_selection();
            editor.completion = None;
            editor.dismiss_keyboard_diagnostic_hover();
            editor.dismiss_source_lsp_tooltips();
            editor.clear_source_tooltip();
            editor.finish_composition();
            editor.cancel_source_snippet(cx);
            editor.report_native_focus(false);
            cx.notify();
        })
        .detach();
        cx.observe_window_activation(window, |editor, window, cx| {
            if !window.is_window_active() {
                let diagnostic = editor.dismiss_keyboard_diagnostic_hover();
                if editor.clear_source_tooltip() || diagnostic {
                    cx.notify();
                }
            }
            editor.sync_caret_blink(window, cx);
        })
        .detach();
        cx.observe_keystrokes(Self::observe_editor_key).detach();
        let mut view = EditorView::new(state);
        let caret_blink = draw_selection::CaretBlink::install(&mut view, cx);
        #[cfg(target_os = "macos")]
        if let Some(plugin) = announcements::AnnouncementPlugin::new(window, cx) {
            view.add_plugin(plugin);
        }
        Ok(Self {
            focus,
            view,
            last_typed_at: None,
            find: None,
            goto_line: None,
            completion: None,
            source_collab,
            diagnostics: SourceDiagnostics::new(cx.focus_handle()),
            #[cfg(target_os = "macos")]
            source_lsp: None,
            #[cfg(target_os = "macos")]
            reference_panel_ui: source_lsp::ReferencePanelUi::new(cx),
            source_tooltip: None,
            #[cfg(test)]
            painted_source_tooltip: None,
            merge_review: None,
            caret_blink,
            selection_epoch: cx.background_executor().now(),
            tab_focus: TabFocusMode::default(),
            upstream: false,
            marked: None,
            active: None,
            source_syntax: None,
            selecting: false,
            rectangle: None,
            layout: None,
            highlighting: highlight::Highlighting::default(),
            input_enabled: true,
            scroll: ScrollHandle::new(),
            end_scroll_padding: EndScrollPadding::default(),
            reveal_caret: false,
            reveal_source: None,
            preferred_x: None,
            graphics: HashMap::new(),
            images: HashMap::new(),
            next_image_request: 0,
            image_requests: 0,
            allow_remote_images: false,
            path: None,
            workspace: None,
            hover: None,
            #[cfg(test)]
            prepainted_overlay: None,
            message: None,
            message_timeout: None,
        })
    }

    fn report_native_focus(&mut self, focused: bool) {
        let result = self
            .view
            .set_focus_checked(focused, self.active.clone(), |transaction| {
                if !transaction.changes().is_empty() {
                    Err(EditError::InvalidRange)
                } else if transaction.selection().is_some() {
                    Err(EditError::InvalidSelection)
                } else {
                    Ok(())
                }
            });
        if let Err(error) = result {
            log_exception(
                self.view.state(),
                &ViewException::new(format!("Rejected focus effects: {error:?}")),
            );
            // Native focus has already changed. Report it without the rejected effects.
            let _ = self
                .view
                .set_viewport(self.view.viewport().clone(), focused);
        }
    }

    pub fn source(&self) -> &str {
        self.view.state().text().as_str()
    }

    pub(crate) fn is_markdown(&self) -> bool {
        self.path.as_deref().is_none_or(is_markdown)
    }

    fn markdown(&self) -> Markdown<'_> {
        Markdown::new(self.source())
    }

    #[cfg(test)]
    pub(crate) fn vertical_scroll_offset(&self) -> Pixels {
        self.scroll.offset().y
    }

    pub(crate) fn headings(&self) -> Vec<Heading> {
        if self.is_markdown() {
            self.markdown().headings()
        } else {
            Vec::new()
        }
    }

    /// Scroll to a UTF-8 byte offset without moving the caret or taking focus.
    pub(crate) fn reveal_source(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.request_source_reveal(offset, false, cx);
    }

    fn request_source_reveal(&mut self, offset: usize, center: bool, cx: &mut Context<Self>) {
        if self.is_composing() || !self.source().is_char_boundary(offset) {
            return;
        }
        let alignment = if center {
            RevealAlignment::Center
        } else {
            RevealAlignment::Nearest
        };
        self.reveal_source = Some(SourceReveal { offset, alignment });
        cx.notify();
    }

    pub fn set_path(&mut self, path: Option<PathBuf>, workspace: Option<Workspace>) {
        if self.path != path {
            self.highlighting.invalidate();
        }
        self.path = path;
        self.workspace = workspace;
        self.images.clear();
        self.allow_remote_images = false;
    }

    pub(crate) fn allow_remote_images(&mut self, cx: &mut Context<Self>) {
        self.allow_remote_images = true;
        cx.notify();
    }

    pub(crate) fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub(crate) fn set_input_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        debug_assert!(enabled || !self.is_composing());
        self.input_enabled = enabled;
        if let Some(find) = &self.find {
            find.set_editable(enabled, cx);
        }
        if let Some(goto_line) = &self.goto_line {
            goto_line.set_editable(enabled, cx);
        }
        if !enabled {
            self.finish_typing();
            self.selecting = false;
            self.completion = None;
            self.dismiss_keyboard_diagnostic_hover();
            self.dismiss_source_lsp_tooltips();
            self.clear_source_tooltip();
        }
        cx.notify();
    }

    fn anchor(&self) -> usize {
        self.view.state().selection().main().anchor()
    }

    fn cursor(&self) -> usize {
        self.view.state().selection().main().head()
    }

    fn selection(&self) -> Range<usize> {
        self.view.state().selection().main().range()
    }

    fn set_selection(&mut self, anchor: usize, head: usize) {
        self.finish_typing();
        self.completion = None;
        self.view
            .set_selection(EditorSelection::single(anchor, head))
            .expect("selection must use source byte boundaries");
    }

    fn set_user_selection(
        &mut self,
        anchor: usize,
        head: usize,
        event: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.finish_typing();
        if self.is_composing() {
            self.set_selection(anchor, head);
            self.finish_composition();
            return;
        }
        self.completion = None;
        let selection = EditorSelection::single(anchor, head);
        if self.view.state().selection() == &selection {
            return;
        }
        self.record_selection_transaction(
            Transaction::default()
                .with_selection(selection)
                .with_user_event(event),
            self.active.clone(),
            cx,
        );
    }

    fn record_selection_transaction(
        &mut self,
        transaction: Transaction,
        active: Option<Section>,
        cx: &mut Context<Self>,
    ) {
        self.finish_typing();
        self.completion = None;
        let elapsed = cx
            .background_executor()
            .now()
            .saturating_duration_since(self.selection_epoch);
        let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        self.active = self
            .view
            .update(transaction.with_time(millis), self.active.clone(), |_| {
                (None, active)
            })
            .expect("selection must use source byte boundaries");
    }

    fn finish_active(&mut self) {
        self.clear_source_tooltip();
        self.rectangle = None;
        self.source_syntax = None;
        if let Some(active) = self.active.take() {
            let snap_outside = |position| {
                if active.source.start < position && position < active.source.end {
                    active.source.end
                } else {
                    position
                }
            };
            self.set_selection(snap_outside(self.anchor()), snap_outside(self.cursor()));
        }
    }

    fn finish_typing(&mut self) {
        if self.last_typed_at.take().is_some() {
            self.view.end_group();
        }
    }

    fn finish_composition(&mut self) {
        self.last_typed_at = None;
        self.marked = None;
        self.view.end_group();
        self.finish_source_collab_composition();
    }

    fn invalidate_edit_layout(&mut self) {
        self.clear_source_tooltip();
        self.rectangle = None;
        self.completion = None;
        self.highlighting.invalidate();
        self.layout = None;
        if self
            .active
            .as_ref()
            .is_none_or(|section| section.kind != SectionKind::DraftCode)
        {
            self.source_syntax = None;
        }
        self.upstream = false;
        self.reveal_caret = true;
        self.reveal_source = None;
        self.preferred_x = None;
    }

    fn replay_history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if !self.input_enabled {
            return;
        }
        self.finish_typing();
        let revision = self.view.revision();
        let active = if redo {
            self.view.redo()
        } else {
            self.view.undo()
        };
        if let Some(active) = active {
            self.finish_history_replay(active, revision, cx);
        }
    }

    fn replay_selection_history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if !self.input_enabled || self.is_composing() {
            return;
        }
        self.finish_typing();
        let revision = self.view.revision();
        let active = self.active.clone();
        let restored = if redo {
            self.view.redo_selection(active)
        } else {
            self.view.undo_selection(active)
        };
        if let Some(active) = restored {
            self.finish_history_replay(active, revision, cx);
            self.leave_source_if_outside(self.cursor());
        }
    }

    fn finish_history_replay(
        &mut self,
        active: Option<Section>,
        previous_revision: u64,
        cx: &mut Context<Self>,
    ) {
        self.active = if self.view.revision() == previous_revision {
            active
        } else {
            active.map(|section| self.restore_source_collab_frame(section))
        };
        self.finish_composition();
        self.invalidate_edit_layout();
        cx.notify();
    }

    fn type_text(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) -> bool {
        if self.is_composing()
            || self.has_multiple_selections()
            || !range.is_empty()
            || range != self.selection()
            || text.is_empty()
            || text.contains(['\r', '\n'])
        {
            return self.replace(range, text, cx);
        }
        let now = cx.background_executor().now();
        if self
            .last_typed_at
            .is_none_or(|previous| now.duration_since(previous) >= TYPING_PAUSE)
        {
            self.view.start_group();
        }
        let applied = self
            .replace_with(
                EditScope::Active,
                range,
                text,
                std::convert::identity,
                |_, _, _| {},
                cx,
            )
            .is_some();
        if applied {
            self.last_typed_at = Some(now);
        } else {
            self.finish_composition();
        }
        applied
    }

    fn propose_native_input(
        &self,
        range: &Range<usize>,
        text: &str,
    ) -> Result<InputHandling, EditError> {
        let state = self.view.state();
        let change = InputChange::new(state, range.clone(), text)?;
        run_input_handlers(change, || {
            let transaction = if *range == self.selection() && self.has_multiple_selections() {
                state.replace_selection(text)?
            } else {
                let changes = ChangeSet::single(Change::new(range.clone(), text))?;
                let at = range
                    .start
                    .checked_add(text.len())
                    .ok_or(EditError::InvalidRange)?;
                Transaction::new(changes).with_selection(EditorSelection::single(at, at))
            };
            Ok(transaction
                .with_scroll_into_view()
                .with_user_event("input.type"))
        })
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) -> bool {
        if let Some(accepted) = self.replace_multiple_selections(&range, text, false, cx) {
            return accepted;
        }
        self.replace_and_finish(EditScope::Active, range, text, |_, _, _| {}, cx)
    }

    fn replace_with_effect(
        &mut self,
        range: Range<usize>,
        text: &str,
        effect: StateEffect,
        cx: &mut Context<Self>,
    ) -> Option<ChangeDesc> {
        self.finish_typing();
        let old_len = self.source().len();
        let mut desc = None;
        self.replace_with(
            EditScope::Active,
            range,
            text,
            |transaction| {
                desc = Some(
                    transaction
                        .changes()
                        .desc(old_len)
                        .expect("valid Markdown edit"),
                );
                transaction.with_effects([effect])
            },
            |_, _, _| {},
            cx,
        )?;
        self.finish_composition();
        desc
    }

    fn replace_and_finish(
        &mut self,
        scope: EditScope,
        range: Range<usize>,
        text: &str,
        finish: impl FnOnce(&str, &mut SelectionRange, &mut Option<Section>),
        cx: &mut Context<Self>,
    ) -> bool {
        self.finish_typing();
        if self
            .replace_with(scope, range, text, std::convert::identity, finish, cx)
            .is_none()
        {
            return false;
        }
        self.finish_composition();
        true
    }

    fn replace_with(
        &mut self,
        scope: EditScope,
        range: Range<usize>,
        text: &str,
        decorate: impl FnOnce(Transaction) -> Transaction,
        finish: impl FnOnce(&str, &mut SelectionRange, &mut Option<Section>),
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if !self.input_enabled {
            return None;
        }
        let active = match scope {
            EditScope::Active => self.active.as_ref(),
            EditScope::Document => None,
        };
        let markdown = self.is_markdown();
        let planned = if markdown {
            self.markdown().plan_replacement(range, text, active).ok()
        } else {
            self.source()
                .get(range.clone())
                .map(|_| (range, Cow::Borrowed(text)))
        };
        let Some((range, inserted)) = planned else {
            self.message = Some(
                if markdown {
                    "Cannot edit this Markdown range"
                } else {
                    "Cannot edit this text range"
                }
                .into(),
            );
            cx.notify();
            return None;
        };
        let start = range.start;
        let caret = start + text.len();
        let mut active = active
            .filter(|section| {
                range.start >= section.content.start && range.end <= section.content.end
            })
            .cloned();
        if let Some(section) = &mut active {
            section.content.end = section.content.end - range.len() + text.len();
            section.source.end = section.source.end - range.len() + inserted.len();
            if section.kind == SectionKind::DraftCode && section.source.is_empty() {
                active = None;
            }
        }
        let changes = ChangeSet::single(Change::new(range, inserted))
            .expect("Markdown plans a valid replacement");
        let transaction = decorate(
            Transaction::new(changes).with_selection(EditorSelection::single(caret, caret)),
        );
        let check = preserve_markdown_edit(&transaction, None);
        let applied = self
            .view
            .update_checked(transaction, self.active.clone(), check, |state| {
                let initial = state.selection().main();
                let mut selection = initial;
                finish(state.text().as_str(), &mut selection, &mut active);
                let selection = (selection != initial)
                    .then(|| EditorSelection::single(selection.anchor(), selection.head()));
                (selection, active)
            });
        let Ok(active) = applied else {
            self.message = Some("Cannot apply this Markdown edit".into());
            cx.notify();
            return None;
        };
        self.active = active;
        self.invalidate_edit_layout();
        self.message = None;
        cx.notify();
        Some(start)
    }

    fn apply_source_transaction(
        &mut self,
        transaction: Transaction,
        scope: Range<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_source_transaction_with_receipt(transaction, scope, None, cx)
    }

    fn apply_source_transaction_with_receipt(
        &mut self,
        mut transaction: Transaction,
        scope: Range<usize>,
        receipt: Option<&mut Option<EditReceipt>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(before) = &self.active else {
            return false;
        };
        let mut after = before.clone();
        let mut removed = 0;
        let mut inserted = 0;
        let mut corrections = Vec::new();
        let markdown = self.markdown();
        for (range, text) in transaction.changes().changes() {
            if range.start < scope.start || range.end > scope.end {
                return false;
            }
            let Ok((planned_range, planned)) =
                markdown.plan_replacement(range.clone(), text, Some(before))
            else {
                return false;
            };
            if *range != planned_range {
                return false;
            }
            let Some(suffix) = planned.strip_prefix(text) else {
                return false;
            };
            if !suffix.is_empty() {
                // The separator belongs to the fence, not to the editable body.
                let at = range.start - removed + inserted + text.len();
                corrections.push(Change::new(at..at, suffix));
            }
            removed += range.len();
            inserted += text.len();
            after.source.end = after.source.end - range.len() + planned.len();
        }
        after.content.end = after.content.end - removed + inserted;
        if !corrections.is_empty() {
            let Ok(changes) = ChangeSet::new(corrections) else {
                return false;
            };
            let Ok(corrected) = transaction.merge(
                Transaction::new(changes),
                self.source().len(),
                TransactionCoordinates::Sequential,
            ) else {
                return false;
            };
            transaction = corrected;
        }
        self.finish_typing();
        let check = preserve_markdown_edit(&transaction, receipt);
        let Ok(active) = self
            .view
            .update_checked(transaction, self.active.clone(), check, |_| {
                (None, Some(after))
            })
        else {
            return false;
        };
        self.active = active;
        self.finish_composition();
        self.invalidate_edit_layout();
        self.message = None;
        cx.notify();
        true
    }

    fn move_horizontal(&mut self, right: bool, extend: bool, cx: &mut Context<Self>) {
        if self.move_in_source_columns(right, extend, cx)
            || self.source_fence().is_some()
                && self.has_multiple_selections()
                && !self.is_composing()
        {
            return;
        }
        let selection = self.selection();
        let position = if !extend && !selection.is_empty() {
            if right {
                selection.end
            } else {
                selection.start
            }
        } else if let Some(layout) = &self.layout {
            layout.step(self.cursor(), right)
        } else {
            self.cursor()
        };
        self.move_to(position, extend, cx);
        self.upstream = !right;
        self.reveal_caret = true;
    }

    fn move_vertical(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        if !extend && self.move_source_completion(down, false, cx) {
            return;
        }
        if self.move_in_source_rows(down, extend, cx)
            || self.source_fence().is_some()
                && self.has_multiple_selections()
                && !self.is_composing()
        {
            return;
        }
        if let Some(((position, upstream), x)) = self.layout.as_ref().and_then(|layout| {
            layout.vertical(self.cursor(), self.upstream, self.preferred_x, down)
        }) {
            self.move_to(position, extend, cx);
            self.upstream = upstream && self.cursor() == position;
            self.preferred_x = Some(x);
            self.reveal_caret = true;
        }
    }

    fn move_to(&mut self, position: usize, extend: bool, cx: &mut Context<Self>) {
        self.move_caret(position, extend, "select", cx);
    }

    fn move_caret(
        &mut self,
        position: usize,
        extend: bool,
        event: &'static str,
        cx: &mut Context<Self>,
    ) {
        if !self.source().is_char_boundary(position) {
            return;
        }
        self.preferred_x = None;
        self.set_user_selection(
            if extend { self.anchor() } else { position },
            position,
            event,
            cx,
        );
        self.upstream = false;
        self.leave_source_if_outside(position);
        cx.notify();
    }

    fn move_to_bidi(
        &mut self,
        target: SelectionRange,
        extend: bool,
        event: &'static str,
        cx: &mut Context<Self>,
    ) {
        let head = target.head();
        let range = EditorSelection::range(
            if extend { self.anchor() } else { head },
            head,
            None,
            target.bidi_level(),
            target.assoc(),
        );
        let old = self.view.state().selection().main();
        if self.has_multiple_selections()
            || old != range
            || old.assoc() != range.assoc()
            || old.bidi_level() != range.bidi_level()
        {
            let selection = EditorSelection::new(vec![range], 0).expect("one range");
            self.record_selection_transaction(
                Transaction::default()
                    .with_selection(selection)
                    .with_user_event(event),
                self.active.clone(),
                cx,
            );
        }
        self.upstream =
            target.assoc() == Some(gpui_editor_codemirror::Assoc::Before) && self.cursor() == head;
        self.preferred_x = None;
        cx.notify();
    }

    fn leave_source_if_outside(&mut self, position: usize) {
        if self.active.as_ref().is_some_and(|section| {
            position < section.content.start || position > section.content.end
        }) {
            self.finish_active();
        }
    }

    fn remove(&mut self, right: bool, cx: &mut Context<Self>) {
        if self.view.state().read_only() {
            return;
        }
        if self.unfold_at_deletion_edge(right, cx)
            || (!right && (self.delete_source_bracket_pair(cx) || self.edit_markup(true, cx)))
        {
            return;
        }
        if self.delete_multiple_selections(right, cx).is_some() {
            return;
        }
        let mut selection = self.selection();
        if !selection.is_empty() && !self.is_composing() {
            let notice =
                gpui_editor_codemirror::commands::announce_selection_deleted(self.view.state());
            self.replace_with_effect(selection, "", notice, cx);
            return;
        }
        if selection.is_empty() {
            if let Some(layout) = &self.layout {
                let mut at = self.cursor();
                if self.active.is_none() && self.is_markdown() {
                    let inline: Vec<_> = self
                        .markdown()
                        .sections()
                        .into_iter()
                        .filter(|section| match section.kind {
                            SectionKind::Strong | SectionKind::Emphasis => true,
                            SectionKind::InlineCode => {
                                self.cursor() <= section.source.start
                                    || self.cursor() >= section.source.end
                            }
                            _ => false,
                        })
                        .collect();
                    // Skip hidden delimiters in the rendered view, not in source edit mode.
                    while let Some(next) = inline.iter().find_map(|section| {
                        if right && at == section.source.start {
                            Some(section.content.start)
                        } else if right && at == section.content.end {
                            Some(section.source.end)
                        } else if !right && at == section.source.end {
                            Some(section.content.end)
                        } else if !right && at == section.content.start {
                            Some(section.source.start)
                        } else {
                            None
                        }
                    }) {
                        at = next;
                    }
                }
                let other = layout.step(at, right);
                selection = at.min(other)..at.max(other);
            }
            if selection.is_empty() {
                return;
            }
        }
        if !right
            && !self.is_composing()
            && self.selection().is_empty()
            && selection.end == self.cursor()
        {
            self.edit_with_source_completion(
                "delete.backward",
                |editor, cx| editor.replace(selection, "", cx),
                cx,
            );
        } else {
            self.replace(selection, "", cx);
        }
    }

    fn finish_pointer_selection(&mut self) {
        self.selecting = false;
        self.rectangle = None;
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_pointer_selection();
        if !self.input_enabled {
            return;
        }
        if self.mouse_down_fold_gutter(event.position, cx) {
            window.prevent_default();
            return;
        }
        if self.start_source_rectangle(event, window, cx) {
            return;
        }
        let Some(layout) = &self.layout else { return };
        let (position, upstream) = layout.position_at(event.position);
        let bidi_hit = layout
            .bidi_hit_at(event.position)
            .filter(|_| !self.is_composing());
        let item = layout.item_at(event.position);
        let in_frame = layout.source_frame_contains(event.position);
        let hit = item.and_then(|item| item.target.clone());
        let block = item
            .filter(|item| matches!(item.paint, ItemPaint::Block(_)))
            .map(|item| item.source.clone());
        if !event.modifiers.shift
            && let Some(range) = item
                .filter(|item| matches!(item.paint, ItemPaint::Fold(_)))
                .map(|item| item.source.clone())
            && self.open_source_fold(range, cx)
        {
            self.selecting = false;
            window.focus(&self.focus);
            return;
        }
        let in_source = self.active.as_ref().is_some_and(|active| {
            active.content.start <= position
                && position <= active.content.end
                && in_frame
                && hit.is_none()
                && block.is_none()
        });
        let position = if in_source {
            position
        } else if let Some(active) = &self.active
            && active.content.contains(&position)
        {
            active.source.end
        } else {
            position
        };
        if !in_source {
            self.finish_active();
        }
        self.selecting = true;
        window.focus(&self.focus);
        if let Some(block) = block.filter(|_| !event.modifiers.shift) {
            self.set_user_selection(block.start, block.end, "select.pointer", cx);
            self.preferred_x = None;
            self.upstream = false;
        } else if let Some(section) = hit.filter(|_| !event.modifiers.shift) {
            self.selecting = false;
            if section.kind == SectionKind::Link {
                self.move_caret(section.source.start, false, "select.pointer", cx);
                match self.link_target(&section) {
                    Ok(crate::media::Target::Web(url)) => cx.open_url(url.as_str()),
                    Ok(crate::media::Target::Local(path)) => cx.emit(OpenLocalDocument(path)),
                    Err(error) => self.message = Some(error),
                }
                cx.notify();
            } else {
                self.activate(section, false, cx);
            }
        } else if let Some(target) = bidi_hit.filter(|_| in_source) {
            self.move_to_bidi(target, event.modifiers.shift, "select.pointer", cx);
        } else {
            self.move_caret(position, event.modifiers.shift, "select.pointer", cx);
            self.upstream = upstream && self.cursor() == position;
        }
    }

    fn link_target(&self, section: &Section) -> Result<crate::media::Target, String> {
        crate::media::link_target(
            section.destination.as_deref().unwrap_or_default(),
            self.path.as_deref(),
            self.workspace.as_ref().map(Workspace::root),
        )
    }

    fn hovered_link(layout: &Layout, position: Point<Pixels>) -> Option<&layout::Item> {
        layout.item_at(position).filter(|item| {
            item.target
                .as_ref()
                .is_some_and(|section| section.kind == SectionKind::Link)
        })
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag_source_rectangle(event.position, cx) {
            return;
        }
        let Some(layout) = &self.layout else { return };
        if self.selecting {
            if let Some(target) = layout
                .bidi_hit_at(event.position)
                .filter(|_| !self.is_composing())
            {
                self.move_to_bidi(target, true, "select.pointer", cx);
                return;
            }
            let (position, upstream) = layout.position_at(event.position);
            self.move_caret(position, true, "select.pointer", cx);
            self.upstream = upstream && self.cursor() == position;
        } else if self.hover.as_ref()
            != Self::hovered_link(layout, event.position).map(|item| &item.source)
            || lint::diagnostic_count(self.view.state()) != 0
        {
            cx.notify();
        }
    }

    fn expire_message(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.message.as_ref() else {
            self.message_timeout = None;
            return;
        };
        if self
            .message_timeout
            .as_ref()
            .is_some_and(|(shown, _)| shown == message)
        {
            return;
        }
        let timer = cx.background_executor().timer(Duration::from_secs(5));
        let task = cx.spawn(async move |editor, cx| {
            timer.await;
            let _ = editor.update(cx, |editor, cx| {
                editor.message = None;
                cx.notify();
            });
        });
        self.message_timeout = Some((message.clone(), task));
    }
}

pub struct OpenLocalDocument(pub PathBuf);
impl EventEmitter<OpenLocalDocument> for Editor {}

pub(crate) struct PreviewDiagnostic(pub &'static str);

impl EventEmitter<PreviewDiagnostic> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset || units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end)
}

struct EditReceipt {
    proposed: Transaction,
    resolved: Transaction,
}

fn preserve_markdown_edit<'a>(
    transaction: &Transaction,
    receipt: Option<&'a mut Option<EditReceipt>>,
) -> impl FnOnce(&Transaction) -> Result<(), EditError> + use<'a> {
    let changes = transaction.changes().clone();
    let selection = transaction.selection().cloned();
    let proposed = receipt.as_ref().map(|_| transaction.clone());
    move |resolved| {
        if &changes != resolved.changes() {
            return Err(EditError::InvalidRange);
        }
        if selection.as_ref() != resolved.selection() {
            return Err(EditError::InvalidSelection);
        }
        if let (Some(receipt), Some(proposed)) = (receipt, proposed) {
            *receipt = Some(EditReceipt {
                proposed,
                resolved: resolved.clone(),
            });
        }
        Ok(())
    }
}

fn to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    text[..range.start].encode_utf16().count()..text[..range.end].encode_utf16().count()
}

#[cfg(not(target_os = "macos"))]
impl Editor {
    fn type_with_source_lsp_signature(
        &mut self,
        _: &str,
        _: &Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Self, &mut Context<Self>) -> bool,
    ) {
        self.edit_with_source_completion("input.type", edit, cx);
    }

    fn dismiss_source_lsp_tooltips(&mut self) -> bool {
        false
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.source();
        let bytes = from_utf16(text, range);
        *actual = Some(to_utf16(text, bytes.clone()));
        text.get(bytes).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !self.input_enabled && !ignore_disabled_input {
            return None;
        }
        Some(UTF16Selection {
            range: to_utf16(self.source(), self.selection()),
            reversed: self.cursor() < self.anchor(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .clone()
            .map(|range| to_utf16(self.source(), range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled || self.view.state().read_only() {
            return;
        }
        let range = range
            .map(|range| from_utf16(self.source(), range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let may_complete =
            self.marked.is_none() && range == self.selection() && text.chars().count() == 1;
        if !self.is_composing() {
            match self.propose_native_input(&range, text) {
                Ok(InputHandling::Pass) => {}
                Ok(InputHandling::Handled(Some(transaction))) => {
                    let edit = |editor: &mut Self, cx: &mut Context<Self>| {
                        editor.apply_multi_edit(transaction, cx)
                    };
                    if may_complete {
                        self.type_with_source_lsp_signature(text, window, cx, edit);
                    } else {
                        edit(self, cx);
                    }
                    return;
                }
                Ok(InputHandling::Handled(None)) => return,
                Err(_) => {
                    self.message = Some("Cannot apply this input".into());
                    cx.notify();
                    return;
                }
            }
        }
        if text == "`"
            && range.is_empty()
            && range.start == self.cursor()
            && self.marked.is_none()
            && self.complete_code_fence(range.start, cx)
        {
            return;
        }
        let edit = |editor: &mut Self, cx: &mut Context<Self>| {
            editor.type_source_tag(&range, text, cx)
                || editor.type_source_percent_brace(&range, text, cx)
                || editor.type_source_bracket(&range, text, cx)
                || editor.type_source_indent(&range, text, cx)
                || editor.type_text(range, text, cx)
        };
        if may_complete {
            self.type_with_source_lsp_signature(text, window, cx, edit);
        } else {
            edit(self, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_enabled || self.view.state().read_only() {
            return;
        }
        let range = range
            .map(|range| from_utf16(self.source(), range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let starting = self.marked.is_none();
        if starting {
            self.finish_typing();
            let lsp = self.dismiss_source_lsp_tooltips();
            if self.clear_source_tooltip() || lsp {
                cx.notify();
            }
            self.cancel_source_snippet(cx);
            self.collapse_multiple_selections(cx);
            self.view.start_group();
            self.start_source_collab_composition();
        }
        let start = range.start;
        let applied = self.replace_with(
            EditScope::Active,
            range,
            text,
            std::convert::identity,
            |_, selection, _| {
                if !text.is_empty()
                    && let Some(selected) = selected
                {
                    let selected = from_utf16(text, selected);
                    *selection = SelectionRange::new(start + selected.start, start + selected.end);
                }
            },
            cx,
        );
        match applied {
            Some(start) if !text.is_empty() => {
                self.dismiss_keyboard_diagnostic_hover();
                self.marked = Some(start..start + text.len());
            }
            Some(_) => self.finish_composition(),
            None if starting => self.finish_composition(),
            None => {}
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let offset = from_utf16(self.source(), range).start;
        let layout = self.layout.as_ref()?;
        if offset == self.cursor() {
            layout.bounds_at_selection(self.view.state().selection().main(), self.upstream)
        } else {
            layout.bounds_at(offset, false)
        }
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let byte = self.layout.as_ref()?.index_at(position);
        Some(self.source().get(..byte)?.encode_utf16().count())
    }
}

const HORIZONTAL_PADDING: f32 = 28.;
const VERTICAL_PADDING: f32 = 24.;

fn wrap_width(viewport_width: Pixels) -> Pixels {
    (viewport_width - px(2. * HORIZONTAL_PADDING + 2.)).max(px(1.))
}

impl Editor {
    fn panel_input_colors(
        cx: &Context<Self>,
    ) -> impl Fn(&App) -> Option<FieldColors> + Clone + use<> {
        let editor = cx.weak_entity();
        // GPUI renders the field after the parent render returns. Read the current
        // extension then, without a field update or a notification to its observers.
        move |cx| {
            let style = view_style(editor.upgrade()?.read(cx).view.state())?;
            Some(FieldColors {
                surface: rgba(style.panels.background.rgba32()),
                text: rgba(style.panels.foreground.rgba32()),
                // The dark base .cm-textfield border in @codemirror/view 6.44.0.
                border: rgb(0x555555),
                accent: rgba(style.caret.rgba32()),
                selection: rgba(style.selection.rgba32()),
            })
        }
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_caret_blink(window, cx);
        if self.is_reviewing_changes() {
            #[cfg(test)]
            {
                self.painted_source_tooltip = None;
            }
            return self.render_merge_review(window, cx).into_any_element();
        }
        #[cfg(target_os = "macos")]
        self.retain_source_lsp_diagnostics();
        self.sync_source_diagnostics(cx);
        let editor = cx.entity();
        // The Shell can change the body width without resizing the window. Use
        // the space GPUI assigned us before laying out the scrollable document.
        let element = div()
            .flex()
            .flex_col()
            .size_full()
            .children(self.render_find(window, cx))
            .children(self.render_goto_line(window, cx))
            .children(self.render_source_diagnostics(window, cx))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let mut body =
                            editor.update(cx, |editor, cx| editor.body(bounds.size, window, cx));
                        body.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                        body
                    },
                    |_, mut body, window, cx| body.paint(window, cx),
                )
                .flex_1()
                .min_h_0()
                .w_full(),
            );
        #[cfg(target_os = "macos")]
        let element = element.children(self.render_source_lsp_references(window, cx));
        element.into_any_element()
    }
}

impl Editor {
    fn body(
        &mut self,
        viewport: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tooltip = view_style(self.view.state()).map(|style| style.tooltip);
        self.expire_message(cx);
        let previous = self.scroll.bounds();
        if viewport.width != previous.size.width
            && self
                .layout
                .as_ref()
                .and_then(|layout| {
                    layout.bounds_at_selection(self.view.state().selection().main(), self.upstream)
                })
                .is_some_and(|caret| previous.intersects(&caret))
        {
            self.reveal_caret = true;
        }
        let wrap_width = wrap_width(viewport.width);
        let (layout, isolation_cache) = self.shape_for_paint(window, cx, wrap_width);
        #[cfg(not(target_os = "macos"))]
        let _ = isolation_cache;
        let drop_cursor_revision = self.view.revision();
        let theme = layout.theme;
        let dark = theme.view.is_some_and(|style| style.dark);
        let frame = layout.source_frame.as_ref().map(|frame| frame.bounds);
        let width = layout
            .rows
            .iter()
            .map(|row| row.width)
            .max()
            .unwrap_or(px(0.))
            .max(frame.map_or(wrap_width, |bounds| bounds.right()))
            + px(2.);
        let top_padding = px(VERTICAL_PADDING);
        let end_padding = self
            .end_scroll_padding
            .update(
                self.view.state(),
                viewport.height.into(),
                DEFAULT_LINE_HEIGHT.into(),
                top_padding.into(),
            )
            .map(Pixels::from);
        let height = layout
            .rows
            .last()
            .map_or(DEFAULT_LINE_HEIGHT, |row| row.y + row.height)
            .max(frame.map_or(px(0.), |bounds| bounds.bottom()))
            .max(viewport.height - (top_padding + end_padding.unwrap_or(top_padding)));
        let editor = cx.entity();
        let overlay_editor = editor.clone();
        let element = div()
            .id("editor")
            .size_full()
            .bg(theme.surface)
            .text_color(theme.text)
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .px(px(HORIZONTAL_PADDING))
            .pt(top_padding)
            .pb(if end_padding.is_some() {
                px(0.)
            } else {
                top_padding
            })
            .cursor(
                if self.input_enabled && window.modifiers().alt && self.source_fence().is_some() {
                    CursorStyle::Crosshair
                } else {
                    CursorStyle::IBeam
                },
            )
            .key_context("Editor")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|editor, event, window, cx| {
                if !editor.handle_editor_tab(event, window, cx) {
                    if editor.focus.is_focused(window) {
                        editor.commit_source_completion(event, cx);
                    }
                    editor.source_key_down(event, cx);
                }
            }))
            .on_action(cx.listener(|editor, _: &SelectSourceParent, _, cx| {
                if !editor.select_source_parent(cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &IndentSourceSelection, _, cx| {
                if !editor.indent_source(SourceIndent::Selection, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &InsertSourceBlankLine, _, cx| {
                if !editor.indent_source(SourceIndent::BlankLine, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &ToggleSourceComment, _, cx| {
                if !editor.toggle_source_comment(cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &ToggleSourceFold, _, cx| {
                if !editor.toggle_source_fold(cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &OpenDiagnostics, window, cx| {
                if !editor.open_source_diagnostics(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &NextDiagnostic, window, cx| {
                if !editor.jump_source_diagnostic(false, window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &PreviousDiagnostic, window, cx| {
                if !editor.jump_source_diagnostic(true, window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &OpenCompletion, _, cx| {
                if !editor.request_source_completion(true, cx) {
                    cx.propagate();
                }
            }));
        #[cfg(target_os = "macos")]
        let element = element
            .on_action(
                cx.listener(|editor, _: &source_lsp::ShowSignatureHelp, window, cx| {
                    if window.is_window_active()
                        && editor.focus.is_focused(window)
                        && editor.request_source_lsp_signature_help()
                    {
                        cx.notify();
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_action(
                cx.listener(|editor, _: &source_lsp::PreviousSignature, window, cx| {
                    if editor.cycle_source_lsp_signature(false, window) {
                        cx.notify();
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_action(
                cx.listener(|editor, _: &source_lsp::NextSignature, window, cx| {
                    if editor.cycle_source_lsp_signature(true, window) {
                        cx.notify();
                    } else {
                        cx.propagate();
                    }
                }),
            );
        element
            .on_action(cx.listener(|editor, _: &CloseCompletion, _, cx| {
                let dismissed = if editor.is_composing() {
                    false
                } else {
                    let lint = editor.dismiss_keyboard_diagnostic_hover();
                    let lsp = editor.dismiss_source_lsp_tooltips();
                    editor.clear_source_tooltip() || lsp || lint
                };
                if dismissed {
                    cx.notify();
                }
                if editor.is_composing()
                    || (!editor.close_source_completion(cx)
                        && !editor.cancel_source_snippet(cx)
                        && !editor.collapse_multiple_selections(cx)
                        && !dismissed)
                {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &ToggleEditorTabFocus, _, cx| {
                if !editor.toggle_editor_tab_focus(cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &CompletionPageUp, _, cx| {
                if !editor.move_source_completion(false, true, cx)
                    && !editor.page_in_source_rows(false, false, cx)
                {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &CompletionPageDown, _, cx| {
                if !editor.move_source_completion(true, true, cx)
                    && !editor.page_in_source_rows(true, false, cx)
                {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &SelectSourcePageUp, _, cx| {
                if !editor.page_in_source_rows(false, true, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &SelectSourcePageDown, _, cx| {
                if !editor.page_in_source_rows(true, true, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &OpenFind, window, cx| editor.open_find(window, cx)))
            .on_action(
                cx.listener(|editor, _: &OpenGotoLine, window, cx| {
                    editor.open_goto_line(window, cx)
                }),
            )
            .on_action(
                cx.listener(|editor, _: &FindNext, window, cx| {
                    editor.repeat_find(false, window, cx)
                }),
            )
            .on_action(cx.listener(|editor, _: &SelectNextOccurrence, _, cx| {
                if !editor.select_search_occurrences(false, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &SelectSelectionMatches, _, cx| {
                if !editor.select_search_occurrences(true, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|editor, _: &FindPrevious, window, cx| {
                editor.repeat_find(true, window, cx)
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_modifiers_changed(cx.listener(|editor, _, _, cx| {
                if editor.source_fence().is_some() {
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|editor, hovered, _, cx| {
                if !hovered
                    && (editor.hover.take().is_some()
                        || lint::diagnostic_count(editor.view.state()) != 0)
                {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.finish_pointer_selection()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.finish_pointer_selection()),
            )
            .on_action(cx.listener(|e, _: &Left, _, cx| e.move_horizontal(false, false, cx)))
            .on_action(cx.listener(|e, _: &Right, _, cx| e.move_horizontal(true, false, cx)))
            .on_action(cx.listener(|e, _: &SelectLeft, _, cx| e.move_horizontal(false, true, cx)))
            .on_action(cx.listener(|e, _: &SelectRight, _, cx| e.move_horizontal(true, true, cx)))
            .on_action(cx.listener(|e, _: &Up, _, cx| e.move_vertical(false, false, cx)))
            .on_action(cx.listener(|e, _: &Down, _, cx| e.move_vertical(true, false, cx)))
            .on_action(cx.listener(|e, _: &AddSourceCursorAbove, _, cx| {
                if !e.add_source_cursor(false, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|e, _: &AddSourceCursorBelow, _, cx| {
                if !e.add_source_cursor(true, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|e, _: &SelectUp, _, cx| e.move_vertical(false, true, cx)))
            .on_action(cx.listener(|e, _: &SelectDown, _, cx| e.move_vertical(true, true, cx)))
            .on_action(cx.listener(|e, _: &SelectAll, _, cx| {
                let range = e
                    .active
                    .as_ref()
                    .map_or(0..e.source().len(), |section| section.content.clone());
                e.set_user_selection(range.start, range.end, "select", cx);
                e.upstream = false;
                e.reveal_caret = true;
                e.preferred_x = None;
                cx.notify();
            }))
            .on_action(cx.listener(|e, _: &Backspace, _, cx| e.remove(false, cx)))
            .on_action(cx.listener(|e, _: &Delete, _, cx| e.remove(true, cx)))
            .on_action(cx.listener(|e, _: &Newline, window, cx| {
                if !e.accept_source_completion(None, cx)
                    && !e.indent_source(SourceIndent::Newline, cx)
                    && !e.edit_markup(false, cx)
                {
                    e.replace_text_in_range(None, "\n", window, cx);
                }
            }))
            .on_action(cx.listener(|e, _: &Copy, _, cx| e.copy_or_cut(false, cx)))
            .on_action(cx.listener(|e, _: &Cut, _, cx| e.copy_or_cut(true, cx)))
            .on_action(cx.listener(|e, _: &Paste, _, cx| {
                if e.input_enabled
                    && !e.view.state().read_only()
                    && let Some(raw) = cx.read_from_clipboard().and_then(|item| item.text())
                    && let Ok(text) = filter_clipboard_input(e.view.state(), raw)
                {
                    let range = e.marked.clone().unwrap_or_else(|| e.selection());
                    if !e.paste_source_lines(&text, cx)
                        && !e.paste_source_link(&text, cx)
                        && e.replace_multiple_selections(&range, &text, true, cx)
                            .is_none()
                    {
                        e.replace(range, &text, cx);
                    }
                }
            }))
            .on_action(cx.listener(|e, _: &Undo, _, cx| e.replay_history(false, cx)))
            .on_action(cx.listener(|e, _: &Redo, _, cx| e.replay_history(true, cx)))
            .on_action(
                cx.listener(|e, _: &UndoSelection, _, cx| e.replay_selection_history(false, cx)),
            )
            .on_action(
                cx.listener(|e, _: &RedoSelection, _, cx| e.replay_selection_history(true, cx)),
            )
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let mut layout = layout;
                        layout.origin = bounds.origin;
                        let viewport = window.content_mask().bounds;
                        let bracket_marks = overlay_editor.update(cx, |e, _| {
                            layout.prepare_fold_gutter(e, viewport);
                            if e.focus.is_focused(window) {
                                e.source_bracket_marks()
                            } else {
                                Vec::new()
                            }
                        });
                        let completion = overlay_editor.update(cx, |editor, cx| {
                            editor.prepaint_source_completion(&layout, viewport, window, cx)
                        });
                        let (hover, overlay) = overlay_editor.read(cx).overlay(
                            &layout,
                            viewport,
                            window,
                            completion.is_some(),
                        );
                        #[cfg(test)]
                        let overlay_text = overlay.as_ref().map(|overlay| overlay.text.clone());
                        let source_tooltip = if overlay.is_none()
                            && completion.is_none()
                            && overlay_editor.read(cx).source_tooltip.is_some()
                        {
                            overlay_editor.update(cx, |editor, cx| {
                                editor.prepaint_source_tooltip(&layout, viewport, window, cx)
                            })
                        } else {
                            None
                        };
                        let overlay = overlay
                            .and_then(|overlay| overlay.prepaint(theme, tooltip, window, cx));
                        #[cfg(test)]
                        overlay_editor.update(cx, |editor, _| {
                            editor.prepainted_overlay = overlay.as_ref().and(overlay_text);
                        });
                        (
                            layout,
                            hover,
                            overlay,
                            completion,
                            source_tooltip,
                            bracket_marks,
                        )
                    },
                    move |bounds,
                          (
                        mut layout,
                        hover,
                        overlay,
                        completion,
                        source_tooltip,
                        bracket_marks,
                    ),
                          window,
                          cx| {
                        #[cfg(target_os = "macos")]
                        layout.prepare_isolated_paint(isolation_cache.as_ref(), window, cx);
                        let e = editor.read(cx);
                        let selection = e.selection();
                        let native_selection = e.view.state().selection();
                        let secondary: Vec<_> = native_selection
                            .ranges()
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| *index != native_selection.main_index())
                            .map(|(_, range)| range.range())
                            .collect();
                        let caret = layout.bounds_at_selection(native_selection.main(), e.upstream);
                        let focus = e.focus.clone();
                        let focused = focus.is_focused(window);
                        let active_lines = layout.active_source_highlights(
                            e.view.state(),
                            focused,
                            e.input_enabled,
                        );
                        let diagnostic_marks = e.source_diagnostic_paint_marks();
                        if e.input_enabled {
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, editor.clone()),
                                cx,
                            );
                        }
                        if let Some(frame) = &layout.source_frame {
                            frame.paint(layout.origin, theme, window, cx);
                        }
                        let clip = window.content_mask().bounds;
                        let top = clip.top() - layout.origin.y;
                        let bottom = clip.bottom() - layout.origin.y;
                        let first = layout.rows.partition_point(|row| row.y + row.height <= top);
                        let last =
                            layout.rows[first..].partition_point(|row| row.y < bottom) + first;
                        let visible_rows = &layout.rows[first..last];
                        let (find_marks, selection_marks) = editor
                            .update(cx, |e, cx| {
                                let visible_ranges = visible_rows.iter().flat_map(|row| {
                                    row.items.iter().filter_map(|item| {
                                        let left = layout.origin.x + item.left;
                                        (!matches!(item.paint, ItemPaint::Fold(_))
                                            && left < clip.right()
                                            && left + item.width > clip.left())
                                        .then(|| item.source.clone())
                                    })
                                });
                                let viewport =
                                    Viewport::around(e.view.state().text(), visible_ranges).ok()?;
                                e.view.set_viewport_geometry(viewport).ok()?;
                                let find = e.find_marks(cx);
                                let selection = e.selection_match_marks(focused);
                                Some((find, selection))
                            })
                            .unwrap_or_default();
                        for row in visible_rows {
                            if let Some(frame) = &layout.source_frame {
                                active_lines.paint_row(
                                    frame,
                                    layout.origin,
                                    row,
                                    theme,
                                    window,
                                    #[cfg(test)]
                                    &mut layout.painted_active_lines,
                                );
                                frame.paint_gutter_row(
                                    layout.origin,
                                    row,
                                    theme,
                                    window,
                                    cx,
                                    #[cfg(test)]
                                    &mut layout.painted_active_lines,
                                );
                            }
                            let row_bounds = Bounds::new(
                                layout.origin + point(px(0.), row.y),
                                size(row.width, row.height),
                            );
                            for item in &row.items {
                                let top = layout.origin + point(item.left, row.y);
                                let framed = item.framed_graphic()
                                    && matches!(&item.paint, ItemPaint::Graphic(_));
                                if framed {
                                    window.paint_quad(quad(
                                        Bounds::new(
                                            top + point(px(0.), px(4.)),
                                            size(item.width, row.height - px(8.)),
                                        ),
                                        px(8.),
                                        theme.preview,
                                        gpui::Edges::all(px(1.)),
                                        theme.border,
                                        Default::default(),
                                    ));
                                }
                                if let ItemPaint::Block(block) = &item.paint {
                                    block.paint_frame(top, theme, window);
                                }
                                let image_bounds = if let ItemPaint::Graphic(graphic) = &item.paint
                                {
                                    let bounds = Bounds::new(
                                        top + point(
                                            px(if framed { 12. } else { 0. }),
                                            (row.height - graphic.height) / 2.,
                                        ),
                                        size(graphic.width, graphic.height),
                                    );
                                    // The math renderer outputs dark glyphs on a transparent raster.
                                    if dark
                                        && item.target.as_ref().is_some_and(|section| {
                                            matches!(
                                                section.kind,
                                                SectionKind::InlineMath | SectionKind::DisplayMath
                                            )
                                        })
                                    {
                                        window.paint_quad(fill(bounds, rgb(0xffffff)));
                                    }
                                    Some(bounds)
                                } else {
                                    None
                                };
                                if item.is_text() {
                                    source_brackets::paint_source_brackets(
                                        &bracket_marks,
                                        item,
                                        row_bounds,
                                        theme.view,
                                        window,
                                        #[cfg(test)]
                                        &mut layout.painted_bracket_quads,
                                    );
                                }
                                if let Some(marks) = &selection_marks
                                    && item.is_text()
                                {
                                    document_find::paint_selection_matches(
                                        marks,
                                        find_marks.as_deref(),
                                        item,
                                        row_bounds,
                                        theme,
                                        window,
                                        #[cfg(test)]
                                        &mut layout.painted_find_quads,
                                    );
                                }
                                document_find::paint_find_and_selection(
                                    find_marks.as_deref(),
                                    (&selection, &secondary),
                                    item,
                                    row_bounds,
                                    theme,
                                    window,
                                    #[cfg(test)]
                                    &mut layout.painted_find_quads,
                                );
                                match &item.paint {
                                    ItemPaint::Text(line) => {
                                        let _ = line.paint(top, row.height, window, cx);
                                    }
                                    #[cfg(target_os = "macos")]
                                    ItemPaint::Isolated(native) => native.paint(top, window),
                                    ItemPaint::Fold(line) => {
                                        let _ = line.paint(top, row.height, window, cx);
                                    }
                                    ItemPaint::Block(block) => block.paint_text(top, window, cx),
                                    ItemPaint::Graphic(graphic) => {
                                        let _ = window.paint_image(
                                            image_bounds.expect("a graphic has image bounds"),
                                            Default::default(),
                                            graphic.image.clone(),
                                            0,
                                            false,
                                        );
                                    }
                                }
                                if item.is_text() {
                                    source_diagnostics::paint_range_marks(
                                        &diagnostic_marks,
                                        item,
                                        row_bounds,
                                        theme,
                                        dark,
                                        window,
                                    );
                                }
                            }
                        }
                        source_diagnostics::paint_point_marks(
                            &layout,
                            &diagnostic_marks,
                            theme,
                            dark,
                            window,
                        );
                        layout.paint_placeholders(window, cx);
                        draw_selection::paint(
                            editor.read(cx),
                            &mut layout,
                            caret,
                            clip,
                            window,
                            cx,
                        );
                        drop_cursor::paint(
                            editor.read(cx),
                            &mut layout,
                            drop_cursor_revision,
                            clip,
                            window,
                        );
                        #[cfg(test)]
                        let mut painted_source_tooltip = None;
                        if let Some(source_tooltip) = source_tooltip {
                            source_tooltip.paint(
                                window,
                                cx,
                                #[cfg(test)]
                                &mut painted_source_tooltip,
                            );
                        }
                        if let Some(mut overlay) = overlay {
                            overlay.paint(window, cx);
                        }
                        if let Some(mut completion) = completion {
                            completion.paint(window, cx);
                        }
                        editor.update(cx, |e, _| {
                            #[cfg(test)]
                            {
                                e.painted_source_tooltip = painted_source_tooltip;
                            }
                            // Scroll bounds and wrap positions now match this frame.
                            let reveal_caret = std::mem::take(&mut e.reveal_caret);
                            let snapshot = e.view.take_scroll_snapshot();
                            let scrolled = if let Some(reveal) = e.reveal_source.take()
                                && !e.is_composing()
                            {
                                layout.source_bounds(reveal.offset).is_some_and(|bounds| {
                                    e.reveal_source_bounds(reveal.offset, bounds, reveal.alignment)
                                })
                            } else if !reveal_caret
                                && !e.is_composing()
                                && let Some(snapshot) = snapshot
                            {
                                e.restore_source_scroll(&layout, snapshot)
                            } else {
                                caret.is_some_and(|bounds| {
                                    reveal_caret
                                        && focus.is_focused(window)
                                        && e.reveal_source_bounds(
                                            e.cursor(),
                                            bounds,
                                            RevealAlignment::Nearest,
                                        )
                                })
                            };
                            if scrolled {
                                window.on_next_frame(|window, _| window.refresh());
                            }
                            e.layout = Some(layout);
                            e.hover = hover;
                        });
                    },
                )
                .w(width)
                .h(height)
                .map(|content| match end_padding {
                    // Put the padding on the content so retained padding in a
                    // short viewport cannot make the viewport itself taller.
                    Some(padding) => div().w(width).pb(padding).child(content).into_any_element(),
                    None => content.into_any_element(),
                }),
            )
            .into_any_element()
    }

    fn overlay(
        &self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        window: &Window,
        completion_visible: bool,
    ) -> (Option<Range<usize>>, Option<Overlay>) {
        let pointer = window.mouse_position();
        let link = (!self.selecting && viewport.contains(&pointer))
            .then(|| Self::hovered_link(layout, pointer))
            .flatten();
        let hover = link.map(|item| item.source.clone());
        let caret = (self.selection().is_empty() && self.focus.is_focused(window))
            .then(|| {
                layout.bounds_at_selection(self.view.state().selection().main(), self.upstream)
            })
            .flatten();
        if let Some(message) = &self.message {
            return (
                hover,
                Some(Overlay {
                    text: message.clone(),
                    anchor: None,
                    viewport,
                    caret,
                    prefer_above: false,
                }),
            );
        }
        let overlay = link.and_then(|item| {
            let section = item.target.as_ref()?;
            let text = match self.link_target(section) {
                Ok(crate::media::Target::Web(url)) => url.to_string(),
                Ok(crate::media::Target::Local(path)) => path.display().to_string(),
                Err(_) => section.destination.clone().unwrap_or_default(),
            };
            let row = layout.row_at(pointer);
            let anchor = Bounds::new(
                layout.origin + point(item.left, row.y),
                size(item.width, row.height),
            );
            (!text.is_empty()).then_some(Overlay {
                text,
                anchor: Some(anchor),
                viewport,
                caret,
                prefer_above: false,
            })
        });
        let overlay = overlay.or_else(|| self.source_diagnostic_tooltip(layout, viewport, window));
        #[cfg(target_os = "macos")]
        let overlay = overlay.or_else(|| {
            if completion_visible {
                None
            } else {
                self.source_lsp_tooltip(layout, viewport, window, caret)
            }
        });
        #[cfg(not(target_os = "macos"))]
        let _ = completion_visible;
        (hover, overlay)
    }
}

struct Overlay {
    text: String,
    anchor: Option<Bounds<Pixels>>,
    viewport: Bounds<Pixels>,
    caret: Option<Bounds<Pixels>>,
    prefer_above: bool,
}

impl Overlay {
    fn prepaint(
        self,
        theme: Theme,
        tooltip: Option<gpui_editor_codemirror::theme_one_dark::TooltipStyle>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let viewport = self.viewport.inset(px(10.));
        if viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
            return None;
        }
        let (background, foreground) = if self.anchor.is_some() {
            (
                tooltip.map_or(theme.tooltip, |style| rgba(style.background.rgba32())),
                theme.on_tooltip,
            )
        } else {
            (theme.error_bg, theme.error)
        };
        // This element has neither event handlers nor a hitbox. It floats over
        // the scroll viewport without consuming space or intercepting input.
        let element = div()
            .max_w(viewport.size.width.min(px(440.)))
            .max_h(viewport.size.height)
            .overflow_hidden()
            .px_3()
            .py_2()
            .rounded_md()
            .text_sm()
            .bg(background)
            .text_color(foreground)
            .child(self.text);
        #[cfg(test)]
        let element = element
            .debug_selector(|| "stock-editor-overlay".into())
            .children(self.anchor.and_then(|_| {
                one_dark_tests::overlay_text_probe(one_dark_tests::OverlayText::Anchored)
            }));
        let mut element = element.into_any_element();
        let size = element.layout_as_root(
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            cx,
        );
        let mut origin = if let Some(anchor) = self.anchor {
            let below = anchor.bottom() + px(4.);
            let above = anchor.top() - size.height - px(4.);
            point(
                anchor.left(),
                if self.prefer_above && above >= viewport.top()
                    || below + size.height > viewport.bottom()
                {
                    above
                } else {
                    below
                },
            )
        } else {
            viewport.bottom_right() - point(size.width, size.height)
        };
        origin.x = origin.x.clamp(
            viewport.left(),
            (viewport.right() - size.width).max(viewport.left()),
        );
        origin.y = origin.y.clamp(
            viewport.top(),
            (viewport.bottom() - size.height).max(viewport.top()),
        );
        if self
            .caret
            .is_some_and(|caret| caret.intersects(&Bounds::new(origin, size)))
        {
            origin.y = if origin.y > viewport.center().y {
                viewport.top()
            } else {
                (viewport.bottom() - size.height).max(viewport.top())
            };
        }
        element.prepaint_at(origin, window, cx);
        Some(element)
    }
}

#[cfg(test)]
pub(crate) enum TestPaint<'a> {
    Text(&'a str),
    Graphic(&'a gpui::RenderImage),
    Block,
}

#[cfg(test)]
impl Editor {
    pub(crate) fn test_painted_item(&self, offset: usize) -> (Point<Pixels>, TestPaint<'_>) {
        let layout = self.layout.as_ref().expect("rendered document");
        for row in &layout.rows {
            if let Some(item) = row.items.iter().find(|item| item.source.contains(&offset)) {
                let center =
                    layout.origin + point(item.left + item.width / 2., row.y + row.height / 2.);
                let paint = match &item.paint {
                    ItemPaint::Text(line) => TestPaint::Text(line.text.as_ref()),
                    ItemPaint::Graphic(graphic) => TestPaint::Graphic(&graphic.image),
                    ItemPaint::Block(_) => TestPaint::Block,
                    _ => panic!("unexpected paint at source offset {offset}"),
                };
                return (center, paint);
            }
        }
        panic!("no item at source offset {offset}");
    }

    pub(crate) fn test_source_frame_is_visible(&self) -> bool {
        self.layout
            .as_ref()
            .expect("rendered document")
            .source_frame
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gpui::{ClipboardItem, Modifiers, TestAppContext};
    use gpui_editor_codemirror::autocomplete::brackets::close_brackets_enabled;

    use super::*;

    #[gpui::test]
    fn host_focus_delivers_effects_outside_layout_and_rejects_document_edits(
        cx: &mut TestAppContext,
    ) {
        use gpui_editor_codemirror::{
            StateEffectType, StateField, transaction_filter, view::focus_change_effect,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("a字b".into(), window, cx));
        let affected = StateEffectType::<bool>::new();
        let field = StateField::<Vec<bool>>::new(|_| Ok(Vec::new()), {
            let affected = affected.clone();
            move |previous, update| {
                let mut next = previous.clone();
                next.extend(
                    update
                        .transaction()
                        .effects()
                        .iter()
                        .filter_map(|effect| effect.get(&affected))
                        .copied(),
                );
                Ok(next)
            }
        });
        let reject = Arc::new(AtomicBool::new(false));
        cx.update(|window, cx| {
            window.activate_window();
            bind_keys(cx);
            editor.update(cx, |editor, _| {
                let reject = reject.clone();
                let config = Extension::group([
                    field.extension(),
                    focus_change_effect(move |_, focus| Some(affected.of(focus))),
                    transaction_filter(move |_, transaction| {
                        if reject.load(Ordering::Relaxed) && transaction.is_focus_change() {
                            let changes =
                                ChangeSet::single(Change::new(0..1, "not allowed")).unwrap();
                            Ok(Transaction::new(changes))
                        } else {
                            Ok(transaction)
                        }
                    }),
                ]);
                editor
                    .view
                    .update(
                        Transaction::default().with_effects([config.append()]),
                        None,
                        |_| (None, None),
                    )
                    .unwrap();
                window.focus(&editor.focus);
            });
        });
        cx.run_until_parked();
        let observe = |editor: &Editor, focused, expected: &[bool]| {
            assert_eq!(editor.view.focused(), focused);
            assert_eq!(
                editor
                    .view
                    .state()
                    .context()
                    .field(&field)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                expected
            );
            assert_eq!(editor.source(), "a字b");
            assert!(!editor.view.can_undo());
        };
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                observe(editor, true, &[true]);
                editor.open_find(window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| observe(editor.read(cx), false, &[true, false]));
        reject.store(true, Ordering::Relaxed);
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|_, cx| observe(editor.read(cx), true, &[true, false]));
    }

    #[gpui::test]
    fn selection_history_shortcuts_restore_ranges_before_falling_back_to_text_history(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("a字b".into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
            cx.background_executor().clone()
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("right");
        cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 1..1));
        executor.advance_clock(Duration::from_millis(501));
        cx.simulate_keystrokes("shift-right");
        cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 1..4));
        cx.simulate_keystrokes("cmd-u");
        cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 1..1));
        cx.simulate_keystrokes("cmd-shift-u");
        cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 1..4));
        cx.simulate_keystrokes("cmd-u cmd-u");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.selection(), 0..0);
            assert_eq!(e.source(), "a字b");
            assert!(!e.view.can_undo());
        });
        cx.simulate_input("X");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "Xa字b"));
        cx.simulate_keystrokes("cmd-u");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "a字b"));
    }

    #[gpui::test]
    fn navigation_never_uses_layout_offsets_from_before_an_edit(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("a好".into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.layout.is_some());
                assert!(e.replace(0..1, "", cx));
                assert!(e.layout.is_none());
                e.move_horizontal(true, false, cx);
                assert_eq!(
                    e.selected_text_range(false, window, cx).unwrap().range,
                    0..0
                );
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                e.move_horizontal(true, false, cx);
                assert_eq!(e.cursor(), "好".len());
            })
        });
    }

    #[gpui::test]
    fn vertical_movement_keeps_the_column_and_extends_selection(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("ABCDE\nX\nABCDE".into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                e.move_to(4, false, cx);
            })
        });
        cx.update(|_, cx| bind_keys(cx));
        cx.simulate_keystrokes("down shift-down");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.anchor(), "ABCDE\nX".len());
            assert_eq!(e.cursor(), "ABCDE\nX\nABCD".len());
        });
    }

    #[gpui::test]
    fn source_brackets_preserve_empty_fences_and_history(cx: &mut TestAppContext) {
        let original = "```mermaid\r\n```\r\n\r\nOutside [ ]";
        let paired = "```mermaid\r\n[]\r\n```\r\n\r\nOutside [ ]";
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new(original.into(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                let section = e
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| section.kind == SectionKind::Mermaid)
                    .unwrap();
                e.activate(section, false, cx);
                let start = e.cursor();
                assert_eq!(close_brackets_enabled(e.view.state()), Ok(false));
                e.replace_text_in_range(None, "[", window, cx);
                assert_eq!(close_brackets_enabled(e.view.state()), Ok(true));
                assert_eq!(e.source(), paired);
                assert_eq!(e.cursor(), start + 1);
                assert_eq!(e.active.as_ref().unwrap().content, start..start + 2);
                let hit = e.source_bracket_match().unwrap();
                assert_eq!(hit.start, start..start + 1);
                assert_eq!(hit.end, Some(start + 1..start + 2));
                assert!(hit.matched);
                e.replace_text_in_range(None, "]", window, cx);
                assert_eq!(e.source(), paired);
                assert_eq!(e.cursor(), start + 2);
            });
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), original));
        cx.simulate_keystrokes("secondary-shift-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), paired));
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.source(), "```mermaid\r\n\r\n```\r\n\r\nOutside [ ]");
            let start = "```mermaid\r\n".len();
            assert_eq!(e.active.as_ref().unwrap().content, start..start);
        });
    }

    #[gpui::test]
    fn source_bracket_input_wraps_a_directed_range_but_does_not_pair_ime_text(
        cx: &mut TestAppContext,
    ) {
        let original = "---\nname: 字\n---\n\nOutside ]";
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new(original.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let section = e
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| section.kind == SectionKind::FrontMatter)
                    .unwrap();
                e.activate(section, false, cx);
                let start = e.source().find('字').unwrap();
                let len = '字'.len_utf8();
                e.set_selection(start + len, start);
                e.replace_text_in_range(None, "\"", window, cx);
                assert_eq!(e.source(), "---\nname: \"字\"\n---\n\nOutside ]");
                assert_eq!((e.anchor(), e.cursor()), (start + len + 1, start + 1));
                e.replace_and_mark_text_in_range(None, "(", None, window, cx);
                assert!(e.is_composing());
                assert!(e.source_bracket_match().is_none());
                e.replace_text_in_range(None, "[", window, cx);
                assert!(!e.is_composing());
                assert_eq!(e.source(), "---\nname: \"[\"\n---\n\nOutside ]");
                let hit = e.source_bracket_match().unwrap();
                assert_eq!(hit.start, start + 1..start + 2);
                assert!(hit.end.is_none());
                assert!(!hit.matched);
            });
        });
    }

    #[gpui::test]
    fn source_group_shortcuts_preserve_crlf_scope_and_undo(cx: &mut TestAppContext) {
        let original = "---\r\ntitle: alpha 世界\r\n---\r\n\r\nAfter";
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new(original.into(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                let section = e
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| section.kind == SectionKind::FrontMatter)
                    .unwrap();
                e.activate(section, false, cx);
            });
        });
        cx.simulate_keystrokes("alt-backspace");
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                assert_eq!(e.source(), original);
                let caret = e.source().find('界').unwrap() + '界'.len_utf8();
                e.move_to(caret, false, cx);
            });
        });
        cx.simulate_keystrokes("alt-shift-left");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(&e.source()[e.selection()], "世界");
        });
        cx.simulate_keystrokes("alt-backspace");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.source(), original.replace("世界", ""));
            assert_eq!(e.active.as_ref().unwrap().kind, SectionKind::FrontMatter);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.source(), original);
            assert_eq!(&e.source()[e.selection()], "世界");
        });
    }

    #[gpui::test]
    fn caret_reveal_scrolls_vertically_after_wrapped_paste(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                e.replace(
                    0..0,
                    &format!("{}{}", "line\n".repeat(50), "x".repeat(250)),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.scroll.offset().x, px(0.));
            assert_eq!(e.scroll.max_offset().width, px(0.));
            assert!(e.scroll.offset().y < px(0.));
        });
    }

    #[gpui::test]
    fn source_reveal_uses_fresh_layout_and_keeps_the_draft_selection_and_undo(
        cx: &mut TestAppContext,
    ) {
        let original = "# 標題\n\n後文";
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new(original.into(), window, cx));
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.update(|_, cx| bind_keys(cx));
        let prefix = format!("開頭\n\n{}", "a long wrapped paragraph ".repeat(110)) + "\n\n";
        let draft = format!("{prefix}{original}");
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                assert_eq!(e.headings()[0].offset, 0);
                assert!(e.replace(0..0, &prefix, cx));
                assert!(e.layout.is_none());
                e.set_selection("開頭".len(), 0);
                let headings = e.headings();
                assert_eq!(headings[0].offset, prefix.len());
                e.reveal_source(headings[0].offset, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(editor.read(cx).scroll.offset().y < px(0.));
            window.refresh();
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            let target = e
                .layout
                .as_ref()
                .unwrap()
                .source_bounds(prefix.len())
                .unwrap();
            assert!(e.scroll.bounds().contains(&target.center()));
            assert_eq!((e.anchor(), e.cursor()), ("開頭".len(), 0));
            assert_eq!(e.source(), draft);
            assert!(e.view.can_undo());
            assert!(!e.view.can_redo());
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.source(), original);
            assert_eq!(e.headings()[0].offset, 0);
        });
    }

    #[gpui::test]
    fn source_reveal_uses_the_heading_row_for_hidden_setext_markers(cx: &mut TestAppContext) {
        let source = format!(
            "{}\nSetext heading\r\n==============\r\n\nNext row",
            "line\n\n".repeat(25)
        );
        let heading = source.find("Setext heading").unwrap();
        let underline = source.find("====").unwrap();
        let next = source.find("Next row").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.update(|_, cx| editor.update(cx, |e, cx| e.reveal_source(underline, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(editor.read(cx).scroll.offset().y < px(0.));
            window.refresh();
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            let layout = e.layout.as_ref().unwrap();
            let revealed = layout.source_bounds(underline).unwrap();
            assert_eq!(revealed.top(), layout.source_bounds(heading).unwrap().top());
            assert!(revealed.top() < layout.source_bounds(next).unwrap().top());
            assert!(e.scroll.bounds().contains(&revealed.center()));
        });
    }

    #[gpui::test]
    fn source_reveal_ignores_invalid_offsets_and_ime_composition(cx: &mut TestAppContext) {
        let source = format!("首行\n\n{}\n# 標題", "line\n".repeat(60));
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.reveal_source(1, cx);
                e.reveal_source(usize::MAX, cx);
                assert_eq!(e.reveal_source, None);
                window.focus(&e.focus);
                e.replace_and_mark_text_in_range(None, "組字", None, window, cx);
                assert!(e.is_composing());
                e.reveal_source(e.headings()[0].offset, cx);
                assert_eq!(e.reveal_source, None);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert!(e.is_composing());
            assert_eq!(e.scroll.offset().y, px(0.));
        });
    }

    #[gpui::test]
    fn wrapping_tracks_the_editor_pane_and_keeps_a_fixed_left_edge(cx: &mut TestAppContext) {
        struct Pane {
            editor: gpui::Entity<Editor>,
            width: Pixels,
        }
        impl Render for Pane {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(self.width).h_full().child(self.editor.clone())
            }
        }
        let source = format!(
            "# {}\n\n{}",
            "A wide **heading** ".repeat(5),
            "A regular paragraph with *emphasis* and **bold**. ".repeat(10)
        );
        let (pane, cx) = cx.add_window_view(|window, cx| Pane {
            editor: cx.new(|cx| Editor::new(source.clone(), window, cx)),
            width: px(240.),
        });
        let check = |editor: &Editor, width| {
            assert_eq!(editor.source(), source);
            assert_eq!(editor.scroll.bounds().size.width, width);
            assert_eq!(editor.scroll.max_offset().width, px(0.));
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.origin.x, px(HORIZONTAL_PADDING));
            assert!(layout.rows.iter().all(|row| row.width <= wrap_width(width)));
            layout.rows.len()
        };
        let narrow = cx.update(|_, cx| check(pane.read(cx).editor.read(cx), px(240.)));
        cx.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                pane.width = px(420.);
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert!(cx.update(|_, cx| check(pane.read(cx).editor.read(cx), px(420.))) < narrow);
    }

    #[gpui::test]
    fn caret_can_hit_both_sides_of_a_soft_break_and_move_by_visual_row(cx: &mut TestAppContext) {
        let source = "a".repeat(100);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.simulate_resize(size(px(230.), px(600.)));
        let (position, offset, next_row_y) = cx.update(|_, cx| {
            let layout = editor.read(cx).layout.as_ref().unwrap();
            let row = &layout.rows[0];
            let end_of_row = layout.origin + point(row.width + px(2.), row.height / 2.);
            let (offset, upstream) = layout.position_at(end_of_row);
            assert!(upstream);
            assert_eq!(
                layout.position_at(layout.origin + point(px(0.), layout.rows[1].y + px(1.))),
                (offset, false)
            );
            (end_of_row, offset, layout.origin.y + layout.rows[1].y)
        });
        cx.simulate_click(position, Modifiers::none());
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.cursor(), offset);
                let caret = editor
                    .bounds_for_range(offset..offset, editor.scroll.bounds(), window, cx)
                    .unwrap();
                assert!(caret.top() < next_row_y);
                editor.move_vertical(true, true, cx);
                let caret = editor
                    .layout
                    .as_ref()
                    .unwrap()
                    .bounds_at(editor.cursor(), editor.upstream)
                    .unwrap();
                assert_eq!(caret.top(), next_row_y);
                assert_eq!(editor.anchor(), offset);
                assert!(editor.cursor() > offset);
                assert_eq!(editor.source(), source);
            })
        });
    }

    #[gpui::test]
    fn wrapped_composition_keeps_graphemes_and_utf16_offsets(cx: &mut TestAppContext) {
        let grapheme = "👩‍💻e\u{301}";
        let marked = grapheme.repeat(32);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.simulate_resize(size(px(190.), px(680.)));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_and_mark_text_in_range(None, &marked, None, window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                use unicode_segmentation::UnicodeSegmentation;
                let units = marked.encode_utf16().count();
                assert_eq!(editor.marked_text_range(window, cx), Some(0..units));
                let layout = editor.layout.as_ref().unwrap();
                assert!(layout.rows.len() > 1);
                let starts: Vec<_> = marked
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain([marked.len()])
                    .collect();
                for row in &layout.rows {
                    assert!(row.width <= wrap_width(px(190.)));
                    for item in &row.items {
                        assert!(
                            starts.contains(&item.source.start)
                                && starts.contains(&item.source.end)
                        );
                    }
                }
                let second = &layout.rows[1];
                let position = layout.origin + point(px(0.), second.y + px(1.));
                let index = editor
                    .character_index_for_point(position, window, cx)
                    .unwrap();
                let bounds = editor
                    .bounds_for_range(index..index, editor.scroll.bounds(), window, cx)
                    .unwrap();
                assert_eq!(bounds.top(), position.y - px(1.));
                editor.replace_text_in_range(None, "完成", window, cx);
                assert_eq!(editor.source(), "完成");
                assert!(!editor.is_composing());
            })
        });
    }

    #[gpui::test]
    fn active_link_source_wraps_without_hiding_markdown_bytes(cx: &mut TestAppContext) {
        let source = format!("[A link](https://example.com/{})", "segment".repeat(16));
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.simulate_resize(size(px(230.), px(460.)));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.command(Command::EditLink, window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.active.as_ref().unwrap().kind, SectionKind::Link);
            assert_eq!(e.scroll.max_offset().width, px(0.));
            let rows = &e.layout.as_ref().unwrap().rows;
            assert!(rows.len() > 2);
            let mut end = 0;
            for item in rows.iter().flat_map(|row| &row.items) {
                assert_eq!(item.source.start, end);
                assert!(item.target.is_none());
                end = item.source.end;
            }
            assert_eq!(end, source.len());
            assert_eq!(e.source(), source);
        });
    }

    #[gpui::test]
    fn a_wide_code_preview_can_still_scroll_horizontally(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(
                format!("```\n{}\n```\n\nAfter the preview", "x".repeat(100)),
                window,
                cx,
            )
        });
        cx.simulate_resize(size(px(230.), px(260.)));
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.scroll.max_offset().width > px(0.));
            let rows = &editor.layout.as_ref().unwrap().rows;
            assert!(matches!(rows[0].items[0].paint, ItemPaint::Block(_)));
            assert!(
                rows.iter()
                    .skip(1)
                    .all(|row| row.width <= wrap_width(px(230.)))
            );
        });
    }

    #[gpui::test]
    fn link_and_message_overlays_do_not_change_layout(cx: &mut TestAppContext) {
        let executor = cx.executor().clone();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(
                "[Link](https://example.com/path)\n\nOther text".into(),
                window,
                cx,
            )
        });
        cx.simulate_resize(size(px(345.), px(240.)));
        let (link, other, viewport) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            (
                layout.origin + point(px(8.), px(10.)),
                layout
                    .bounds_at(editor.source().len(), false)
                    .unwrap()
                    .center(),
                editor.scroll.bounds(),
            )
        });
        cx.simulate_mouse_move(link, None, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert!(editor.hover.is_some());
            let layout = editor.layout.as_ref().unwrap();
            let (_, tooltip) = editor.overlay(layout, viewport, window, false);
            assert_eq!(tooltip.unwrap().text, "https://example.com/path");
            assert_eq!(editor.scroll.bounds(), viewport);
        });
        cx.simulate_mouse_move(other, None, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert!(editor.hover.is_none());
                editor.message = Some("Cannot edit this Markdown range".into());
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.message.is_some());
            assert_eq!(editor.scroll.bounds(), viewport);
            assert_eq!(
                editor
                    .layout
                    .as_ref()
                    .unwrap()
                    .bounds_at(editor.source().len(), false)
                    .unwrap()
                    .center(),
                other
            );
        });
        executor.advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        cx.update(|_, cx| assert!(editor.read(cx).message.is_none()));
    }

    #[gpui::test]
    fn trailing_bold_markers_have_a_caret_stop_and_are_removed_with_last_character(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("**x**".into(), window, cx));
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                let layout = e.layout.as_ref().unwrap();
                assert!(layout.bounds_at(5, false).is_some());
                assert_eq!(layout.step(3, true), 5);
                e.move_to(5, false, cx);
                e.remove(false, cx);
                assert_eq!(e.source(), "");
                assert_eq!(e.cursor(), 0);
            })
        });
    }

    #[gpui::test]
    fn closing_heading_markers_have_a_caret_stop_before_the_line_break(cx: &mut TestAppContext) {
        let heading = "# Title ###";
        for eol in ["", "\n", "\r\n"] {
            let source = format!("{heading}{eol}");
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
            cx.update(|window, cx| {
                editor.update(cx, |e, cx| {
                    window.focus(&e.focus);
                    e.move_to("# Title".len(), false, cx);
                });
                bind_keys(cx);
            });
            cx.run_until_parked();
            cx.simulate_keystrokes("right");
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.cursor(), heading.len());
                let layout = editor.layout.as_ref().unwrap();
                assert!(layout.bounds_at(editor.cursor(), editor.upstream).is_some());
                assert_eq!(
                    layout.index_at(layout.origin + point(px(500.), px(0.))),
                    heading.len()
                );
            });
            cx.simulate_keystrokes("enter");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), format!("{heading}\n{eol}")));
        }
    }

    #[gpui::test]
    fn inline_code_hides_delimiters_until_the_caret_enters_and_keeps_the_source(
        cx: &mut TestAppContext,
    ) {
        let source = "Before `ls -la` after";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        let code = 8..14;
        let visible = |e: &Editor, expected: &str| {
            let layout = e.layout.as_ref().unwrap();
            assert!(layout.source_frame.is_none());
            assert!(layout.rows.iter().flat_map(|row| &row.items).any(
                |item| matches!(&item.paint, ItemPaint::Text(line) if line.text.as_ref() == expected)
            ));
        };
        let hit = cx.update(|_, cx| {
            let e = editor.read(cx);
            visible(e, "ls -la");
            let layout = e.layout.as_ref().unwrap();
            layout.bounds_at(code.start + 1, false).unwrap().center()
        });
        cx.simulate_click(hit, Modifiers::none());
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.cursor(), code.start + 1);
            visible(e, "`ls -la`");
        });
        cx.simulate_input("X");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "Before `lXs -la` after"));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.source(), source);
            assert_eq!(e.cursor(), code.start + 1);
        });
        cx.simulate_keystrokes("left left");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.cursor(), code.start - 1);
            visible(e, "ls -la");
        });
        cx.simulate_keystrokes("right");
        cx.update(|_, cx| {
            let e = editor.read(cx);
            assert_eq!(e.cursor(), code.start);
            visible(e, "`ls -la`");
        });
    }

    #[gpui::test]
    fn deletion_keys_respect_the_read_only_facet(cx: &mut TestAppContext) {
        use gpui_editor_codemirror::READ_ONLY;

        let source = "Read only prose";
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source.into(), READ_ONLY.of(true), window, cx).unwrap()
        });
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        for (anchor, head) in [(5, 9), (5, 5)] {
            for key in ["backspace", "delete"] {
                cx.update(|_, cx| {
                    editor.update(cx, |editor, _| editor.set_selection(anchor, head))
                });
                cx.simulate_keystrokes(key);
                cx.update(|_, cx| {
                    let editor = editor.read(cx);
                    assert_eq!(editor.source(), source, "{key} at {anchor}..{head}");
                    assert_eq!(editor.selection(), anchor..head);
                    assert!(!editor.view.can_undo());
                });
            }
        }
    }

    #[gpui::test]
    fn deleting_next_to_inline_code_skips_hidden_backticks(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("`x` followed".into(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("delete");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), " followed"));
        cx.simulate_keystrokes("secondary-z right backspace");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "x` followed"));
    }

    #[gpui::test]
    fn backspace_in_source_mode_deletes_a_visible_marker(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new("[**xy**](https://example.com)".into(), window, cx)
        });
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                window.focus(&e.focus);
                let link = e
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|s| s.kind == SectionKind::Link);
                e.activate(link.unwrap(), false, cx);
                e.move_to(3, false, cx);
            });
            bind_keys(cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "[*xy**](https://example.com)"));
    }

    #[gpui::test]
    fn empty_mermaid_body_has_a_caret_stop(cx: &mut TestAppContext) {
        let source = "```mermaid\n```";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let block = e
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|s| s.kind == SectionKind::Mermaid);
                e.activate(block.unwrap(), false, cx);
                let layout = e.shape(window, cx);
                assert_eq!(e.cursor(), "```mermaid\n".len());
                let caret = layout.bounds_at(e.cursor(), e.upstream).unwrap().center();
                assert!(layout.source_frame_contains(caret));
                assert_eq!(layout.index_at(caret), e.cursor());
                assert_eq!(e.source(), source);
            });
        });
    }

    #[gpui::test]
    fn preview_cap_does_not_label_a_valid_formula_invalid(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let formulas: Vec<_> = (0..129).map(|n| format!("x^{n}")).collect();
                let source: String = formulas.iter().map(|body| format!("${body}$\n")).collect();
                e.view = EditorView::new(EditorState::new(source));
                for body in formulas.into_iter().take(128) {
                    e.graphics.insert(
                        (
                            body,
                            SectionKind::InlineMath,
                            window.scale_factor().to_bits(),
                        ),
                        Err("test cached preview".into()),
                    );
                }
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            let item = e.layout.as_ref().unwrap().rows.iter().rev()
                .find_map(|row| row.items.last()).unwrap();
            assert!(matches!(&item.paint, ItemPaint::Text(line) if line.text.as_ref() == "Preview limit reached"));
        });
    }

    #[gpui::test]
    fn remote_images_need_consent_and_old_requests_do_not_replace_new_ones(
        cx: &mut TestAppContext,
    ) {
        let image_url = "https://example.invalid/picture.png";
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(format!("![Preview]({image_url})"), window, cx)
        });
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                assert!(e.has_blocked_images());
                e.allow_remote_images(cx);
                assert!(!e.has_blocked_images());
                e.set_path(None, None);
                assert!(e.has_blocked_images());
                assert_eq!(e.image_requests, 0);
                assert!(e.images.is_empty());
                e.images
                    .insert("old.png".into(), ImageEntry::Ready(Err("old".into())));
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| editor.update(cx, |e, _| {
            assert!(e.images.is_empty());
            e.images.insert("same.png".into(), ImageEntry::Loading(1));
            e.image_requests = 2;
            e.set_path(Some("new/doc.md".into()), None);
            e.images.insert("same.png".into(), ImageEntry::Loading(2));
            e.complete_image_request("same.png".into(), 1, Err("stale".into()));
            assert!(matches!(e.images.get("same.png"), Some(ImageEntry::Loading(2))));
            e.complete_image_request("same.png".into(), 2, Err("new".into()));
            assert!(matches!(e.images.get("same.png"), Some(ImageEntry::Ready(Err(message))) if message == "new"));
            assert_eq!(e.image_requests, 0);
        }));
    }

    #[gpui::test]
    fn typing_backticks_in_plain_text_does_not_complete_a_fence(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::new(String::new(), window, cx);
            editor.set_path(Some("script.txt".into()), None);
            editor
        });
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        cx.simulate_input("```");
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).source(), "```");
            assert_eq!(editor.read(cx).cursor(), 3);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert!(editor.read(cx).source().is_empty()));
    }

    #[gpui::test]
    fn typing_an_opening_code_fence_leaves_the_caret_on_an_editable_blank_line(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        for _ in 0..3 {
            cx.simulate_input("`");
        }
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), "```\n\n```");
            assert_eq!(editor.selection(), 4..4);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "``"));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert!(editor.read(cx).source().is_empty()));
        cx.simulate_keystrokes("secondary-shift-z secondary-shift-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 4..4));
        cx.simulate_input("let x = 1;");
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "```\nlet x = 1\n```"));
        cx.simulate_input(";");
        let outside = cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), "```\nlet x = 1;\n```");
            assert_eq!(editor.active.as_ref().unwrap().kind, SectionKind::DraftCode);
            assert!(editor.message.is_none());
            let layout = editor.layout.as_ref().unwrap();
            let frame = layout.source_frame.as_ref().unwrap();
            layout.origin + point(px(12.), frame.bounds.bottom() + px(5.))
        });
        cx.simulate_click(outside, Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.active.is_none());
            assert!(editor.layout.as_ref().unwrap().rows.iter().any(|row| {
                row.items
                    .iter()
                    .any(|item| matches!(item.paint, ItemPaint::Block(_)))
            }));
        });
    }

    #[gpui::test]
    fn pasted_opening_code_fence_stays_editable_without_autocompletion(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
        });
        cx.run_until_parked();
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("```".into())));
        cx.simulate_keystrokes("secondary-v");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "```"));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert!(editor.read(cx).source().is_empty()));
        cx.simulate_keystrokes("secondary-shift-z backspace");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("`".into())));
        cx.simulate_keystrokes("secondary-v");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "```"));
        for text in ["rust", "\n", "let x = 1;", "\n", "`", "`", "`"] {
            cx.simulate_input(text);
        }
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), "```rust\nlet x = 1;\n```");
            assert!(editor.message.is_none());
            assert!(editor.layout.as_ref().unwrap().rows.iter().any(|row| {
                row.items
                    .iter()
                    .any(|item| matches!(item.paint, ItemPaint::Block(_)))
            }));
        });
    }

    #[gpui::test]
    fn code_and_tables_render_as_read_only_selection_units(cx: &mut TestAppContext) {
        let source = "Before\n\n| Name | Value |\n| --- | ---: |\n| First | 1 |\n\n```rust\nfn main() {}\n```\n\nAfter";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|_, cx| bind_keys(cx));
        let blocks = cx.update(|_, cx| {
            let layout = editor.read(cx).layout.as_ref().unwrap();
            layout
                .rows
                .iter()
                .flat_map(|row| {
                    row.items
                        .iter()
                        .filter(|item| matches!(item.paint, ItemPaint::Block(_)))
                        .map(|item| {
                            (
                                layout.origin + point(item.left + px(12.), row.y + row.height / 2.),
                                item.source.clone(),
                            )
                        })
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(blocks.len(), 2);
        assert!(source[blocks[0].1.clone()].starts_with("| Name"));
        assert!(source[blocks[1].1.clone()].starts_with("```rust"));
        for (position, range) in blocks {
            cx.simulate_click(position, Modifiers::none());
            cx.update(|_, cx| {
                let e = editor.read(cx);
                assert_eq!(e.selection(), range);
                assert!(e.active.is_none());
            });
            cx.simulate_input("no change");
            cx.simulate_keystrokes("backspace");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
        }
        let after = cx.update(|_, cx| {
            editor
                .read(cx)
                .layout
                .as_ref()
                .unwrap()
                .bounds_at(source.len(), false)
                .unwrap()
                .center()
        });
        cx.simulate_click(after, Modifiers::none());
        cx.simulate_input("!");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), format!("{source}!")));
    }

    #[gpui::test]
    fn edits_markdown_in_one_surface(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.md");
        let original = include_str!("../../document/tests/fixtures/sample.md");
        fs::write(&path, original).unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(fs::read_to_string(&path).unwrap(), window, cx)
        });
        cx.update(|_, cx| bind_keys(cx));
        let (heading, formula, paragraph) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let heading = editor.source().find("原始標題").unwrap();
            let paragraph = editor.source().find("行內公式").unwrap();
            let formula = layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items
                        .iter()
                        .find(|item| {
                            matches!(item.paint, ItemPaint::Graphic(_))
                                && item
                                    .target
                                    .as_ref()
                                    .is_some_and(|section| section.kind == SectionKind::InlineMath)
                        })
                        .map(|item| {
                            layout.origin
                                + point(item.left + item.width / 2., row.y + row.height / 2.)
                        })
                })
                .expect("the formula must render as a graphic");
            (
                layout.bounds_at(heading, false).unwrap().center(),
                formula,
                layout.bounds_at(paragraph, false).unwrap().center(),
            )
        });
        cx.simulate_click(heading, Modifiers::none());
        cx.simulate_keystrokes("shift-right shift-right shift-right shift-right");
        cx.simulate_input("新標題");
        cx.simulate_click(formula, Modifiers::none());
        cx.update(|_, cx| assert!(editor.read(cx).active.is_some()));
        cx.simulate_keystrokes("secondary-a");
        cx.simulate_input("y^3");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.active.is_some());
            fs::write(&path, editor.source()).unwrap();
        });
        cx.simulate_click(paragraph, Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.active.is_none());
            assert!(editor.layout.as_ref().unwrap().rows.iter().any(|row| {
                row.items
                    .iter()
                    .any(|item| matches!(item.paint, ItemPaint::Graphic(_)))
            }));
        });
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            original
                .replacen("# 原始標題", "# 新標題", 1)
                .replacen("$x^2$", "$y^3$", 1)
        );
        cx.simulate_mouse_down(heading, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(paragraph, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(paragraph, MouseButton::Left, Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let selected = &editor.source()[editor.selection()];
            assert!(selected.contains("\n\n") || selected.contains("\r\n\r\n"));
        });
    }

    #[gpui::test]
    fn inserts_and_removes_editable_items(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        for kind in [
            SectionKind::FrontMatter,
            SectionKind::Mermaid,
            SectionKind::InlineMath,
            SectionKind::DisplayMath,
            SectionKind::Image,
            SectionKind::Link,
        ] {
            cx.update(|window, cx| {
                editor.update(cx, |e, cx| {
                    e.command(Command::Add(kind), window, cx);
                    assert_eq!(e.active.as_ref().map(|s| s.kind), Some(kind));
                    e.command(Command::RemoveItem, window, cx);
                    assert!(
                        e.markdown()
                            .sections()
                            .iter()
                            .all(|section| section.kind != kind)
                    );
                })
            });
        }
    }

    #[gpui::test]
    fn typing_undo_groups_adjacent_input_until_a_pause(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
            cx.background_executor().clone()
        });
        cx.run_until_parked();
        cx.simulate_input("ab");
        executor.advance_clock(Duration::from_millis(750));
        cx.simulate_input("好");
        executor.advance_clock(Duration::from_millis(750));
        cx.simulate_input("d");
        executor.advance_clock(Duration::from_secs(1));
        cx.simulate_input("ef");
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "ab好d"));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.source().is_empty());
            assert_eq!(editor.selection(), 0..0);
        });
        cx.simulate_keystrokes("secondary-shift-z secondary-shift-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), "ab好def");
            assert_eq!(editor.selection(), 8..8);
        });
    }

    #[gpui::test]
    fn typing_undo_separates_navigation_paste_newlines_and_deletion(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
            cx.write_to_clipboard(ClipboardItem::new_string("XY".into()));
        });
        cx.run_until_parked();
        cx.simulate_input("ab");
        cx.simulate_keystrokes("left right");
        cx.simulate_input("cd");
        cx.simulate_keystrokes("secondary-v");
        cx.simulate_input("ef");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("gh");
        cx.simulate_keystrokes("backspace");
        cx.simulate_input("ij");
        for source in [
            "abcdXYef\ng",
            "abcdXYef\ngh",
            "abcdXYef\n",
            "abcdXYef",
            "abcdXY",
            "abcd",
            "ab",
            "",
        ] {
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
        }
    }

    #[gpui::test]
    fn typing_undo_keeps_ime_composition_separate(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            window.focus(&editor.read(cx).focus);
            cx.background_executor().clone()
        });
        cx.run_until_parked();
        cx.simulate_input("ab");
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.replace_and_mark_text_in_range(None, "n", None, window, cx);
            });
        });
        executor.advance_clock(Duration::from_secs(2));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.replace_and_mark_text_in_range(None, "ni", None, window, cx);
                e.replace_text_in_range(None, "你", window, cx);
                assert_eq!(e.source(), "ab你");
                assert!(!e.is_composing());
            });
        });
        cx.simulate_input("cd");
        for source in ["ab你", "ab", ""] {
            cx.simulate_keystrokes("secondary-z");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
        }
    }

    #[gpui::test]
    fn uses_one_undo_history_for_formatting_and_typing(cx: &mut TestAppContext) {
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new("選取\n\n也選取".into(), window, cx));
        cx.update(|_, cx| bind_keys(cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.set_selection(0, e.source().len());
                e.command(Command::Bold, window, cx);
                assert_eq!(e.source(), "**選取**\n\n**也選取**");
                e.command(Command::Heading, window, cx);
                assert_eq!(e.source(), "**選取**\n\n# **也選取**");
            })
        });
        cx.simulate_input("字");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "**選取**\n\n# 字**也選取**"));
        cx.simulate_keystrokes("secondary-z secondary-z secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "選取\n\n也選取"));
        cx.simulate_keystrokes("secondary-shift-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "**選取**\n\n**也選取**"));
    }

    #[gpui::test]
    fn formatting_across_headings_keeps_both_block_markers(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new("# First\n\n# Second ###\n".into(), window, cx)
        });
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.set_selection("# ".len(), "# First\n\n# Second".len());
                e.command(Command::Bold, window, cx);
                assert_eq!(e.source(), "# **First**\n\n# **Second** ###\n");
                e.command(Command::Bullet, window, cx);
                assert_eq!(e.source(), "# **First**\n\n- **Second**\n");
            })
        });
    }

    #[gpui::test]
    fn invalid_rich_item_keeps_its_source_and_can_be_repaired(cx: &mut TestAppContext) {
        let source = "# Graph\n\n```mermaid\nflowchart LR\nA[Start] -->\n```\n\nParagraph";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        let click_item = |e: &Editor| {
            let layout = e.layout.as_ref().unwrap();
            layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items
                        .iter()
                        .find(|item| {
                            item.target
                                .as_ref()
                                .is_some_and(|section| section.kind == SectionKind::Mermaid)
                        })
                        .map(|item| {
                            (
                                matches!(item.paint, ItemPaint::Graphic(_)),
                                layout.origin
                                    + point(item.left + item.width / 2., row.y + row.height / 2.),
                            )
                        })
                })
                .unwrap()
        };
        let (painted, center) = cx.update(|_, cx| click_item(editor.read(cx)));
        assert!(!painted);
        cx.simulate_click(center, Modifiers::none());
        let header = cx.update(|_, cx| {
            let layout = editor.read(cx).layout.as_ref().unwrap();
            let frame = layout.source_frame.as_ref().unwrap();
            layout.origin + frame.bounds.origin + point(px(16.), px(10.))
        });
        cx.simulate_click(header, Modifiers::none());
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                let section = e.active.clone().unwrap();
                assert_eq!(e.cursor(), section.content.start);
                e.replace(section.content, "flowchart LR\nA[Start] --> B[End]", cx);
            })
        });
        let outside = cx.update(|_, cx| {
            let e = editor.read(cx);
            e.layout
                .as_ref()
                .unwrap()
                .bounds_at(e.source().find("Paragraph").unwrap(), false)
                .unwrap()
                .center()
        });
        cx.simulate_click(outside, Modifiers::none());
        assert!(cx.update(|_, cx| click_item(editor.read(cx)).0));
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).source(),
                source.replace("A[Start] -->", "A[Start] --> B[End]")
            )
        });
    }
}
