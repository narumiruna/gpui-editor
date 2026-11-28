//! Native `yamlFrontmatter` wrapping based on `@codemirror/lang-yaml` 6.1.3.
//! See `docs/licenses/codemirror-lang-yaml.txt`.
//!
//! The wrapper mounts a native body parser and can forward native extensions.
//! Its outer syntax tree is available separately from the mounted YAML and body.

use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

use crate::{
    Assoc, Change, ChangeSet, Text,
    language::{
        HighlightSpan, IndentOptions, StreamSettings,
        lezer::{ParseError, ReuseStats, SyntaxTree},
        line_projection::{EditorOffsets, splits_crlf},
    },
    language_data::{
        JsonLanguage, LezerLanguage, NativeLanguage, NativeSyntax, SupportedDocument,
        SyntaxUpdateError, WastLanguage, YamlLanguage,
    },
};

use super::YamlFrontmatter;

mod state;
mod tree;

/// Parse optional YAML front matter and mount the selected parser in the body.
#[derive(Clone, Eq, PartialEq)]
pub struct YamlFrontmatterLanguage {
    content: ContentParser,
}

#[derive(Clone, Eq, PartialEq)]
enum ContentParser {
    Native(NativeLanguage),
    Json(JsonLanguage),
    Lezer(LezerLanguage),
    Wast(WastLanguage),
    Yaml(YamlLanguage),
}

impl ContentParser {
    fn parse(&self, text: &Text, start: usize) -> Result<SupportedDocument, SyntaxUpdateError> {
        let body = text.slice_owned(start..text.len()).unwrap();
        let settings = StreamSettings::default();
        match self {
            Self::Native(language) => {
                return language
                    .support()
                    .parse(body)
                    .map_err(SyntaxUpdateError::Tokenize);
            }
            Self::Json(language) => language.parse_for_editor(body, settings),
            Self::Lezer(language) => language.parse_for_editor(body, settings),
            Self::Wast(language) => language.parse_for_editor(body, settings),
            Self::Yaml(language) => language.parse_for_editor(body, settings),
        }
        .map_err(|error| SyntaxUpdateError::StrictSyntax(error.map_offset(|at| start + at)))
    }
}

impl YamlFrontmatterLanguage {
    pub const fn new(content: NativeLanguage) -> Self {
        Self {
            content: ContentParser::Native(content),
        }
    }

    /// Wrap a JSON parser without discarding its strictness or node properties.
    pub fn with_json(content: JsonLanguage) -> Self {
        Self {
            content: ContentParser::Json(content),
        }
    }

    /// Wrap a Lezer grammar parser without discarding its strictness or node properties.
    pub fn with_lezer(content: LezerLanguage) -> Self {
        Self {
            content: ContentParser::Lezer(content),
        }
    }

    /// Wrap a WAST parser without discarding its strictness or node properties.
    pub fn with_wast(content: WastLanguage) -> Self {
        Self {
            content: ContentParser::Wast(content),
        }
    }

    /// Use the supplied YAML parser for the body. The front matter still
    /// uses the stock permissive parser. Both mounts share the YAML data facet.
    pub fn with_yaml(content: YamlLanguage) -> Self {
        Self {
            content: ContentParser::Yaml(content),
        }
    }

    /// Parse the content even if the body is empty. Without front matter,
    /// the content parser receives the entire document.
    ///
    /// # Errors
    ///
    /// Return a tokenization error or strict parse error from the body parser.
    /// A strict parse error uses the original document's byte offset.
    pub fn parse(&self, text: Text) -> Result<YamlFrontmatterDocument, SyntaxUpdateError> {
        let frontmatter = YamlFrontmatter::parse(&text);
        let body_start = frontmatter.as_ref().map_or(0, |front| front.body.start);
        let body = self.content.parse(&text, body_start)?;
        Ok(YamlFrontmatterDocument {
            text,
            frontmatter,
            body_start,
            body: Arc::new(body),
            content: self.content.clone(),
            editor_offsets: OnceLock::new(),
            wrapper: OnceLock::new(),
        })
    }
}

