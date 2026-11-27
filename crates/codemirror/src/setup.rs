//! Native composition of `codemirror` 6.0.2 presets.
//! See `docs/licenses/codemirror-setup.txt`.
//!
//! The presets share the pinned fallback style and native history. The host
//! draws projections and supplies geometry, language snapshots, and completion
//! sources. Without geometry, existing shortcuts keep their logical behavior.

use std::time::Duration;

use crate::{
    ALLOW_MULTIPLE_SELECTIONS, Change, ChangeSet, EditError, EditorState, Extension, RangeEdit,
    SelectionRange, Transaction,
    autocomplete::{
        AutocompleteCommand,
        brackets::{close_brackets, close_brackets_enabled, delete_bracket_pair, insert_bracket},
        session::{CompletionSession, CompletionStep},
    },
    commands::{
        Indentation,
        bindings::{CommandId, TabFocusMode, toggle_tab_focus_mode},
        visual::{VisualGeometry, VisualSelection, VisualSyntax},
    },
    language::{
        DEFAULT_HIGHLIGHT_STYLE, HighlightLayer,
        brackets::{BracketMatch, bracket_matching, state_bracket_matches},
        fold::{FoldCommand, code_folding, syntax_fallback},
        line_projection::{command_line_projection, editor_line_extension},
    },
    language_data::{NativeLanguageSupport, NativeSyntax},
    lint::LintCommand,
    search::{SearchActionResult, SearchPanelInput, selection_match_extension},
    transaction_filter,
    view::{
        ViewHighlight,
        keymap::{KeyEvent, KeyPlatform, KeyResult, KeySequence, Keymap},
        selection_layers::{
            CrosshairCursor, crosshair_cursor, draw_selection, drop_cursor, rectangular_selection,
        },
        special_char_extension,
    },
};

pub use crate::view::EditorView;

mod bindings;
mod editor_commands;
mod render;
pub use bindings::{SetupCommand, basic_setup_keymap, minimal_setup_keymap};
pub use render::SetupDecorations;

/// A portable editor with the preset's native behavior.
pub struct SetupEditor<M> {
    view: EditorView<M>,
    platform: KeyPlatform,
    keyboard: Keymap<SetupCommand>,
    sequence: KeySequence,
    tab_focus: TabFocusMode,
    crosshair: Option<CrosshairCursor>,
    basic: bool,
}

/// The caller's geometry for the current editor snapshot.
pub struct SetupGeometry<'a> {
    pub geometry: &'a dyn VisualGeometry,
    pub selection: Option<&'a VisualSelection>,
}

/// The caller owns completion sources, timers, and the visible option count.
/// Forward accepted state changes to the session before passing it here.
pub struct SetupCompletion<'a> {
    pub session: &'a mut CompletionSession,
    pub visible_options: usize,
}

/// Optional services and input scope for one shortcut.
pub struct SetupKeyContext<'a> {
    pub scope: &'a str,
    pub search_panel_input: SearchPanelInput,
    /// Current parser snapshot for folds and indentation. It overrides attached syntax.
    pub syntax: Option<&'a NativeSyntax>,
    /// Current tree and optional grammar bracket data for syntax motion.
    pub visual_syntax: Option<&'a dyn VisualSyntax>,
    pub geometry: Option<SetupGeometry<'a>>,
    pub completion: Option<SetupCompletion<'a>>,
}

impl Default for SetupKeyContext<'_> {
    fn default() -> Self {
        Self {
            scope: "editor",
            search_panel_input: SearchPanelInput::Missing,
            syntax: None,
            visual_syntax: None,
            geometry: None,
            completion: None,
        }
    }
}

/// The host action requested by an accepted shortcut.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupHostAction {
    FocusSearchPanel,
    OpenGotoLine,
    FocusLintPanel,
}

pub struct SetupKeyResult<M> {
    pub key: KeyResult,
    pub metadata: Option<M>,
    pub host_action: Option<SetupHostAction>,
    /// Retain this for the next visual movement. Reset it after unrelated
    /// changes to the document or selection.
    pub visual_selection: Option<VisualSelection>,
}

