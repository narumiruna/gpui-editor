use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, ConfigurationError, EditError, EditorState, Extension, Facet, StateContext,
    StateEffectType, StateField, StateFieldOptions, Transaction,
    autocomplete::session::SourceResult,
    commands::CommandResult,
    language::{LanguageData, StreamSettings, fold::fold_service, language_data_provider},
    language_data::{NativeLanguage, NativeSyntax, SupportedDocument, SyntaxUpdateError},
    view::keymap::KeyBinding,
};

use super::{
    MarkdownDocument, MarkdownMarkupCommand, MarkdownParserConfig, markdown_keymap,
    paste_url_as_link,
};

/// Options with native consumers in the Markdown package extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkdownConfig {
    /// Enable the Enter and Backspace markup commands for host key dispatch.
    pub add_keymap: bool,
    /// Register the tag source in Markdown prose, not in mounted HTML.
    pub complete_html_tags: bool,
    /// Let the host convert a URL paste over selected plain text to a link.
    pub paste_url_as_link: bool,
}

impl Default for MarkdownConfig {
    fn default() -> Self {
        Self {
            add_keymap: true,
            complete_html_tags: true,
            paste_url_as_link: true,
        }
    }
}

#[derive(Clone)]
struct InstalledConfig {
    host: MarkdownConfig,
    parser: Arc<MarkdownParserConfig>,
}

static CONFIG: LazyLock<Facet<InstalledConfig, Option<InstalledConfig>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&InstalledConfig]| inputs.first().map(|config| (*config).clone()))
});
static DEFAULT_PARSER: LazyLock<Arc<MarkdownParserConfig>> = LazyLock::new(Arc::default);

pub(crate) struct MarkdownRefresh {
    source: Arc<SupportedDocument>,
    parser: Arc<MarkdownParserConfig>,
    published: Arc<SupportedDocument>,
}

static REFRESH: LazyLock<StateEffectType<Arc<MarkdownRefresh>>> =
    LazyLock::new(StateEffectType::new);

static DOCUMENT: LazyLock<StateField<SupportedDocument>> = LazyLock::new(|| {
    StateField::define_shared(
        |state| {
            let config = state.facet(&CONFIG)?;
            let config = config
                .as_ref()
                .as_ref()
                .ok_or(ConfigurationError::InvalidFieldData)?;
            parse(state.text().clone(), config).map(Arc::new)
        },
        |previous: &Arc<SupportedDocument>, update| {
            let next = update.state().facet(&CONFIG)?;
            let next = next
                .as_ref()
                .as_ref()
                .ok_or(ConfigurationError::InvalidFieldData)?;
            if let Some(refresh) = refresh_effects(update.transaction()).next() {
                if !Arc::ptr_eq(&refresh.parser, &next.parser) {
                    return Err(ConfigurationError::InvalidFieldData);
                }
                return Ok(Arc::clone(&refresh.published));
            }
            let start = update.start().facet(&CONFIG)?;
            if !start
                .as_ref()
                .as_ref()
                .is_some_and(|start| Arc::ptr_eq(&start.parser, &next.parser))
            {
                return parse(update.state().text().clone(), next).map(Arc::new);
            }
            let changes = update.transaction().changes();
            if changes.is_empty() {
                Ok(Arc::clone(previous))
            } else {
                previous
                    .reparse(changes)
                    .map(Arc::new)
                    .map_err(|error| match error {
                        SyntaxUpdateError::StrictSyntax(error) => {
                            ConfigurationError::StrictSyntax(error)
                        }
                        _ => ConfigurationError::InvalidFieldData,
                    })
            }
        },
        StateFieldOptions::default().provide(|_| {
            Extension::group([
                fold_service(|state, start, end| {
                    let snapshot = markdown_snapshot(state)?;
                    syntax(&snapshot).heading_fold(start, end)
                }),
                language_data_provider(|state, at, side| {
                    let Some(snapshot) = markdown_snapshot(state) else {
                        return Vec::new();
                    };
                    if let Some(data) = snapshot.markdown_mounted_data_at(state, at, side) {
                        return data;
                    }
                    let mut data = snapshot.data_at(at, side);
                    if config(state).is_some_and(|config| config.complete_html_tags)
                        && snapshot.language_at(at, side) == Some(NativeLanguage::Markdown)
                    {
                        data.push(Arc::clone(&HTML_TAG_COMPLETION));
                    }
                    data
                }),
            ])
        }),
    )
});

