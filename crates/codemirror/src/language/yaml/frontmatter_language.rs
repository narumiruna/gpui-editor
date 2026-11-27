//! Native `yamlFrontmatter` wrapping based on `@codemirror/lang-yaml` 6.1.3.
//! See `docs/licenses/codemirror-lang-yaml.txt`.
//!
//! The wrapper uses the existing YAML delimiter scan. It does not expose Lezer
//! wrapper error nodes or reproduce its recovery around U+FFFF. It accepts a
//! native parser and can forward native editor extensions.

use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, Change, ChangeSet, Text,
    language::{HighlightSpan, lezer::ReuseStats},
    language_data::{NativeLanguage, NativeSyntax, SupportedDocument, SyntaxUpdateError},
    stream_parser::TokenizeError,
};

use super::YamlFrontmatter;

mod state;

/// Parse optional YAML front matter and mount the selected parser in the body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YamlFrontmatterLanguage {
    content: NativeLanguage,
}

impl YamlFrontmatterLanguage {
    pub const fn new(content: NativeLanguage) -> Self {
        Self { content }
    }

    /// Parse the content even if the body is empty. Without front matter,
    /// the content parser receives the entire document.
    ///
    /// # Errors
    ///
    /// Return a tokenization error from the selected content parser.
    pub fn parse(&self, text: Text) -> Result<YamlFrontmatterDocument, TokenizeError> {
        let frontmatter = YamlFrontmatter::parse(&text);
        let body_range = frontmatter
            .as_ref()
            .map_or(0..text.len(), YamlFrontmatter::body);
        let body = self
            .content
            .support()
            .parse(text.slice_owned(body_range.clone()).unwrap())?;
        Ok(YamlFrontmatterDocument {
            text,
            frontmatter,
            body_start: body_range.start,
            body: Arc::new(body),
            language: self.content,
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
    language: NativeLanguage,
}

struct MountUpdate {
    text: Text,
    frontmatter: Option<YamlFrontmatter>,
    body_start: usize,
    local_changes: Option<ChangeSet>,
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
            body.apply_changes(local)?;
        } else {
            self.body = self.updated_body(&update)?;
        }
        debug_assert_eq!(
            Some(self.body.text().as_str()),
            update.text.slice(update.body_start..update.text.len())
        );
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
            language: self.language,
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
            Some(local) => Ok(Arc::new(self.body.reparse(local)?)),
            None => self
                .language
                .support()
                .parse(
                    update
                        .text
                        .slice_owned(update.body_start..update.text.len())
                        .unwrap(),
                )
                .map(Arc::new)
                .map_err(SyntaxUpdateError::Tokenize),
        }
    }

    /// Checkpoint reuse in the content mount, with its original body-relative bytes.
    pub fn body_reuse_stats(&self) -> ReuseStats {
        self.body.reuse_stats()
    }
    pub fn text(&self) -> &Text {
        &self.text
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