/// Construct the minimal preset without changing the caller's selection policy.
/// If the caller's configuration rejects adding the preset's view extensions,
/// retain the original state. Use [`try_minimal_setup`] to report that error.
pub fn minimal_setup<M: Clone>(mut state: EditorState, platform: KeyPlatform) -> SetupEditor<M> {
    let _ = install(&mut state, minimal_view_extensions());
    SetupEditor::new(state, platform, false)
}

pub fn try_minimal_setup<M: Clone>(
    mut state: EditorState,
    platform: KeyPlatform,
) -> Result<SetupEditor<M>, EditError> {
    install(&mut state, minimal_view_extensions())?;
    Ok(SetupEditor::new(state, platform, false))
}

fn minimal_view_extensions() -> Extension {
    Extension::group([
        editor_line_extension(),
        draw_selection(Default::default()),
        special_char_extension(Default::default()),
    ])
}

pub fn basic_setup<M: Clone>(
    state: EditorState,
    platform: KeyPlatform,
) -> Result<SetupEditor<M>, EditError> {
    basic(state, platform, None)
}

/// Also install the selected native language's reindent-on-input filter.
/// The host can supply current parsed syntax for shortcuts and fold markers.
pub fn basic_setup_with_language<M: Clone>(
    state: EditorState,
    platform: KeyPlatform,
    language: NativeLanguageSupport,
) -> Result<SetupEditor<M>, EditError> {
    let indent_on_input = transaction_filter(move |state, transaction| {
        language.reindent_input(state, transaction, Indentation::for_state(state))
    });
    basic(state, platform, Some(indent_on_input))
}

fn basic<M: Clone>(
    mut state: EditorState,
    platform: KeyPlatform,
    indent_on_input: Option<Extension>,
) -> Result<SetupEditor<M>, EditError> {
    let extensions = Extension::group(
        render::gutter_extensions()
            .into_iter()
            .chain([
                code_folding(),
                minimal_view_extensions(),
                ViewHighlight::ActiveLine.extension(),
                bracket_matching(Default::default()),
                selection_match_extension(Default::default()),
                drop_cursor(),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
            ])
            .chain(indent_on_input)
            .chain([close_brackets(), rectangular_selection(Default::default())]),
    );
    install(&mut state, extensions)?;
    Ok(SetupEditor::new(state, platform, true))
}

fn install(state: &mut EditorState, extension: Extension) -> Result<(), EditError> {
    state.update(
        Transaction::default()
            .with_effects([extension.append()])
            .without_filtering(),
    )
}

impl<M: Clone> SetupEditor<M> {
    fn new(state: EditorState, platform: KeyPlatform, basic: bool) -> Self {
        let bindings = if basic {
            basic_setup_keymap()
        } else {
            minimal_setup_keymap()
        };
        Self {
            view: EditorView::new(state),
            platform,
            keyboard: Keymap::new(platform, bindings).expect("preset bindings are valid"),
            sequence: KeySequence::default(),
            tab_focus: TabFocusMode::default(),
            crosshair: basic.then(|| crosshair_cursor(Default::default())),
            basic,
        }
    }

    pub fn view(&self) -> &EditorView<M> {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut EditorView<M> {
        &mut self.view
    }

    /// Pass this layer to the renderer along with any caller-provided styles.
    pub fn highlight_fallback(&self) -> HighlightLayer<'static> {
        HighlightLayer::Fallback(DEFAULT_HIGHLIGHT_STYLE)
    }

    pub fn clear_key_sequence(&mut self) {
        self.sequence.clear();
    }

    /// Current Tab focus policy. The host performs actual focus traversal.
    pub fn tab_focus_mode(&self) -> TabFocusMode {
        self.tab_focus
    }

    /// The basic preset's Alt crosshair. The host forwards modifier and
    /// pointer events and changes the pointer icon when the policy changes.
    pub fn crosshair_cursor_mut(&mut self) -> Option<&mut CrosshairCursor> {
        self.crosshair.as_mut()
    }