/// The callable `autocomplete` datum on the Markdown host language.
pub type MarkdownCompletionSource = fn(&EditorState, usize, bool) -> Option<SourceResult>;

static HTML_TAG_COMPLETION: LazyLock<Arc<LanguageData>> = LazyLock::new(|| {
    Arc::new(LanguageData::default().with(
        "autocomplete",
        complete_html_tag as MarkdownCompletionSource,
    ))
});

/// Install CommonMark for the whole document, with native HTML mounts and
/// no code-fence parser. The extension tracks accepted edits in UTF-8 bytes
/// and registers heading folds before syntax fallback. The first configuration
/// in extension precedence order wins. The host routes key events, clipboard
/// input, and completion queries.
pub fn markdown(config: MarkdownConfig) -> Extension {
    install(config, Arc::clone(&DEFAULT_PARSER))
}

/// Install a selected Markdown dialect and native code parsers.
/// The editor resolves a named parser before the fallback. It rebuilds the
/// snapshot when the parser configuration changes. A strict child parse error
/// rejects state creation or the complete transaction.
pub fn markdown_with_parser(config: MarkdownConfig, parser: MarkdownParserConfig) -> Extension {
    install(config, Arc::new(parser))
}

/// Return the parser options from `markdown_with_parser`. A stock
/// `markdown` installation leaves the native host's own fence routing alone.
pub fn configured_markdown_parser(state: &EditorState) -> Option<Arc<MarkdownParserConfig>> {
    let config = state
        .context()
        .facet(&CONFIG)
        .expect("typed Markdown config");
    let parser = &config.as_ref().as_ref()?.parser;
    (!Arc::ptr_eq(parser, &DEFAULT_PARSER)).then(|| Arc::clone(parser))
}

fn install(host: MarkdownConfig, parser: Arc<MarkdownParserConfig>) -> Extension {
    Extension::group([
        CONFIG.of(InstalledConfig { host, parser }),
        DOCUMENT.extension(),
    ])
}

fn parse(
    text: crate::Text,
    config: &InstalledConfig,
) -> Result<SupportedDocument, ConfigurationError> {
    SupportedDocument::parse_markdown(text, StreamSettings::default(), Arc::clone(&config.parser))
        .map_err(ConfigurationError::StrictSyntax)
}

/// The active syntax snapshot, or `None` if the extension is not installed.
/// Use [`markdown_completions`] to query the configured completion sources.
pub fn markdown_snapshot(state: &EditorState) -> Option<Arc<SupportedDocument>> {
    state
        .context()
        .field(&DOCUMENT)
        .expect("typed Markdown document field")
}

/// Stage a syntax-only update for fences whose selected loads now have support.
/// The host polls the loads separately. This does not run the resolver, start
/// attempts, or change the state. Return `None` if there is nothing to publish.
/// Apply the transaction to a state with the same Markdown snapshot and parser.
/// It does not record history. It bypasses filters, but extenders still run.
/// A stale plan or a combined text, selection, or parser change is rejected.
pub fn plan_markdown_refresh(state: &EditorState) -> Result<Option<Transaction>, EditError> {
    let (Some(source), Some(parser)) =
        (markdown_snapshot(state), configured_markdown_parser(state))
    else {
        return Ok(None);
    };
    let Some(published) = source
        .refresh_markdown_mounts()
        .map_err(ConfigurationError::StrictSyntax)?
    else {
        return Ok(None);
    };
    let refresh = MarkdownRefresh {
        source,
        parser,
        published: Arc::new(published),
    };
    Ok(Some(
        Transaction::default()
            .with_effects([REFRESH.of(Arc::new(refresh))])
            .with_add_to_history(false)
            .without_filtering(),
    ))
}

fn refresh_effects(transaction: &Transaction) -> impl Iterator<Item = &Arc<MarkdownRefresh>> {
    transaction
        .effects()
        .iter()
        .filter_map(|effect| effect.get(&REFRESH))
}

pub(crate) fn checked_markdown_refresh(
    start: StateContext<'_>,
    transaction: &Transaction,
) -> Result<Option<Arc<MarkdownRefresh>>, ConfigurationError> {
    let mut effects = refresh_effects(transaction);
    let Some(refresh) = effects.next() else {
        return Ok(None);
    };
    if effects.next().is_some()
        || !transaction.changes().is_empty()
        || transaction.selection().is_some()
        || transaction.add_to_history()
        || !start
            .field(&DOCUMENT)?
            .is_some_and(|doc| Arc::ptr_eq(&doc, &refresh.source))
    {
        return Err(ConfigurationError::InvalidFieldData);
    }
    Ok(Some(Arc::clone(refresh)))
}

