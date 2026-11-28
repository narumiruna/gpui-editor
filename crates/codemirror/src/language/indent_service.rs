use std::{
    collections::HashMap,
    num::NonZeroUsize,
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Change, ChangeSet, EditError, EditorState, Extension, Facet, Text,
    language_data::{
        NativeSyntax, SupportedDocument, attached_language_snapshot, indent_catalog_markdown_html,
    },
};

use super::{
    IndentContext, IndentOptions, StreamSettings,
    lezer::indent::rust_indentation,
    line_projection::{command_line_projection, keeps_separate_breaks, splits_crlf},
    markdown::markdown_snapshot,
};

mod mounted;

pub use mounted::{MarkdownCodeIndent, MarkdownWastIndent};

/// The repeated whitespace character that forms one indentation unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndentStyle {
    character: char,
    count: NonZeroUsize,
}

impl Default for IndentStyle {
    fn default() -> Self {
        Self {
            character: ' ',
            count: NonZeroUsize::new(2).unwrap(),
        }
    }
}

impl IndentStyle {
    /// Accept a nonempty string that repeats one ECMAScript whitespace character.
    pub fn parse(unit: &str) -> Result<Self, EditError> {
        let mut chars = unit.chars();
        let character = chars
            .next()
            .filter(|&ch| is_space(ch))
            .ok_or(EditError::InvalidRange)?;
        let count = unit.len() / character.len_utf8();
        if !chars.all(|ch| ch == character) {
            return Err(EditError::InvalidRange);
        }
        Ok(Self {
            character,
            count: NonZeroUsize::new(count).unwrap(),
        })
    }

    /// The width of one indentation unit. Tabs use the editor's tab stop width.
    pub fn width(self, tab_size: NonZeroUsize) -> usize {
        if self.character == '\t' {
            self.count.get().saturating_mul(tab_size.get())
        } else {
            self.count.get()
        }
    }

    /// Build text from column zero. A tab style uses spaces for a final partial tab.
    pub fn at_column(self, tab_size: NonZeroUsize, column: usize) -> String {
        if self.character == '\t' {
            let tabs = tab_size.get();
            ["\t".repeat(column / tabs), " ".repeat(column % tabs)].concat()
        } else {
            self.character.to_string().repeat(column)
        }
    }
}

/// The first indentation style in extension precedence order wins.
pub static INDENT_UNIT: LazyLock<Facet<IndentStyle, IndentStyle>> = LazyLock::new(|| {
    Facet::define(
        |inputs: &[&IndentStyle]| {
            inputs
                .first()
                .map_or_else(IndentStyle::default, |style| **style)
        },
        |options| options.compare(PartialEq::eq),
    )
});

/// A service can defer, supply a column, or stop without a column. In the
/// last case, a command can inherit indentation from the previous line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndentDecision {
    Defer,
    Inherit,
    Column(usize),
}

