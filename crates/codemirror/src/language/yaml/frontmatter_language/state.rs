use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, ConfigurationError, EditorState, Extension, Facet, StateField, StateFieldOptions,
    StateFieldReader,
    language::{
        IndentContext, IndentDecision, LanguageData,
        fold::syntax_fold_fallback,
        indent_service::syntax_indent_fallback,
        language_data_provider,
        line_projection::{EditorOffsets, splits_crlf},
        yaml,
    },
    language_data::{NativeSyntax, SyntaxUpdateError},
};

use super::{ContentParser, YamlFrontmatterDocument, YamlFrontmatterLanguage};

type WrapperField = StateFieldReader<Option<YamlFrontmatterDocument>>;
static WRAPPER_FIELD: LazyLock<Facet<WrapperField, Option<WrapperField>>> =
    LazyLock::new(|| Facet::new(|fields: &[&WrapperField]| fields.first().map(|field| **field)));

fn snapshot(
    result: Result<YamlFrontmatterDocument, SyntaxUpdateError>,
) -> Result<Arc<Option<YamlFrontmatterDocument>>, ConfigurationError> {
    match result {
        Ok(document) => Ok(Arc::new(Some(document))),
        Err(SyntaxUpdateError::StrictSyntax(error)) => Err(ConfigurationError::StrictSyntax(error)),
        Err(_) => Ok(Arc::new(None)),
    }
}

impl YamlFrontmatterLanguage {
    /// Track this wrapper in an editor state. Install the result with
    /// [`StateField::extension`]. Use [`Extension::default`] for no extra support.
    ///
    /// The supplied extensions are installed unchanged in the editor's
    /// configuration, not scoped to the body. Pass support extensions only,
    /// not a separate positional provider for the body parser.
    ///
    /// The field parses at installation and updates after accepted text edits.
    /// It retains the snapshot on other updates and reuses validated mount
    /// checkpoints when available. A strict body parse error rejects state
    /// creation or the transaction. A tokenization error leaves the snapshot
    /// and its language data absent until a later edit parses.
    pub fn state_field(&self, support: Extension) -> StateField<Option<YamlFrontmatterDocument>> {
        let initial = self.clone();
        let language = self.clone();
        StateField::define_shared(
            move |state| {
                let parsed = initial.parse(state.text().clone());
                if state.is_reconfiguration_preview()
                    && matches!(parsed, Err(SyntaxUpdateError::StrictSyntax(_)))
                {
                    // A transaction can repair the body while it installs strict mode.
                    return Ok(Arc::new(None));
                }
                snapshot(parsed)
            },
            move |document, update| {
                let changes = update.transaction().changes();
                if changes.is_empty()
                    && (document.is_some() || matches!(language.content, ContentParser::Native(_)))
                {
                    return Ok(Arc::clone(document));
                }
                if let Some(old) = document.as_ref()
                    && let result @ (Ok(_) | Err(SyntaxUpdateError::StrictSyntax(_))) =
                        old.reparse(changes)
                {
                    return snapshot(result);
                }
                snapshot(language.parse(update.state().text().clone()))
            },
            StateFieldOptions::<Option<YamlFrontmatterDocument>>::default().provide(|field| {
                Extension::group([
                    WRAPPER_FIELD.of(field),
                    language_data_provider(move |state, position, side| {
                        state
                            .context()
                            .field(&field)
                            .expect("typed front matter field")
                            .as_deref()
                            .and_then(Option::as_ref)
                            .map_or_else(Vec::new, |document| {
                                document.data_at(state, position, side)
                            })
                    }),
                    support,
                    syntax_fold_fallback(move |state, start, end| {
                        let snapshot = state
                            .context()
                            .field(&field)
                            .expect("typed front matter field")?;
                        snapshot
                            .as_ref()
                            .as_ref()?
                            .foldable_in_state(state, start..end)
                    }),
                    syntax_indent_fallback(move |state, context| {
                        let document = state
                            .context()
                            .field(&field)
                            .expect("typed front matter field");
                        let Some(document) = document.as_deref().and_then(Option::as_ref) else {
                            return IndentDecision::Defer;
                        };
                        document
                            .indentation_in_state(state, context)
                            .map_or(IndentDecision::Inherit, IndentDecision::Column)
                    }),
                ])
            }),
        )
    }
}

