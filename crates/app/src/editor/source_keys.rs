use std::{
    ops::Range,
    sync::{Arc, LazyLock},
    time::Duration,
};

use gpui::{Context, KeyDownEvent, actions};
use gpui_editor_codemirror::{
    EditorSelection, Transaction,
    commands::{
        self as core,
        bindings::{CommandId as SourceCommand, default_keymap},
    },
    language::{
        IndentOptions, LanguageData, MarkdownCodeIndent, StreamSettings, language_data_provider,
        markdown::{MarkdownCodeParser, markdown_snapshot},
    },
    view::{
        ANNOUNCE,
        keymap::{KeyEvent, KeyModifiers, KeyPlatform, KeySequence, Keymap},
    },
};

use super::{Editor, source_fence::translate_selection};

actions!(
    editor,
    [
        SelectSourceParent,
        IndentSourceSelection,
        InsertSourceBlankLine,
        ToggleSourceComment
    ]
);

pub(super) enum SourceIndent {
    Selection,
    Newline,
    BlankLine,
}

#[cfg(test)]
mod tests;

static BINDINGS: LazyLock<Keymap<SourceCommand>> = LazyLock::new(|| {
    use SourceCommand::*;
    let bindings = default_keymap().into_iter().filter(|binding| {
        matches!(
            binding.run,
            Some(
                CursorGroupLeft
                    | CursorGroupRight
                    | DeleteGroupBackward
                    | DeleteGroupForward
                    | MoveLineUp
                    | MoveLineDown
                    | CopyLineUp
                    | CopyLineDown
                    | IndentMore
                    | IndentLess
                    | SelectLine
                    | CursorLineBoundaryLeft
                    | CursorLineBoundaryRight
                    | CursorLineBoundaryBackward
                    | CursorLineBoundaryForward
                    | DeleteLineBoundaryBackward
                    | DeleteLineBoundaryForward
            )
        )
    });
    Keymap::new(KeyPlatform::Mac, bindings).expect("source keymap is valid")
});

fn joins_closing_fence_break(source: &str, scope: &Range<usize>, edit: &Transaction) -> bool {
    if !source[scope.end..].starts_with('\n') {
        return false;
    }
    let body = &source[scope.clone()];
    let mut from = 0;
    let mut last = None;
    for (range, insert) in edit.changes().changes() {
        last = insert
            .as_bytes()
            .last()
            .or_else(|| body.as_bytes()[from..range.start].last())
            .or(last);
        from = range.end;
    }
    body.as_bytes()[from..].last().or(last) == Some(&b'\r')
}