    /// Dispatch a shortcut in the editor scope without host geometry.
    pub fn key_event(
        &mut self,
        event: KeyEvent<'_>,
        now: Duration,
        metadata: M,
    ) -> Result<(KeyResult, Option<M>), EditError> {
        let result = self.key_event_with(event, now, metadata, SetupKeyContext::default())?;
        Ok((result.key, result.metadata))
    }

    /// Dispatch a shortcut with the supplied host services. State edits and
    /// history replay return metadata. Some keys request a host action instead.
    pub fn key_event_with(
        &mut self,
        event: KeyEvent<'_>,
        now: Duration,
        metadata: M,
        mut context: SetupKeyContext<'_>,
    ) -> Result<SetupKeyResult<M>, EditError> {
        let mut output = SetupKeyResult {
            key: KeyResult::default(),
            metadata: None,
            host_action: None,
            visual_selection: None,
        };
        if context.scope == "editor" && self.tab_focus.bypass_key_bindings(event, now) {
            return Ok(output);
        }
        let view = &mut self.view;
        let tab_focus = &mut self.tab_focus;
        let platform = self.platform;
        let scope = context.scope;
        let mut error = None;
        output.key = self
            .keyboard
            .dispatch(scope, event, now, &mut self.sequence, |command| {
                let result = dispatch(
                    *command,
                    view,
                    KeyDispatch {
                        tab_focus,
                        platform,
                        context: &mut context,
                        now,
                        metadata: &metadata,
                        output: &mut output,
                    },
                );
                match result {
                    Ok(handled) => handled,
                    Err(err) => {
                        error = Some(err);
                        true
                    }
                }
            });
        if scope == "editor" && !output.key.handled {
            self.tab_focus.handle_unhandled_key(event, now);
        }
        error.map_or(Ok(output), Err)
    }

    /// Handle ordinary typed text. Do not use this for uncommitted IME text.
    /// The basic preset supports automatic pairs. Both presets record history.
    pub fn insert_text(
        &mut self,
        text: &str,
        now: Duration,
        metadata: M,
    ) -> Result<bool, EditError> {
        let state = self.view.state();
        if state.read_only() {
            return Ok(false);
        }
        let paired = if self.basic && text.chars().count() == 1 && close_brackets_enabled(state)? {
            insert_bracket(state, text)?
        } else {
            None
        };
        let transaction = match paired {
            Some(transaction) => transaction,
            None => state
                .change_by_range(|selection| {
                    let range = selection.range();
                    let cursor = range.start + text.len();
                    Ok(RangeEdit {
                        changes: ChangeSet::single(Change::new(range, text))?,
                        selection: SelectionRange::new(cursor, cursor),
                        effects: Vec::new(),
                    })
                })?
                .with_user_event("input.type")
                .with_scroll_into_view(),
        };
        self.view.update(
            transaction.with_time(milliseconds(now)),
            metadata.clone(),
            |_| (None, metadata),
        )?;
        Ok(true)
    }

    /// Match brackets with the installed options and the active attached syntax
    /// when supported. This query does not call the configured renderer.
    pub fn bracket_matches(&self) -> Vec<BracketMatch> {
        state_bracket_matches(self.view.state()).expect("valid bracket matching configuration")
    }
}

struct KeyDispatch<'a, 'host, M> {
    tab_focus: &'a mut TabFocusMode,
    platform: KeyPlatform,
    context: &'a mut SetupKeyContext<'host>,
    now: Duration,
    metadata: &'a M,
    output: &'a mut SetupKeyResult<M>,
}

