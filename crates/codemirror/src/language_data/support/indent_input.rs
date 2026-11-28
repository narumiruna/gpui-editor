use std::ops::Range;

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorSelection, EditorState, Extension, Text,
    Transaction, TransactionCoordinates,
    commands::Indentation,
    language::{
        IndentOptions, MarkdownCodeIndent, StreamSettings, get_indentation, json, lezer_grammar,
        line_projection::{editor_lines, keeps_separate_breaks, splits_crlf},
        markdown::{MarkdownCodeParser, markdown_snapshot},
    },
    transaction_filter,
};

use super::{NativeLanguageSupport, NativeSyntax, SupportedDocument, attached_language_snapshot};

impl NativeLanguageSupport {
    /// Reindent typed or completed input in this native language. The filter
    /// uses the starting language data and the resulting document's syntax.
    /// An input and its indentation changes remain one transaction.
    pub fn indent_on_input(self, indentation: Indentation) -> Extension {
        transaction_filter(move |state, transaction| {
            self.reindent_input(state, transaction, indentation)
        })
    }

    /// Apply the same indentation rule without installing a state filter.
    pub fn reindent_input(
        self,
        state: &EditorState,
        transaction: Transaction,
        indentation: Indentation,
    ) -> Result<Transaction, EditError> {
        if !is_input(&transaction) {
            return Ok(transaction);
        }
        let old_text = state.text();
        transaction.changes().check(old_text)?;
        let settings = StreamSettings {
            tab_size: indentation.tab_size,
            indent_unit: indentation.width(),
        };
        let Ok(before) = self.parse_with_code_languages(old_text.clone(), settings, &[]) else {
            return Ok(transaction);
        };
        before.reindent_input(state, transaction, indentation)
    }
}

impl SupportedDocument {
    /// Reindent input using this syntax snapshot for positional language data.
    /// The snapshot must match the state before the transaction.
    pub fn reindent_input(
        &self,
        state: &EditorState,
        transaction: Transaction,
        indentation: Indentation,
    ) -> Result<Transaction, EditError> {
        let old_text = state.text();
        if !is_input(&transaction) || self.text() != old_text {
            return Ok(transaction);
        }
        transaction.changes().check(old_text)?;
        let position = state.selection().main().head();
        let attached_lr = matches!(
            self.syntax,
            NativeSyntax::Java(_)
                | NativeSyntax::Json(_)
                | NativeSyntax::LezerGrammar(_)
                | NativeSyntax::Rust(_)
        ) && attached_language_snapshot(state)
            .is_some_and(|snapshot| std::ptr::eq(snapshot.as_ref(), self));
        let mut rules =
            state.language_data_at::<fn(&str) -> bool>("indentOnInput", position, Assoc::Before);
        if !attached_lr {
            rules.extend(
                self.data_at(position, Assoc::Before)
                    .iter()
                    .filter_map(|entry| entry.get::<fn(&str) -> bool>("indentOnInput").copied()),
            );
        }
        if rules.is_empty() {
            return Ok(transaction);
        }
        let selection = transaction
            .selection()
            .cloned()
            .unwrap_or_else(|| state.selection().map(transaction.changes()));
        let mut text = old_text.clone();
        transaction.changes().apply(&mut text);
        selection.check(|position| text.is_char_boundary(position))?;
        // These syntax services treat lone CR as a line break. Use the same
        // line bounds for typed input without changing edit bytes.
        let editor_text = self
            .language
            .uses_projected_lines()
            .then(|| editor_lines(text.as_str()))
            .flatten();
        let lines = editor_text.as_ref().unwrap_or(&text);
        if !triggers_indentation(lines, selection.main().head(), rules)? {
            return Ok(transaction);
        }
        let use_provisional = attached_lr
            && (self.has_configured_indent_rules()
                || state.has_indent_services()?
                || !matches!(
                    state.extensions.prepare_reconfiguration(&transaction),
                    Ok(None)
                ));
        let changes = if use_provisional {
            // Read the input state before adding indentation. Do not rerun
            // filters or extenders, or parse a separate stock syntax tree.
            let mut provisional = state.clone();
            if provisional.apply_resolved(transaction.clone()).is_err() {
                // Leave validation to the final transaction. A later filter
                // can still replace an invalid candidate.
                return Ok(transaction);
            }
            let changes = indentation_changes(
                lines,
                provisional.selection(),
                Indentation::for_state(&provisional),
                |start| get_indentation(&provisional, start, IndentOptions::default(), None),
            )?;
            if editor_text.is_some() && !keeps_separate_breaks(text.as_str(), &changes) {
                return Ok(transaction);
            }
            changes
        } else {
            let settings = StreamSettings {
                tab_size: indentation.tab_size,
                indent_unit: indentation.width(),
            };
            let parse_current = self.strict_parser.is_some()
                || matches!(self.syntax, NativeSyntax::Wast(_))
                || (attached_lr && matches!(self.syntax, NativeSyntax::Java(_)));
            let parsed = if parse_current {
                let Ok(parsed) = self.reparse(transaction.changes()) else {
                    return Ok(transaction);
                };
                parsed
            } else if let Some(parser) = &self.markdown_parser {
                let Ok(parsed) = Self::parse_markdown(text.clone(), settings, parser.clone())
                else {
                    // Leave the transaction to the installed parser for validation.
                    return Ok(transaction);
                };
                parsed
            } else {
                let Ok(parsed) =
                    self.language
                        .support()
                        .parse_with_code_languages(text.clone(), settings, &[])
                else {
                    return Ok(transaction);
                };
                parsed
            };
            indentation_changes(lines, &selection, indentation, |start| {
                Ok(parsed
                    .syntax()
                    .indentation(start, settings, IndentOptions::default()))
            })?
        };
        if changes.is_empty() {
            return Ok(transaction);
        }
        transaction.merge(
            Transaction::new(changes),
            old_text.len(),
            TransactionCoordinates::Sequential,
        )
    }