/// A document with an optional YAML mount and a required content mount.
/// The wrapper's positions are UTF-8 byte offsets in the original source.
/// The content parser's tree has positions relative to the start of the body.
pub struct YamlFrontmatterDocument {
    text: Text,
    frontmatter: Option<YamlFrontmatter>,
    body_start: usize,
    body: Arc<SupportedDocument>,
    content: ContentParser,
    editor_offsets: OnceLock<EditorOffsets>,
    wrapper: OnceLock<Result<SyntaxTree, ParseError>>,
}

struct MountUpdate {
    text: Text,
    frontmatter: Option<YamlFrontmatter>,
    body_start: usize,
    local_changes: Option<ChangeSet>,
}

fn body_error(error: SyntaxUpdateError, start: usize) -> SyntaxUpdateError {
    match error {
        SyntaxUpdateError::StrictSyntax(error) => {
            SyntaxUpdateError::StrictSyntax(error.map_offset(|at| start + at))
        }
        _ => error,
    }
}

impl YamlFrontmatterDocument {
    /// Apply edits in original byte coordinates. Redetect the delimiters.
    /// Reuse each mount when its previous start maps to the new start and
    /// the parser can use the unchanged text. An error leaves both mounts intact.
    pub fn apply_changes(&mut self, changes: &ChangeSet) -> Result<(), SyntaxUpdateError> {
        if changes.is_empty() {
            return Ok(());
        }
        let update = self.prepare_update(changes)?;
        if let (Some(local), Some(body)) = (&update.local_changes, Arc::get_mut(&mut self.body)) {
            body.apply_changes(local)
                .map_err(|error| body_error(error, update.body_start))?;
        } else {
            self.body = self.updated_body(&update)?;
        }
        debug_assert_eq!(
            Some(self.body.text().as_str()),
            update.text.slice(update.body_start..update.text.len())
        );
        self.editor_offsets.take();
        self.wrapper.take();
        self.text = update.text;
        self.frontmatter = update.frontmatter;
        self.body_start = update.body_start;
        Ok(())
    }

    fn reparse(&self, changes: &ChangeSet) -> Result<Self, SyntaxUpdateError> {
        let update = self.prepare_update(changes)?;
        let body = self.updated_body(&update)?;
        debug_assert_eq!(
            Some(body.text().as_str()),
            update.text.slice(update.body_start..update.text.len())
        );
        Ok(Self {
            text: update.text,
            frontmatter: update.frontmatter,
            body_start: update.body_start,
            body,
            content: self.content.clone(),
            editor_offsets: OnceLock::new(),
            wrapper: OnceLock::new(),
        })
    }

