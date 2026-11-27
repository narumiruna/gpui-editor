use std::sync::{Arc, LazyLock};

use crate::{
    ConfigurationError, EditorState, Extension, Facet, StateContext, StateField, StateFieldOptions,
    Text,
    language::{StreamSettings, language_data_provider},
};

use super::{NativeLanguageSupport, NativeSyntax, SupportedDocument, SyntaxUpdateError};

mod json;
mod lezer;
mod lr;
mod wast;
pub use json::{JsonLanguage, json_language};
pub use lezer::{LezerLanguage, lezer_language};
pub(crate) use wast::WastConfig;
pub use wast::{WastLanguage, wast_language};

#[derive(Clone)]
enum Attachment {
    Native(NativeLanguageSupport),
    Lr(Arc<lr::LrConfig>),
    Wast(Arc<WastConfig>),
    WastViewport(Arc<WastConfig>),
}

impl Attachment {
    fn same_parser(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(a), Self::Native(b)) => a == b,
            (Self::Lr(a), Self::Lr(b)) => Arc::ptr_eq(a, b),
            (Self::Wast(a), Self::Wast(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    fn parse(&self, text: Text) -> Result<Option<SupportedDocument>, ConfigurationError> {
        match self {
            Self::Lr(config) => config
                .parse(text, StreamSettings::default())
                .map(Some)
                .map_err(ConfigurationError::StrictSyntax),
            Self::Wast(config) => wast::parse(text, config).map(Some),
            Self::WastViewport(_) => Ok(None),
            Self::Native(support) => Ok(support.parse(text).ok()),
        }
    }
}

#[derive(Clone)]
struct ActiveDocument {
    attachment: Attachment,
    document: Arc<SupportedDocument>,
}

static ACTIVE_SUPPORT: LazyLock<Facet<Attachment, Option<Attachment>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Attachment]| inputs.first().map(|support| (**support).clone()))
});

static ACTIVE_DOCUMENT: LazyLock<StateField<Option<ActiveDocument>>> = LazyLock::new(|| {
    StateField::new(parse_active, |previous: &Option<ActiveDocument>, update| {
        let state = update.state();
        let Some(attachment) = active_support(state)? else {
            return Ok(None);
        };
        if let Some(previous) = previous
            .as_ref()
            .filter(|old| old.attachment.same_parser(&attachment))
        {
            let changes = update.transaction().changes();
            if changes.is_empty() {
                return Ok(Some(previous.clone()));
            }
            return match previous.document.reparse(changes) {
                Ok(doc) => Ok(Some(ActiveDocument {
                    attachment,
                    document: Arc::new(doc),
                })),
                Err(SyntaxUpdateError::StrictSyntax(error)) => {
                    Err(ConfigurationError::StrictSyntax(error))
                }
                Err(_) => Ok(None),
            };
        }
        parse_document(attachment, state.text().clone())
    })
});

fn active_support(state: &StateContext<'_>) -> Result<Option<Attachment>, ConfigurationError> {
    state
        .facet(&ACTIVE_SUPPORT)
        .map(|support| support.as_ref().clone())
}

pub(crate) fn active_wast_viewport(state: &StateContext<'_>) -> Option<Arc<WastConfig>> {
    match active_support(state).expect("typed attached parser")? {
        Attachment::WastViewport(config) => Some(config),
        _ => None,
    }
}

fn parse_active(state: &StateContext<'_>) -> Result<Option<ActiveDocument>, ConfigurationError> {
    match active_support(state)? {
        Some(attachment) => parse_document(attachment, state.text().clone()),
        None => Ok(None),
    }
}

fn parse_document(
    attachment: Attachment,
    text: Text,
) -> Result<Option<ActiveDocument>, ConfigurationError> {
    Ok(attachment.parse(text)?.map(|document| ActiveDocument {
        attachment,
        document: Arc::new(document),
    }))
}

fn install_attachment(attachment: Attachment) -> Extension {
    Extension::group([
        ACTIVE_SUPPORT.of(attachment),
        ACTIVE_DOCUMENT.extension(),
        // Each language installation adds one provider for the active attachment.
        language_data_provider(|state, position, side| {
            if let Some(document) = attached_language_snapshot(state) {
                return match document.syntax() {
                    NativeSyntax::Wast(syntax) => syntax.configured_data_at(state, position, side),
                    syntax => match syntax.lr_root_type() {
                        Some(row) => row
                            .data_facet()
                            .map_or_else(Vec::new, |facet| facet.read(state)),
                        None => document.data_at(position, side),
                    },
                };
            }
            state.wast_viewport_data(position, side)
        }),
    ])
}

/// The read-only snapshot of the active synchronous language attachment.
/// Return `None` without an attachment or if parsing failed. Reading does not parse.
/// The first installed attachment in precedence order is active. Switching
/// synchronous languages parses the final text; previously returned snapshots
/// stay valid. This excludes viewport WAST, even after a complete parse.
pub fn attached_language_snapshot(state: &EditorState) -> Option<Arc<SupportedDocument>> {
    state
        .context()
        .field(&ACTIVE_DOCUMENT)
        .expect("typed active syntax field")?
        .as_ref()
        .as_ref()
        .map(|active| Arc::clone(&active.document))
}