impl Editor {
    pub(super) fn type_source_indent(
        &mut self,
        range: &Range<usize>,
        inserted: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.input_enabled
            || self.is_composing()
            || self.has_source_snippet()
            || inserted.chars().count() != 1
            || *range != self.selection()
        {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let Some(parser) = fence.parser else {
            return false;
        };
        let scope = fence.body;
        let host = self.view.state();
        if !matches!(&parser, MarkdownCodeParser::Native(_))
            && let Some(snapshot) = markdown_snapshot(host)
            && let Ok(input) = host.replace_selection(inserted)
        {
            let input = input.with_user_event("input.type").with_scroll_into_view();
            let plan = snapshot.reindent_mounted_input(host, scope.clone(), &parser, input.clone());
            if !matches!(plan, Ok(None)) {
                if let Ok(Some(edit)) = plan {
                    if edit.changes() == input.changes() {
                        return false;
                    }
                    if self.apply_source_transaction(edit, scope, cx) {
                        return true;
                    }
                }
                // Do not retry a rejected combined input as a literal insert.
                self.message = Some("Cannot apply this input".into());
                cx.notify();
                return true;
            }
        }
        let Some(state) = self.state_in_source(&scope) else {
            return false;
        };
        let Ok(input) = state.replace_selection(inserted) else {
            return false;
        };
        let indentation = core::Indentation {
            tab_size: state.tab_size(),
            ..Default::default()
        };
        let settings = StreamSettings {
            tab_size: indentation.tab_size,
            indent_unit: indentation.width(),
        };
        let input = input.with_user_event("input.type");
        let Some(document) = self.source_document(&scope, &parser, settings) else {
            return false;
        };
        let Ok(edit) = document.reindent_input(&state, input.clone(), indentation) else {
            return false;
        };
        edit.changes() != input.changes() && self.apply_local_source_edit(edit, scope, cx)
    }

    pub(super) fn toggle_source_comment(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled || self.is_composing() || self.has_source_snippet() {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let Some(parser) = fence.parser else {
            return false;
        };
        let scope = fence.body;
        let Some(mut state) = self.state_in_source(&scope) else {
            return false;
        };
        let Ok(document) = parser.parse(state.text().clone(), StreamSettings::default()) else {
            return false;
        };
        let lines = if parser.language().uses_projected_lines() {
            document.syntax().projected_lines().cloned()
        } else {
            None
        };
        let extension = if matches!(&parser, MarkdownCodeParser::Wast(_)) {
            let original = self.view.state().clone();
            let body = scope.clone();
            language_data_provider(move |_, position, side| {
                if position > body.len() {
                    return Vec::new();
                }
                original
                    .language_data_at::<core::CommentTokens<'static>>(
                        "commentTokens",
                        body.start + position,
                        side,
                    )
                    .into_iter()
                    .next()
                    .map(|tokens| Arc::new(LanguageData::default().with("commentTokens", tokens)))
                    .into_iter()
                    .collect()
            })
        } else {
            Arc::new(document).language_data_extension()
        };
        if state
            .update(Transaction::default().with_effects([extension.append()]))
            .is_err()
        {
            return false;
        }
        let mode = core::CommentMode::Toggle;
        let style = core::CommentStyle::PreferLine;
        let edit = match &lines {
            Some(lines) => core::comment_with_language_data_in_lines(&state, mode, style, lines),
            None => core::comment_with_language_data(&state, mode, style),
        };
        let Ok(Some(edit)) = edit else {
            return false;
        };
        self.apply_local_source_edit(edit, scope, cx)
    }

    pub(super) fn indent_source(&mut self, command: SourceIndent, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled
            || self.view.state().read_only()
            || self.is_composing()
            || self.has_source_snippet()
        {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let Some(parser) = fence.parser else {
            return false;
        };
        let scope = fence.body;
        let Some(state) = self.state_in_source(&scope) else {
            return false;
        };
        let host =
            (!matches!(&parser, MarkdownCodeParser::Native(_))).then(|| self.view.state().clone());
        let snapshot = host.as_ref().and_then(markdown_snapshot);
        let mounted = host
            .as_ref()
            .zip(snapshot.as_ref())
            .and_then(|(host, snapshot)| {
                MarkdownCodeIndent::for_parser(host, snapshot, scope.clone(), &parser)
            });
        let indent = host.as_ref().filter(|_| mounted.is_some()).map_or_else(
            || core::Indentation {
                tab_size: state.tab_size(),
                ..Default::default()
            },
            core::Indentation::for_state,
        );
        let settings = StreamSettings {
            tab_size: indent.tab_size,
            indent_unit: indent.width(),
        };
        let after = &self.source()[scope.end..];
        let ending = if after.starts_with("\r\n")
            || (!after.starts_with('\n') && self.source()[..scope.start].ends_with("\r\n"))
        {
            "\r\n"
        } else {
            "\n"
        };
        let syntax = if let Some(mounted) = &mounted {
            mounted.syntax()
        } else {
            let Some(document) = self.source_document(&scope, &parser, settings) else {
                return false;
            };
            document.syntax()
        };
        let lines = parser
            .language()
            .uses_projected_lines()
            .then(|| syntax.projected_lines())
            .flatten();
        let indentation_at = |position, settings, options: IndentOptions<'_>| match &mounted {
            Some(mounted) => mounted.at(position, settings, options),
            None => syntax.indentation(position, settings, options),
        };
        let bracket_gap = |position| syntax.bracket_gap_for_indent(position);
        let transaction = match (command, lines) {
            (SourceIndent::Selection, Some(lines)) => {
                core::indent_selection_with_projection(&state, lines, indent, indentation_at)
            }
            (SourceIndent::Newline, Some(lines)) => {
                core::insert_newline_and_indent_with_projection(
                    &state,
                    lines,
                    indent,
                    indentation_at,
                    bracket_gap,
                    ending,
                )
            }
            (SourceIndent::BlankLine, Some(lines)) => core::insert_blank_line_with_projection(
                &state,
                lines,
                indent,
                indentation_at,
                ending,
            ),
            (SourceIndent::Selection, None) => {
                core::indent_selection(&state, indent, indentation_at)
            }
            (SourceIndent::Newline, None) => core::insert_newline_and_indent_with_ending(
                &state,
                indent,
                indentation_at,
                bracket_gap,
                ending,
            ),
            (SourceIndent::BlankLine, None) => {
                core::insert_blank_line(&state, indent, indentation_at, ending)
            }
        };
        let Ok(Some(transaction)) = transaction else {
            // Do not replace a rejected projected indent with a raw newline.
            return lines.is_some() || mounted.is_some();
        };
        if lines.is_some() && joins_closing_fence_break(self.source(), &scope, &transaction) {
            return true;
        }
        transaction.changes().is_empty()
            || self.apply_local_source_edit(transaction, scope, cx)
            || mounted.is_some()
    }

    pub(super) fn select_source_parent(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled || self.is_composing() || self.has_source_snippet() {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let Some(parser) = fence.parser else {
            return false;
        };
        let scope = fence.body;
        let current = self.view.state().selection().clone();
        if current.ranges().iter().any(|range| {
            let range = range.range();
            range.start < scope.start || range.end > scope.end
        }) {
            return false;
        }
        let settings = StreamSettings {
            tab_size: self.view.state().tab_size(),
            ..StreamSettings::default()
        };
        let Some(doc) = self.source_document(&scope, &parser, settings) else {
            return false;
        };
        let ranges = current
            .ranges()
            .iter()
            .map(|&range| {
                let span = range.range();
                doc.parent_selection(span.start - scope.start..span.end - scope.start)
                    .map_or(range, |parent| {
                        EditorSelection::undirectional_range(
                            parent.start + scope.start,
                            parent.end + scope.start,
                        )
                    })
            })
            .collect();
        let Ok(expanded) = EditorSelection::new(ranges, current.main_index()) else {
            return false;
        };
        if expanded == current {
            return false;
        }
        for range in expanded.ranges() {
            self.reveal_folded_range(&range.range(), cx);
        }
        self.record_selection_transaction(
            Transaction::default()
                .with_selection(expanded)
                .with_user_event("select.parentSyntax"),
            self.active.clone(),
            cx,
        );
        self.preferred_x = None;
        self.upstream = false;
        self.reveal_caret = true;
        cx.notify();
        true
    }

    pub(super) fn source_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.input_enabled || self.is_composing() || self.active.is_none() {
            return;
        }
        let keystroke = &event.keystroke;
        let name = match keystroke.key.as_str() {
            "left" => "ArrowLeft",
            "right" => "ArrowRight",
            "up" => "ArrowUp",
            "down" => "ArrowDown",
            "backspace" => "Backspace",
            "delete" => "Delete",
            "home" => "Home",
            "end" => "End",
            "l" | "[" | "]" => keystroke.key.as_str(),
            _ => return,
        };
        let modifiers = &keystroke.modifiers;
        let event = KeyEvent::new(
            name,
            KeyModifiers {
                alt: modifiers.alt,
                control: modifiers.control,
                meta: modifiers.platform,
                shift: modifiers.shift,
            },
        );
        let result = BINDINGS.dispatch(
            "editor",
            event,
            Duration::ZERO,
            &mut KeySequence::default(),
            |command| self.run_source_command(*command, cx),
        );
        if result.handled {
            cx.stop_propagation();
        }
    }

    fn run_source_command(&mut self, command: SourceCommand, cx: &mut Context<Self>) -> bool {
        use SourceCommand::*;
        match command {
            SelectLine => return self.select_source_lines(cx),
            CursorLineBoundaryLeft | CursorLineBoundaryBackward => {
                return self.move_to_source_row_edge(false, false, cx);
            }
            CursorLineBoundaryRight | CursorLineBoundaryForward => {
                return self.move_to_source_row_edge(true, false, cx);
            }
            SelectLineBoundaryLeft | SelectLineBoundaryBackward => {
                return self.move_to_source_row_edge(false, true, cx);
            }
            SelectLineBoundaryRight | SelectLineBoundaryForward => {
                return self.move_to_source_row_edge(true, true, cx);
            }
            DeleteLineBoundaryBackward => return self.delete_to_source_row_edge(false, cx),
            DeleteLineBoundaryForward => return self.delete_to_source_row_edge(true, cx),
            MoveLineUp | MoveLineDown | CopyLineUp | CopyLineDown | IndentMore | IndentLess => {
                return self.edit_source_lines(command, cx);
            }
            _ => {}
        }
        let deletion = matches!(command, DeleteGroupBackward | DeleteGroupForward);
        if deletion && self.unfold_at_deletion_edge(command == DeleteGroupForward, cx) {
            return true;
        }
        if !deletion
            && self.move_in_source_groups(
                matches!(command, CursorGroupRight | SelectGroupRight),
                matches!(command, SelectGroupLeft | SelectGroupRight),
                cx,
            )
        {
            return true;
        }
        let section = self.active.as_ref().expect("source key requires a section");
        let bounds = section.content.clone();
        let run = match command {
            CursorGroupLeft => core::cursor_group_backward,
            CursorGroupRight => core::cursor_group_forward,
            SelectGroupLeft => core::select_group_backward,
            SelectGroupRight => core::select_group_forward,
            DeleteGroupBackward => core::delete_group_backward,
            DeleteGroupForward => core::delete_group_forward,
            _ => return false,
        };
        let Ok(Some(transaction)) = run(&self.state_with_source_words(true)) else {
            return false;
        };
        if deletion {
            let mut changes = transaction.changes().changes();
            if let Some((range, text)) = changes.next()
                && changes.next().is_none()
                && text.is_empty()
            {
                let range = range.start.max(bounds.start)..range.end.min(bounds.end);
                if range.start < range.end {
                    if let Some(notice) = transaction
                        .effects()
                        .iter()
                        .find(|effect| effect.get(&ANNOUNCE).is_some())
                    {
                        return self
                            .replace_with_effect(range, text, notice.clone(), cx)
                            .is_some();
                    }
                    return self.replace(range, text, cx);
                }
            }
            return false;
        }
        let Some(selection) = transaction.selection() else {
            return false;
        };
        let head = selection.main().head().clamp(bounds.start, bounds.end);
        self.move_to(
            head,
            matches!(command, SelectGroupLeft | SelectGroupRight),
            cx,
        );
        self.reveal_caret = true;
        true
    }

    fn select_source_lines(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let scope = fence.body;
        let lines = self.source_body_line_projection();
        let Some(state) = self.state_in_source_with_lines(&scope, lines) else {
            return false;
        };
        let Ok(Some(transaction)) = core::select_line(&state) else {
            return false;
        };
        let Some(selection) = transaction.selection().and_then(|selection| {
            translate_selection(selection, |offset| Some(scope.start + offset))
        }) else {
            return false;
        };
        self.record_selection_transaction(
            transaction.with_selection(selection),
            self.active.clone(),
            cx,
        );
        self.upstream = false;
        self.preferred_x = None;
        cx.notify();
        true
    }

    fn edit_source_lines(&mut self, command: SourceCommand, cx: &mut Context<Self>) -> bool {
        if self.view.state().read_only() {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let scope = fence.body;
        use SourceCommand::*;
        let mut lines = self.source_body_line_projection();
        let state = if matches!(command, IndentMore | IndentLess) {
            self.state_in_source_with_lines(&scope, lines.take())
        } else {
            self.state_in_source(&scope)
        };
        let Some(state) = state else {
            return false;
        };
        let edit = match (command, lines.as_ref()) {
            (MoveLineUp | MoveLineDown, Some(lines)) => {
                core::move_line_with_projection(&state, lines, command == MoveLineDown)
            }
            (CopyLineUp | CopyLineDown, Some(lines)) => {
                core::copy_line_with_projection(&state, lines, command == CopyLineDown)
            }
            (MoveLineUp, None) => core::move_line_up(&state),
            (MoveLineDown, None) => core::move_line_down(&state),
            (CopyLineUp | CopyLineDown, None)
                if !state.text().as_str().contains('\n')
                    && self.source()[..scope.start].ends_with("\r\n") =>
            {
                core::copy_line_with_ending(&state, command == CopyLineDown, "\r\n")
            }
            (CopyLineUp, None) => core::copy_line_up(&state),
            (CopyLineDown, None) => core::copy_line_down(&state),
            (IndentMore, _) => {
                core::indent_more_with(&state, core::Indentation::for_state(self.view.state()))
            }
            (IndentLess, _) => {
                core::indent_less_with(&state, core::Indentation::for_state(self.view.state()))
            }
            _ => unreachable!(),
        };
        let Ok(Some(transaction)) = edit else {
            return false;
        };
        // The raw separator before the closing fence is outside the body.
        if lines.is_some() && joins_closing_fence_break(self.source(), &scope, &transaction) {
            return false;
        }
        transaction.changes().is_empty() || self.apply_local_source_edit(transaction, scope, cx)
    }
}