impl MarkdownRefresh {
    pub(crate) fn check_published(
        &self,
        start: StateContext<'_>,
        next: StateContext<'_>,
    ) -> Result<(), ConfigurationError> {
        // The field update does not run if reconfiguration removes Markdown.
        if start.selection() != next.selection()
            || !next
                .field(&DOCUMENT)?
                .is_some_and(|doc| Arc::ptr_eq(&doc, &self.published))
        {
            return Err(ConfigurationError::InvalidFieldData);
        }
        Ok(())
    }
}

fn config(state: &EditorState) -> Option<MarkdownConfig> {
    state
        .context()
        .facet(&CONFIG)
        .expect("typed Markdown config")
        .as_ref()
        .as_ref()
        .map(|config| config.host)
}

fn syntax(document: &SupportedDocument) -> &MarkdownDocument {
    let NativeSyntax::Markdown(syntax) = document.syntax() else {
        unreachable!("Markdown support uses Markdown syntax");
    };
    syntax
}

/// Get the configured markup bindings. A host should try these before its
/// default bindings, falling through when the command returns no transaction.
pub fn markdown_keymap_for(state: &EditorState) -> Option<[KeyBinding<MarkdownMarkupCommand>; 2]> {
    config(state).filter(|config| config.add_keymap)?;
    Some(markdown_keymap())
}

/// Plan a configured key command against the current syntax. Return `None`
/// when disabled, read-only, or the command cannot edit all selection ranges.
pub fn plan_markdown_markup(state: &EditorState, command: MarkdownMarkupCommand) -> CommandResult {
    if state.read_only() || !config(state).is_some_and(|config| config.add_keymap) {
        return Ok(None);
    }
    let Some(snapshot) = markdown_snapshot(state) else {
        return Ok(None);
    };
    if state.selection().ranges().iter().any(|range| {
        let at = range.range().start;
        let markdown = Some(NativeLanguage::Markdown);
        snapshot.language_at(at, Assoc::Before) != markdown
            && (command != MarkdownMarkupCommand::Continue
                || snapshot.language_at(at, Assoc::After) != markdown)
    }) {
        return Ok(None);
    }
    command.run(syntax(&snapshot), state)
}

/// Try Markdown's paste policy on raw clipboard text. When it declines, the
/// host can run the normal clipboard planner and its input filters.
pub fn plan_markdown_paste(state: &EditorState, clipboard: &str) -> CommandResult {
    if state.read_only() || !config(state).is_some_and(|config| config.paste_url_as_link) {
        return Ok(None);
    }
    let Some(snapshot) = markdown_snapshot(state) else {
        return Ok(None);
    };
    if snapshot.language_at(state.selection().main().range().start, Assoc::After)
        != Some(NativeLanguage::Markdown)
    {
        return Ok(None);
    }
    Ok(paste_url_as_link(syntax(&snapshot), state, clipboard))
}

/// Query the installed completion source at an original UTF-8 byte offset.
/// Mounted HTML services are independent of `complete_html_tags`.
pub fn markdown_completions(
    state: &EditorState,
    position: usize,
    explicit: bool,
) -> Vec<SourceResult> {
    let Some(snapshot) = markdown_snapshot(state) else {
        return Vec::new();
    };
    match snapshot.language_at(position, Assoc::Before) {
        Some(NativeLanguage::Markdown) => state
            .language_data_at::<MarkdownCompletionSource>("autocomplete", position, Assoc::Before)
            .into_iter()
            .filter_map(|source| source(state, position, explicit))
            .collect(),
        Some(_) => snapshot.completions(position, explicit),
        None => Vec::new(),
    }
}

fn complete_html_tag(state: &EditorState, at: usize, explicit: bool) -> Option<SourceResult> {
    config(state).filter(|config| config.complete_html_tags)?;
    let snapshot = markdown_snapshot(state)?;
    if snapshot.language_at(at, Assoc::Before) != Some(NativeLanguage::Markdown) {
        return None;
    }
    super::markdown_completion_source(syntax(&snapshot), at, explicit)
}