impl YamlFrontmatterDocument {
    pub(crate) fn state_snapshot(state: &EditorState) -> Option<Arc<Option<Self>>> {
        let context = state.context();
        let field = *context
            .facet(&WRAPPER_FIELD)
            .expect("typed front matter facet");
        context.field(&field?).expect("typed front matter field")
    }

    fn foldable_in_state(&self, state: &EditorState, line: Range<usize>) -> Option<Range<usize>> {
        if line.start >= self.body_start
            && !matches!(
                self.content,
                ContentParser::Native(_) | ContentParser::Yaml(_)
            )
        {
            return self
                .body()
                .foldable_in_scope(state, self.body_range(), line);
        }
        self.foldable(line.start, line.end)
    }

    fn indentation_in_state(
        &self,
        state: &EditorState,
        context: &IndentContext<'_>,
    ) -> Option<usize> {
        let position = context.position();
        let settings = context.settings();
        let options = context.options();
        if !matches!(
            self.content,
            ContentParser::Native(_) | ContentParser::Yaml(_)
        ) && (position > self.body_start || self.frontmatter.is_none())
        {
            let source = self.frontmatter.as_ref().map_or_else(
                || self.body().projected_lines().unwrap_or(&self.text),
                |frontmatter| frontmatter.yaml().syntax_lines(),
            );
            if splits_crlf(source.as_str(), position) {
                return None;
            }
            let lines = IndentContext::new(source, position, settings, options)?;
            return match self.body() {
                NativeSyntax::Wast(wast) => {
                    wast.indentation_in_body(self.body_start, state, &lines)
                }
                syntax => syntax.lr_indentation_in_body(self.body_start, state, &lines),
            };
        }
        self.indentation(position, settings, options)
    }

    pub(crate) fn yaml_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.frontmatter()
            .map(|front| front.yaml().root().range())
            .into_iter()
            .chain(matches!(self.body(), NativeSyntax::Yaml(_)).then(|| self.body_range()))
    }

    pub(crate) fn yaml_at(&self, position: usize, side: Assoc) -> bool {
        self.text.is_char_boundary(position)
            && !splits_crlf(self.text.as_str(), position)
            && self
                .yaml_ranges()
                .any(|range| enters(range, position, side))
    }

    fn data_at(&self, state: &EditorState, position: usize, side: Assoc) -> Vec<Arc<LanguageData>> {
        if self.yaml_at(position, side) {
            return yaml::stock_data_facet().read(state);
        }
        if enters(self.body_range(), position, side)
            && !matches!(self.body(), NativeSyntax::Yaml(_))
        {
            if !matches!(self.content, ContentParser::Native(_)) {
                if splits_crlf(state.text().as_str(), position) {
                    return Vec::new();
                }
                if let NativeSyntax::Wast(wast) = self.body() {
                    return wast.configured_data_in_body(state, self.body_start, side, || {
                        let offsets = self
                            .editor_offsets
                            .get_or_init(|| EditorOffsets::new(self.text.as_str()));
                        let at = offsets
                            .source(offsets.editor(position) - offsets.editor(self.body_start));
                        // Upstream can resolve into the parent tree after subtracting the mounted origin.
                        // A WAST predicate receives only WAST nodes; defer in that case.
                        enters(self.body_range(), at, side).then_some(at)
                    });
                }
                return self
                    .body
                    .root_facet_at(position - self.body_start, side)
                    .map_or_else(Vec::new, |facet| facet.read(state));
            }
            return self.body.data_at(position - self.body_start, side);
        }
        Vec::new()
    }
}

fn enters(range: Range<usize>, position: usize, side: Assoc) -> bool {
    match side {
        Assoc::Before => range.start < position && position <= range.end,
        Assoc::After => range.start <= position && position < range.end,
    }
}
