use crate::{
    Assoc, Change, ChangeSet, EditError, EditorState, Extension, Transaction,
    TransactionCoordinates,
    commands::Indentation,
    language::{IndentOptions, StreamSettings, line_projection::editor_lines},
    transaction_filter,
};

use super::{NativeLanguageSupport, SupportedDocument};

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
        let data = self.data_at(position, Assoc::Before);
        let mut rules =
            state.language_data_at::<fn(&str) -> bool>("indentOnInput", position, Assoc::Before);
        rules.extend(
            data.iter()
                .filter_map(|entry| entry.get::<fn(&str) -> bool>("indentOnInput").copied()),
        );
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
        let head = selection.main().head();
        let line = lines.line_at(head).ok_or(EditError::InvalidSelection)?;
        let prefix = &lines.as_str()[line.range.start..head];
        if prefix.encode_utf16().take(201).count() > 200
            || !rules.into_iter().any(|rule| rule(prefix))
        {
            return Ok(transaction);
        }
        let settings = StreamSettings {
            tab_size: indentation.tab_size,
            indent_unit: indentation.width(),
        };
        let parsed = if matches!(self.syntax, super::NativeSyntax::Wast(_)) {
            let Ok(parsed) = self.reparse(transaction.changes()) else {
                return Ok(transaction);
            };
            parsed
        } else if let Some(parser) = &self.markdown_parser {
            let Ok(parsed) = Self::parse_markdown(text.clone(), settings, parser.clone()) else {
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
            let Some(columns) =
                parsed
                    .syntax()
                    .indentation(line.range.start, settings, IndentOptions::default())
            else {
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
        if changes.is_empty() {
            return Ok(transaction);
        }
        transaction.merge(
            Transaction::new(ChangeSet::new(changes)?),
            old_text.len(),
            TransactionCoordinates::Sequential,
        )
    }
}

fn is_input(transaction: &Transaction) -> bool {
    !transaction.changes().is_empty()
        && (transaction.is_user_event("input.type") || transaction.is_user_event("input.complete"))
}