fn dispatch<M: Clone>(
    command: SetupCommand,
    view: &mut EditorView<M>,
    input: KeyDispatch<'_, '_, M>,
) -> Result<bool, EditError> {
    use CommandId::{Redo, RedoSelection, ToggleTabFocusMode, Undo, UndoSelection};
    use SetupCommand::*;

    let KeyDispatch {
        tab_focus,
        platform,
        context,
        now,
        metadata,
        output,
    } = input;
    let state = view.state();
    let syntax = current_syntax(state, context.syntax);
    let mut visual_selection = None;
    let transaction = match command {
        Editor(Undo | Redo) => {
            if state.read_only() {
                return Ok(false);
            }
            output.metadata = if command == Editor(Undo) {
                view.undo()
            } else {
                view.redo()
            };
            return Ok(output.metadata.is_some());
        }
        Editor(UndoSelection | RedoSelection) => {
            output.metadata = if command == Editor(UndoSelection) {
                view.undo_selection(metadata.clone())
            } else {
                view.redo_selection(metadata.clone())
            };
            return Ok(output.metadata.is_some());
        }
        Editor(ToggleTabFocusMode) => return Ok(toggle_tab_focus_mode(tab_focus)),
        Editor(command) => editor_commands::transaction(
            command,
            state,
            platform,
            syntax,
            context.visual_syntax,
            context.geometry.as_ref(),
            &mut visual_selection,
        )?,
        Search(action) => match action.run(state, context.search_panel_input) {
            SearchActionResult::Unhandled => None,
            SearchActionResult::Update(transaction) => Some(transaction),
            SearchActionResult::OpenSearchPanel(transaction) => {
                output.host_action = Some(SetupHostAction::FocusSearchPanel);
                transaction
            }
            SearchActionResult::OpenGotoLine => {
                output.host_action = Some(SetupHostAction::OpenGotoLine);
                None
            }
        },
        Completion(AutocompleteCommand::DeleteBracketPair) => {
            if state.read_only() || !close_brackets_enabled(state)? {
                None
            } else {
                delete_bracket_pair(state)?
            }
        }
        Completion(command) => {
            use AutocompleteCommand::*;
            let Some(completion) = &mut context.completion else {
                return Ok(false);
            };
            let session = &mut completion.session;
            if session.state().text() != state.text()
                || session.state().selection() != state.selection()
            {
                return Ok(false);
            }
            match command {
                StartCompletion => return Ok(!state.read_only() && session.start_completion(now)),
                CloseCompletion => return Ok(session.close_completion()),
                NextCompletion | PreviousCompletion => {
                    return session.move_selection(
                        command == NextCompletion,
                        CompletionStep::Option,
                        now,
                    );
                }
                NextCompletionPage | PreviousCompletionPage => {
                    return session.move_selection(
                        command == NextCompletionPage,
                        CompletionStep::Page(completion.visible_options),
                        now,
                    );
                }
                AcceptCompletion if !state.read_only() => session.accept_selected(now)?,
                _ => None,
            }
        }
        Fold | Unfold | FoldAll | UnfoldAll => {
            let fold = match command {
                Fold => FoldCommand::FoldCode,
                Unfold => FoldCommand::UnfoldCode,
                FoldAll => FoldCommand::FoldAll,
                UnfoldAll => FoldCommand::UnfoldAll,
                _ => unreachable!(),
            };
            let projection = command_line_projection(state);
            let lines = projection.as_ref().unwrap_or(state.text());
            fold.run_on_lines(state, lines, syntax_fallback(syntax))
        }
        Lint(command) => {
            if command == LintCommand::OpenPanel {
                output.host_action = Some(SetupHostAction::FocusLintPanel);
            }
            command.transaction(state)
        }
    };
    if let Some(transaction) = transaction {
        let visual = visual_selection.zip(transaction.selection().cloned());
        let old_text = visual.as_ref().map(|_| state.text().clone());
        output.metadata = Some(view.update(
            transaction.with_time(milliseconds(now)),
            metadata.clone(),
            |state| {
                if let Some((visual, proposed)) = visual
                    && old_text.as_ref() == Some(state.text())
                    && state.selection().eq_with_assoc(&proposed)
                {
                    output.visual_selection = Some(visual);
                }
                (None, metadata.clone())
            },
        )?);
    }
    Ok(output.metadata.is_some() || output.host_action.is_some())
}

fn current_syntax<'a>(
    state: &EditorState,
    syntax: Option<&'a NativeSyntax>,
) -> Option<&'a NativeSyntax> {
    syntax.filter(|syntax| syntax.text() == state.text())
}

fn milliseconds(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}
