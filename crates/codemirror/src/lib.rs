//! Text, selections, transactions, and undo history for a native editor.
//!
//! All positions are UTF-8 byte offsets. Text retains its original line endings.

use std::sync::Arc;

pub mod autocomplete;
mod change;
mod change_by_range;
pub mod collab;
mod columns;
pub mod commands;
mod config_merge;
mod extension;
mod history;
pub mod language;
pub mod language_data;
pub mod legacy_modes;
pub mod lint;
pub mod lsp;
pub mod merge;
mod range_set;
pub mod search;
mod selection;
mod serialization;
pub mod setup;
mod state_config;
pub mod stream_parser;
mod text;
pub mod theme_one_dark;
mod transaction_filter;
mod transaction_merge;
mod transaction_meta;
pub mod utf16;
pub mod view;
mod word;

pub use change::{Assoc, Change, ChangeDesc, ChangeSet, ChangeSpec, MapMode, RangeTouch};
pub use change_by_range::RangeEdit;
pub use columns::{count_column, find_column};
pub use config_merge::combine_config;
use extension::Extensions;
pub use extension::{
    Compartment, ConfigurationError, Extension, Facet, FacetReader, Precedence, StateContext,
    StateField, StateFieldOptions, StateFieldReader, StateUpdate,
};
pub use history::{History, HistoryIsolation, inverted_effects};
use language::{lezer::LrPublication, wast::WastPublication};
pub use range_set::{
    Range, RangeComparator, RangeSet, RangeSetBuilder, RangeSetError, RangeSetUpdate, RangeValue,
    SpanIterator,
};
pub use selection::{EditorSelection, SelectionRange};
pub use serialization::{JsonField, SerializationError};
pub use state_config::{
    ALLOW_MULTIPLE_SELECTIONS, LINE_SEPARATOR, PHRASES, PhraseMap, READ_ONLY, TAB_SIZE,
};
pub use text::{Line, Text, TextCursor, TextLineCursor};
pub use transaction_filter::{
    ChangePermission, TransactionExtension, change_filter, transaction_extender, transaction_filter,
};
pub use transaction_merge::TransactionCoordinates;
pub use transaction_meta::{Annotation, AnnotationType, StateEffect, StateEffectType};
pub use word::{CharCategorizer, CharCategory, find_cluster_break, word_characters};

/// An invalid edit or selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditError {
    InvalidRange,
    OverlappingChanges,
    InvalidSelection,
    MismatchedLength,
    /// The replacement would make the document too long to address as a string.
    LengthOverflow,
    Configuration(ConfigurationError),
}

impl From<ConfigurationError> for EditError {
    fn from(error: ConfigurationError) -> Self {
        Self::Configuration(error)
    }
}

/// A group of changes in old-document coordinates with an optional new selection.
#[derive(Clone, Debug, Default)]
pub struct Transaction {
    changes: ChangeSet,
    selection: Option<EditorSelection>,
    effects: Vec<StateEffect>,
    annotations: Vec<Annotation>,
    scroll_into_view: bool,
    unfiltered: bool,
}

impl Transaction {
    pub fn new(changes: ChangeSet) -> Self {
        Self {
            changes,
            ..Self::default()
        }
    }

    pub fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    pub fn selection(&self) -> Option<&EditorSelection> {
        self.selection.as_ref()
    }

    pub fn effects(&self) -> &[StateEffect] {
        &self.effects
    }

    pub fn annotation<T: Send + Sync + 'static>(&self, kind: &AnnotationType<T>) -> Option<&T> {
        self.annotations
            .iter()
            .find_map(|annotation| annotation.get(kind))
    }

    pub fn with_effects(mut self, effects: impl IntoIterator<Item = StateEffect>) -> Self {
        self.effects.extend(effects);
        self
    }

    pub fn with_annotation(mut self, annotation: Annotation) -> Self {
        self.annotations.push(annotation);
        self
    }

    pub fn scroll_into_view(&self) -> bool {
        self.scroll_into_view
    }

    pub fn with_scroll_into_view(mut self) -> Self {
        self.scroll_into_view = true;
        self
    }

    /// Bypass change and transaction filters. Extenders still run.
    pub fn without_filtering(mut self) -> Self {
        self.unfiltered = true;
        self
    }

    pub fn with_selection(mut self, selection: EditorSelection) -> Self {
        self.selection = Some(selection);
        self
    }
}

#[derive(Debug)]
struct AppliedEdit {
    forward: ChangeSet,
    inverse: ChangeSet,
}

/// The authoritative text and selection, with no dependency on a view.
#[derive(Clone, Debug)]
pub struct EditorState {
    text: Text,
    selection: EditorSelection,
    extensions: Extensions,
    wast_publication: Option<Arc<WastPublication>>,
    lr_publication: Option<Arc<LrPublication>>,
}

impl EditorState {
    pub fn new(source: impl Into<String>) -> Self {
        Self::from_text(Text::new(source))
    }