    fn prepare_update(&self, changes: &ChangeSet) -> Result<MountUpdate, SyntaxUpdateError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let previous = self
            .frontmatter
            .as_ref()
            .zip(changes.changes().next().map(|(range, _)| range.start));
        let frontmatter = YamlFrontmatter::parse_with_prefix(&text, previous);
        let body_start = frontmatter.as_ref().map_or(0, |front| front.body.start);
        let local_changes = if changes.map_pos(self.body_start, Assoc::Before) == body_start
            && !changes
                .changes()
                .any(|(range, _)| range.start < self.body_start && self.body_start < range.end)
        {
            let local = changes
                .changes()
                .filter(|(range, _)| range.start >= self.body_start)
                .map(|(range, insert)| {
                    Change::new(
                        range.start - self.body_start..range.end - self.body_start,
                        insert,
                    )
                });
            Some(ChangeSet::new(local.collect())?)
        } else {
            None
        };
        Ok(MountUpdate {
            text,
            frontmatter,
            body_start,
            local_changes,
        })
    }

    fn updated_body(
        &self,
        update: &MountUpdate,
    ) -> Result<Arc<SupportedDocument>, SyntaxUpdateError> {
        match &update.local_changes {
            Some(local) if local.is_empty() => Ok(Arc::clone(&self.body)),
            Some(local) => self
                .body
                .reparse(local)
                .map(Arc::new)
                .map_err(|error| body_error(error, update.body_start)),
            None => self
                .content
                .parse(&update.text, update.body_start)
                .map(Arc::new),
        }
    }

    /// Checkpoint reuse in the content mount, with its original body-relative bytes.
    pub fn body_reuse_stats(&self) -> ReuseStats {
        self.body.reuse_stats()
    }
    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Read the outer `Document`, `Frontmatter`, and `Body` syntax, including
    /// wrapper errors. Ranges use original bytes. Mounted trees remain separate.
    /// This query builds the outer tree once per snapshot.
    pub fn wrapper_tree(&self) -> Result<&SyntaxTree, ParseError> {
        self.wrapper
            .get_or_init(|| tree::parse(self.text.as_str()))
            .as_ref()
            .map_err(|&error| error)
    }

    /// The delimiters and parsed YAML, with absolute source ranges.
    pub fn frontmatter(&self) -> Option<&YamlFrontmatter> {
        self.frontmatter.as_ref()
    }

    /// The absolute source range of the content mount.
    pub fn body_range(&self) -> Range<usize> {
        self.body_start..self.text.len()
    }

    /// The body syntax. Add `body_range().start` to map its positions to the source.
    pub fn body(&self) -> &NativeSyntax {
        self.body.syntax()
    }

    /// Get the delimiter, YAML, and body highlights in source order, clipped to
    /// the requested range. Return `None` for an invalid UTF-8 byte range.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        if !self.text.is_valid_range(range.clone()) {
            return None;
        }
        let front = match &self.frontmatter {
            Some(frontmatter) => Some(frontmatter.highlights(range.clone())?),
            None => None,
        };
        let offset = self.body_start;
        let body = if range.end > offset && !range.is_empty() {
            let local = range.start.max(offset) - offset..range.end - offset;
            Some(self.body.highlights(local)?)
        } else {
            None
        };
        Some(
            front
                .into_iter()
                .flatten()
                .chain(body.into_iter().flatten().map(move |span| HighlightSpan {
                    range: span.range.start + offset..span.range.end + offset,
                    tags: span.tags,
                })),
        )
    }

    /// Query YAML or body indentation at an original byte position. The
    /// wrapper supplies column zero on its delimiter lines and projection gaps.
    /// Overrides use host byte offsets, including in the body parser.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        if !self.text.is_char_boundary(position) || splits_crlf(self.text.as_str(), position) {
            return None;
        }
        if let Some(frontmatter) = &self.frontmatter {
            let yaml = frontmatter.yaml();
            let range = yaml.root().range();
            if range.start < position && position < range.end
                || position == range.end && yaml.flow_at(position).is_some()
            {
                return yaml.indentation(position, settings, options);
            }
            if position <= self.body_start {
                return Some(0);
            }
        }
        if let NativeSyntax::Yaml(body) = self.body()
            && (position == self.body_start
                || position == self.text.len()
                    && body.flow_at(position - self.body_start).is_none())
        {
            // The wrapper supplies the indent outside the mount, except when
            // an unfinished flow node extends to the end of the document.
            return Some(0);
        }
        let indentation = |at| options.override_indentation?(self.body_start + at);
        self.body().indentation(
            position - self.body_start,
            settings,
            IndentOptions {
                override_indentation: Some(&indentation),
                simulate_break: options
                    .simulate_break
                    .and_then(|at| at.checked_sub(self.body_start)),
                ..options
            },
        )
    }

    /// Get a fold from the parser for the supplied logical line. The delimiters
    /// have no fold of their own.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        if !self.text.is_valid_range(line_start..line_end) {
            return None;
        }
        let offset = self.body_start;
        if line_start >= offset {
            let range = self
                .body()
                .foldable(line_start - offset, line_end - offset)?;
            return Some(range.start + offset..range.end + offset);
        }
        let yaml = self.frontmatter.as_ref()?.yaml();
        let range = yaml.root().range();
        if line_start < range.start || line_end > range.end {
            return None;
        }
        yaml.foldable(line_start, line_end)
    }
}