impl NativeLanguageSupport {
    /// Install positional language metadata for the whole editor document.
    /// This uses the default parser settings and mounts from [`Self::parse`].
    /// It does not install the other native language services.
    ///
    /// The field parses at installation and follows accepted text edits with
    /// the native reparse APIs. It keeps the snapshot on selection-only changes.
    /// If parsing fails, the provider returns no data until a later text edit
    /// can be parsed. This does not run a viewport or background parse worker.
    pub fn tracked_language_data_extension(self) -> Extension {
        self.tracked_language_data_field().extension()
    }

    pub(in crate::language_data) fn tracked_syntax_extension(self) -> Extension {
        install_attachment(Attachment::Native(self))
    }

    fn tracked_language_data_field(self) -> StateField<Option<SupportedDocument>> {
        StateField::<Option<SupportedDocument>>::define_shared(
            move |state| Ok(Arc::new(self.parse(state.text().clone()).ok())),
            move |document, update| {
                // The state has already filtered and validated these edits.
                let changes = update.transaction().changes();
                if changes.is_empty() {
                    return Ok(Arc::clone(document));
                }
                let next = match document.as_ref() {
                    Some(document) => document.reparse(changes).ok(),
                    None => self.parse(update.state().text().clone()).ok(),
                };
                Ok(Arc::new(next))
            },
            StateFieldOptions::<Option<SupportedDocument>>::default().provide(|field| {
                language_data_provider(move |state, position, side| {
                    state
                        .context()
                        .field(&field)
                        .expect("typed tracked language data field")
                        .as_deref()
                        .and_then(Option::as_ref)
                        .map_or_else(Vec::new, |document| document.data_at(position, side))
                })
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Assoc, Change, ChangePermission, ChangeSet, Compartment, EditorSelection, Transaction,
        change_filter,
        commands::CommentTokens,
        language::{IndentOptions, fold::foldable_in_state, get_indentation},
        language_data::{NativeLanguage, json_extension, wast_extension},
    };

    #[test]
    fn selection_only_updates_keep_the_field_and_text_edits_reuse_checkpoints() {
        let input = format!(
            "(module\r\n{}  (func $last)\r\n)",
            "  (func $f)\r\n".repeat(200)
        );
        let filter =
            change_filter(|_, _| ChangePermission::Protect(std::iter::once(0..7).collect()));
        let mut state =
            EditorState::with_extensions(&input, Extension::group([wast_extension(), filter]))
                .unwrap();
        let snapshot = |state: &EditorState| attached_language_snapshot(state).unwrap();
        let previous = snapshot(&state);
        let comments = state.language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].line, Some(";;"));
        assert_eq!(
            get_indentation(&state, 9, IndentOptions::default(), None),
            Ok(Some(2))
        );
        assert_eq!(foldable_in_state(&state, 0, 7), Some(1..input.len() - 1));
        assert!(Arc::ptr_eq(&previous, &snapshot(&state)));
        state
            .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
            .unwrap();
        let blocked = ChangeSet::single(Change::new(0..7, "blocked")).unwrap();
        state.update(Transaction::new(blocked)).unwrap();
        assert!(Arc::ptr_eq(&previous, &snapshot(&state)));
        let at = input.find("last").unwrap();
        let changes = ChangeSet::single(Change::new(at..at + 4, "end")).unwrap();
        state.update(Transaction::new(changes)).unwrap();
        let next = snapshot(&state);
        assert_eq!(next.text(), state.text());
        assert!(next.reuse_stats().nodes_reused > 0);
        assert_eq!(previous.text().as_str(), input);
    }

    #[test]
    fn returning_to_a_language_creates_a_new_active_tree() {
        let slot = Compartment::new();
        let mut state = EditorState::with_extensions(
            "(module)",
            Extension::group([slot.of(Extension::default()), wast_extension()]),
        )
        .unwrap();
        let original = attached_language_snapshot(&state).unwrap();
        state
            .update(Transaction::default().with_effects([slot.reconfigure(json_extension())]))
            .unwrap();
        state
            .update(Transaction::default().with_effects([slot.reconfigure(Extension::default())]))
            .unwrap();
        let restored = attached_language_snapshot(&state).unwrap();
        assert!(!Arc::ptr_eq(&original, &restored));
        let final_text = "{\r\n  \"😀\": 1\r}";
        let changes = ChangeSet::single(Change::new(0..state.text().len(), final_text)).unwrap();
        state
            .update(Transaction::new(changes).with_effects([slot.reconfigure(json_extension())]))
            .unwrap();
        let json = attached_language_snapshot(&state).unwrap();
        assert_eq!(json.language, NativeLanguage::Json);
        assert_eq!(json.text().as_str(), final_text);
        assert_eq!(restored.text().as_str(), "(module)");
    }

    #[test]
    fn missing_snapshot_retries_on_the_next_text_edit() {
        let field = NativeLanguage::Wast.support().tracked_language_data_field();
        let extension = field.init(|_| Ok(None));
        let mut state = EditorState::with_extensions("(module)", extension).unwrap();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
            .unwrap();
        assert!(state.context().field(&field).unwrap().unwrap().is_none());
        let changes = ChangeSet::single(Change::new(0..0, ";; 😀\r\n")).unwrap();
        state.update(Transaction::new(changes)).unwrap();
        let snapshot = state.context().field(&field).unwrap().unwrap();
        assert_eq!(snapshot.as_ref().as_ref().unwrap().text(), state.text());
    }
}