    /// Use an existing document without copying its storage or changing its bytes.
    /// The selection starts at byte zero and there are no extensions.
    pub fn from_text(text: Text) -> Self {
        Self {
            text,
            selection: EditorSelection::single(0, 0),
            extensions: Extensions::default(),
            wast_publication: None,
            lr_publication: None,
        }
    }

    /// Create a state with an initial set of extensions.
    pub fn with_extensions(
        source: impl Into<String>,
        extension: Extension,
    ) -> Result<Self, ConfigurationError> {
        let mut state = Self::new(source);
        state.extensions = Extensions::create(extension, &state.text, &state.selection)?;
        Ok(state)
    }

    pub fn context(&self) -> StateContext<'_> {
        self.extensions.context(&self.text, &self.selection)
    }

    pub fn compartment(&self, compartment: &Compartment) -> Option<&Extension> {
        self.extensions.compartment(compartment)
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn selection(&self) -> &EditorSelection {
        &self.selection
    }

    fn set_selection(&mut self, selection: EditorSelection) -> Result<(), EditError> {
        self.update(
            Transaction::default()
                .with_selection(selection)
                .without_filtering(),
        )
    }

    /// Apply an edit without recording history. Use [`History::update`] for undoable edits.
    /// Errors leave the state unchanged.
    pub fn update(&mut self, transaction: Transaction) -> Result<(), EditError> {
        self.apply(transaction).map(|_| ())
    }

    fn apply(&mut self, transaction: Transaction) -> Result<Option<AppliedEdit>, EditError> {
        let transaction = self.filter_transaction(transaction)?;
        self.apply_resolved(transaction)
    }

    // A native parser publication is not a host edit. It updates the resolved
    // fields without running filters, extenders, or touching selection/history.
    pub(crate) fn refresh_viewport_fields(&mut self) -> Result<ChangeDesc, EditError> {
        let transaction = Transaction::default().with_add_to_history(false);
        let changes = transaction.changes().desc(self.text.len())?;
        let current = (&self.text, &self.selection);
        let extensions = self
            .extensions
            .update(current, current, &transaction, &changes, None)?;
        self.extensions = extensions;
        Ok(changes)
    }

    fn apply_resolved(
        &mut self,
        transaction: Transaction,
    ) -> Result<Option<AppliedEdit>, EditError> {
        self.apply_with_replacement_policy(transaction, false)
    }

    fn apply_with_replacement_policy(
        &mut self,
        mut transaction: Transaction,
        force_preserve_replacements: bool,
    ) -> Result<Option<AppliedEdit>, EditError> {
        let source = &self.text;
        transaction.changes.check(source)?;
        if let Some(selection) = &transaction.selection {
            selection.check(|pos| transaction.changes.is_boundary_after(source, pos))?;
        }
        let markdown_refresh =
            language::markdown::checked_markdown_refresh(self.context(), &transaction)?;
        let reconfiguration = self.extensions.prepare_reconfiguration(&transaction)?;
        collab::validate_receipt(self, &transaction, reconfiguration.as_ref())?;
        let preserve_replacements = force_preserve_replacements
            || (self.extensions.active()
                && *self.context().facet(&state_config::PRESERVE_REPLACEMENTS)?)
            || reconfiguration
                .as_ref()
                .is_some_and(|config| config.has_facet(&state_config::PRESERVE_REPLACEMENTS));
        let changes = &mut transaction.changes;
        if !preserve_replacements {
            changes.remove_noops(source);
        }
        let selection = match transaction.selection.clone() {
            Some(selection) => selection,
            None => {
                let selection = self.selection.map(changes);
                selection.check(|pos| changes.is_boundary_after(source, pos))?;
                selection
            }
        };
        let selection = if selection.ranges().len() > 1 && !self.allows_multiple_selections() {
            selection.as_single()
        } else {
            selection
        };
        let inverse = (!changes.is_empty()).then(|| changes.invert(source));
        let changes = &transaction.changes;
        if self.extensions.active() || reconfiguration.is_some() {
            let desc = changes.desc(source.len())?;
            let mut text = self.text.clone();
            changes.apply(&mut text);
            let reconfigured = reconfiguration.is_some();
            let extensions = self.extensions.update(
                (&self.text, &self.selection),
                (&text, &selection),
                &transaction,
                &desc,
                reconfiguration,
            )?;
            if let Some(refresh) = markdown_refresh {
                refresh.check_published(self.context(), extensions.context(&text, &selection))?;
            }
            if !changes.is_empty() {
                self.wast_publication = None;
                self.lr_publication = None;
            } else if reconfigured {
                let next = extensions.context(&text, &selection);
                if self.wast_publication.is_some() {
                    self.retain_wast_publication(&next);
                }
                if self.lr_publication.is_some() {
                    self.retain_lr_publication(&next);
                }
            }
            self.text = text;
            self.extensions = extensions;
        } else {
            changes.apply(&mut self.text);
        }
        self.selection = selection;
        Ok(inverse.map(|inverse| AppliedEdit {
            forward: transaction.changes,
            inverse,
        }))
    }
}