type Provider = Arc<dyn Fn(&EditorState, &IndentContext<'_>) -> IndentDecision + Send + Sync>;
static SERVICES: LazyLock<Facet<Provider, Vec<Provider>>> = LazyLock::new(Facet::collect);
static SYNTAX_FALLBACKS: LazyLock<Facet<Provider, Vec<Provider>>> = LazyLock::new(Facet::collect);

/// Register an indentation service. Services run before the current syntax rule.
pub fn indent_service(
    provider: impl Fn(&EditorState, &IndentContext<'_>) -> IndentDecision + Send + Sync + 'static,
) -> Extension {
    SERVICES.of(Arc::new(provider))
}

pub(super) fn syntax_indent_fallback(
    provider: impl Fn(&EditorState, &IndentContext<'_>) -> IndentDecision + Send + Sync + 'static,
) -> Extension {
    SYNTAX_FALLBACKS.of(Arc::new(provider))
}

impl EditorState {
    pub(crate) fn has_indent_services(&self) -> Result<bool, EditError> {
        Ok(!self.context().facet(&SERVICES)?.is_empty())
    }
}

pub fn get_indent_unit(state: &EditorState) -> usize {
    style(state).width(state.tab_size())
}

pub fn indent_string(state: &EditorState, columns: usize) -> String {
    style(state).at_column(state.tab_size(), columns)
}

pub(crate) fn style(state: &EditorState) -> IndentStyle {
    *state
        .context()
        .facet(&INDENT_UNIT)
        .expect("typed indentation facet")
}

/// Get indentation at a byte position. Consult services in priority order,
/// then current explicit or attached syntax, including installed Markdown code
/// mounts with WAST, JSON, Lezer, or configured Rust syntax. A matching syntax projection
/// supplies logical line bounds. Ignore stale syntax.
pub fn get_indentation(
    state: &EditorState,
    position: usize,
    options: IndentOptions<'_>,
    syntax: Option<&NativeSyntax>,
) -> Result<Option<usize>, EditError> {
    Indenter::new(state, syntax)?.at(position, options)
}

/// Create changes for every logical line touched by the inclusive endpoints.
/// Later queries see the indentation computed for earlier replacements. A
/// whitespace-only line is cleared only if its query produced a column.
/// Do not apply a projected edit that joins separate line breaks.
pub fn indent_range(
    state: &EditorState,
    range: Range<usize>,
    syntax: Option<&NativeSyntax>,
) -> Result<ChangeSet, EditError> {
    if !state.text().is_valid_range(range.clone()) {
        return Err(EditError::InvalidRange);
    }
    let indenter = Indenter::new(state, syntax)?;
    let text = indenter.lines();
    let first = text.line_at(range.start).unwrap().number;
    let last = text.line_at(range.end).unwrap().number;
    let style = style(state);
    let mut updated = HashMap::new();
    let mut changes = Vec::new();
    for number in first..=last {
        let line = text.line(number).unwrap();
        let start = line.range.start;
        let lookup = |start| updated.get(&start).copied();
        let options = IndentOptions {
            override_indentation: Some(&lookup),
            ..IndentOptions::default()
        };
        let Some(column) = indenter.at(start, options)? else {
            continue;
        };
        let current = line.text.len() - line.text.trim_start_matches(is_space).len();
        let column = if current == line.text.len() {
            0
        } else {
            column
        };
        let desired = style.at_column(indenter.settings.tab_size, column);
        if line.text[..current] != desired {
            changes.push(Change::new(start..start + current, desired));
            updated.insert(start, column);
        }
    }
    let changes = ChangeSet::new(changes)?;
    if !std::ptr::eq(text, state.text()) && !keeps_separate_breaks(state.text().as_str(), &changes)
    {
        return Ok(ChangeSet::default());
    }
    Ok(changes)
}

struct Indenter<'a> {
    state: &'a EditorState,
    projection: Option<Text>,
    explicit: Option<&'a NativeSyntax>,
    tracked: Option<Arc<SupportedDocument>>,
    markdown: Option<Arc<SupportedDocument>>,
    settings: StreamSettings,
    services: Arc<Vec<Provider>>,
    syntax_fallbacks: Arc<Vec<Provider>>,
}

impl<'a> Indenter<'a> {
    fn new(state: &'a EditorState, syntax: Option<&'a NativeSyntax>) -> Result<Self, EditError> {
        let explicit = syntax.filter(|syntax| syntax.text() == state.text());
        let tracked = if let Some(explicit) = explicit {
            matches!(explicit, NativeSyntax::Rust(_))
                .then(|| attached_language_snapshot(state))
                .flatten()
                .filter(|doc| std::ptr::eq(doc.syntax(), explicit))
        } else {
            attached_language_snapshot(state)
        };
        let markdown = (explicit.is_none() && tracked.is_none())
            .then(|| markdown_snapshot(state))
            .flatten();
        Ok(Self {
            state,
            projection: command_line_projection(state),
            explicit,
            tracked,
            markdown,
            settings: StreamSettings {
                tab_size: state.tab_size(),
                indent_unit: get_indent_unit(state),
            },
            services: state.context().facet(&SERVICES)?,
            syntax_fallbacks: state.context().facet(&SYNTAX_FALLBACKS)?,
        })
    }

    fn syntax(&self) -> Option<&NativeSyntax> {
        self.explicit
            .or_else(|| self.tracked.as_deref().map(SupportedDocument::syntax))
    }

    fn lines(&self) -> &Text {
        self.projection
            .as_ref()
            .or_else(|| self.syntax().and_then(NativeSyntax::projected_lines))
            .or_else(|| match self.markdown.as_ref()?.syntax() {
                NativeSyntax::Markdown(doc) => doc.projected_lines(),
                _ => None,
            })
            .unwrap_or(self.state.text())
    }

    fn at(&self, position: usize, options: IndentOptions<'_>) -> Result<Option<usize>, EditError> {
        let context = IndentContext::new(self.lines(), position, self.settings, options)
            .ok_or(EditError::InvalidRange)?;
        Ok(indent_with_services(
            self.state,
            &self.services,
            &context,
            || match self.syntax() {
                Some(NativeSyntax::Wast(doc)) => {
                    doc.indentation_with_context(Some(self.state), &context)
                }
                Some(NativeSyntax::Json(doc)) => {
                    doc.indentation_with_state(Some(self.state), position, self.settings, options)
                }
                Some(NativeSyntax::LezerGrammar(doc)) => {
                    doc.indentation_with_state(Some(self.state), position, self.settings, options)
                }
                Some(NativeSyntax::Rust(doc))
                    if self.tracked.is_some() && !splits_crlf(doc.text().as_str(), position) =>
                {
                    rust_indentation(doc, self.state, &context)
                }
                Some(syntax @ NativeSyntax::Markdown(_)) => self
                    .tracked
                    .as_deref()
                    .and_then(|doc| indent_catalog_markdown_html(self.state, doc, &context))
                    .or_else(|| syntax.indentation(position, self.settings, options)),
                Some(syntax) => syntax.indentation(position, self.settings, options),
                None => indent_with_services(self.state, &self.syntax_fallbacks, &context, || {
                    self.markdown
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot.markdown_code_indentation(self.state, &context)
                        })
                        .or_else(|| (position == 0).then_some(0))
                }),
            },
        ))
    }
}

fn indent_with_services(
    state: &EditorState,
    services: &[Provider],
    context: &IndentContext<'_>,
    syntax: impl FnOnce() -> Option<usize>,
) -> Option<usize> {
    for service in services {
        match service(state, context) {
            IndentDecision::Defer => {}
            IndentDecision::Inherit => return None,
            IndentDecision::Column(column) => return Some(column),
        }
    }
    syntax()
}

fn is_space(ch: char) -> bool {
    ch == '\u{feff}' || ch.is_whitespace() && ch != '\u{85}'
}