    /// Reindent input in a verified contiguous Markdown code body. The callback
    /// sees the provisional parent state and host byte positions. Return `None`
    /// if this snapshot, parser, and body do not identify the same mount.
    /// The host must still validate and dispatch the returned transaction.
    pub fn reindent_mounted_input(
        &self,
        state: &EditorState,
        body: Range<usize>,
        parser: &MarkdownCodeParser,
        input: Transaction,
    ) -> Result<Option<Transaction>, EditError> {
        if MarkdownCodeIndent::for_parser(state, self, body.clone(), parser).is_none() {
            return Ok(None);
        }
        if !is_input(&input) {
            return Ok(Some(input));
        }
        let old_text = state.text();
        let source = old_text.as_str();
        input.changes().check(old_text)?;
        if input.changes().changes().any(|(range, _)| {
            range.start < body.start
                || range.end > body.end
                || splits_crlf(source, range.start)
                || splits_crlf(source, range.end)
        }) {
            return Err(EditError::InvalidRange);
        }
        let rules = state.language_data_at::<fn(&str) -> bool>(
            "indentOnInput",
            state.selection().main().head(),
            Assoc::Before,
        );
        if !keeps_separate_breaks(source, input.changes()) {
            return Err(EditError::InvalidRange);
        }
        if rules.is_empty() {
            return Ok(Some(input));
        }
        let head = input.selection().map_or_else(
            || state.selection().map(input.changes()).main().head(),
            |selection| selection.main().head(),
        );
        let mapped = input.changes().map_pos(body.start, Assoc::Before)
            ..input.changes().map_pos(body.end, Assoc::After);
        let mut text = old_text.clone();
        input.changes().apply(&mut text);
        let projection = editor_lines(text.as_str());
        let lines = projection.as_ref().unwrap_or(&text);
        if !mapped.contains(&head) && head != mapped.end
            || !triggers_indentation(lines, head, rules)?
        {
            return Ok(Some(input));
        }
        let mut provisional = state.clone();
        if provisional.apply_resolved(input.clone()).is_err() {
            return Ok(Some(input));
        }
        let Some(snapshot) = markdown_snapshot(&provisional) else {
            return Ok(Some(input));
        };
        let Some(mount) =
            MarkdownCodeIndent::for_parser(&provisional, &snapshot, mapped.clone(), parser)
        else {
            return Ok(Some(input));
        };
        let indentation = Indentation::for_state(&provisional);
        let settings = StreamSettings {
            tab_size: indentation.tab_size,
            indent_unit: indentation.width(),
        };
        let changes = indentation_changes(lines, provisional.selection(), indentation, |start| {
            if !mapped.contains(&start) {
                return Ok(None);
            }
            Ok(mount.at(start - mapped.start, settings, IndentOptions::default()))
        })?;
        if changes.is_empty() || !keeps_separate_breaks(text.as_str(), &changes) {
            return Ok(Some(input));
        }
        if changes.changes().any(|(range, _)| range.end > mapped.end) {
            return Err(EditError::InvalidRange);
        }
        input
            .merge(
                Transaction::new(changes),
                old_text.len(),
                TransactionCoordinates::Sequential,
            )
            .map(Some)
    }

    fn has_configured_indent_rules(&self) -> bool {
        match &self.syntax {
            NativeSyntax::Json(doc) => !doc.node_types().same_set(json::stock_node_types()),
            NativeSyntax::LezerGrammar(doc) => {
                !doc.node_types().same_set(lezer_grammar::stock_node_types())
            }
            NativeSyntax::Rust(doc) => doc.has_configured_indentation(),
            _ => false,
        }
    }
}

fn triggers_indentation(
    lines: &Text,
    head: usize,
    rules: Vec<fn(&str) -> bool>,
) -> Result<bool, EditError> {
    let line = lines.line_at(head).ok_or(EditError::InvalidSelection)?;
    let prefix = lines
        .slice(line.range.start..head)
        .ok_or(EditError::InvalidSelection)?;
    Ok(
        prefix.encode_utf16().take(201).count() <= 200
            && rules.into_iter().any(|rule| rule(prefix)),
    )
}

fn indentation_changes(
    lines: &Text,
    selection: &EditorSelection,
    indentation: Indentation,
    mut column_at: impl FnMut(usize) -> Result<Option<usize>, EditError>,
) -> Result<ChangeSet, EditError> {
    let mut previous = None;
    let mut changes = Vec::new();
    for selection in selection.ranges() {
        let line = lines
            .line_at(selection.head())
            .ok_or(EditError::InvalidSelection)?;
        if previous == Some(line.range.start) {
            continue;
        }
        previous = Some(line.range.start);
        let Some(columns) = column_at(line.range.start)? else {
            continue;
        };
        let current = line.text.trim_start_matches(|ch: char| {
            ch == '\u{feff}' || (ch.is_whitespace() && ch != '\u{85}')
        });
        let length = line.text.len() - current.len();
        let desired = indentation.at_column(columns);
        if line.text[..length] != desired {
            changes.push(Change::new(
                line.range.start..line.range.start + length,
                desired,
            ));
        }
    }
    ChangeSet::new(changes)
}

fn is_input(transaction: &Transaction) -> bool {
    !transaction.changes().is_empty()
        && (transaction.is_user_event("input.type") || transaction.is_user_event("input.complete"))
}
