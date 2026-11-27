use std::{collections::HashSet, sync::Arc};

use crate::{
    Text,
    autocomplete::session::{CompletionItem, SourceResult},
};

use super::{
    super::{Bias, SqlDocument, SqlLanguage, SqlNodeKind::*},
    word,
};

/// A fixed list of keyword, type, and builtin completions for a SQL dialect.
#[derive(Clone, Debug)]
pub struct SqlKeywordCompletionSource {
    options: Arc<[CompletionItem]>,
    pattern: Arc<Prefix>,
}

/// Build a keyword source. Each default option has a boost of -1.
pub fn keyword_completion_source(
    language: &SqlLanguage,
    upper_case: bool,
) -> SqlKeywordCompletionSource {
    SqlKeywordCompletionSource::with_builder(language, upper_case, |label, kind| {
        let mut item = CompletionItem::new(label);
        item.kind = Some(kind.to_owned());
        item.boost = -1;
        item
    })
}

impl SqlKeywordCompletionSource {
    /// Build one option per dialect word. The returned labels determine which
    /// prefixes trigger completion. Uppercasing occurs before calling `build`.
    pub fn with_builder(
        language: &SqlLanguage,
        upper_case: bool,
        mut build: impl FnMut(&str, &str) -> CompletionItem,
    ) -> Self {
        let options: Arc<[_]> = language
            .words()
            .iter()
            .map(|entry| {
                let label = if upper_case {
                    &entry.label.to_uppercase()
                } else {
                    &entry.label
                };
                build(label, entry.completion_type())
            })
            .collect();
        let pattern = Arc::new(Prefix::new(&options));
        Self { options, pattern }
    }

    /// Return unfiltered options at a valid UTF-8 byte position. Syntax in a
    /// document with parse errors uses the native parser's recovery tree.
    pub fn complete(
        &self,
        doc: &SqlDocument,
        position: usize,
        explicit: bool,
    ) -> Option<SourceResult> {
        let text = doc.text();
        if !text.is_char_boundary(position)
            || doc.resolve(position, Bias::Left).ancestors().any(|node| {
                matches!(
                    node.kind(),
                    QuotedIdentifier | String | LineComment | BlockComment | Dot
                )
            })
        {
            return None;
        }
        let from = match self.pattern.match_before(text, position) {
            Some(start) => start,
            None if explicit => position,
            None => return None,
        };
        let pattern = self.pattern.clone();
        Some(
            SourceResult::new(from, self.options.clone())
                .with_valid_for(move |value, _, _| pattern.valid(value)),
        )
    }
}

#[derive(Debug)]
enum Prefix {
    Word,
    Custom { first: CharSet, tail: CharSet },
}

impl Prefix {
    fn new(options: &[CompletionItem]) -> Self {
        if options
            .iter()
            .all(|item| !item.label.is_empty() && item.label.encode_utf16().all(word))
        {
            return Self::Word;
        }
        let (mut first, mut tail) = (CharSet::default(), CharSet::default());
        for item in options {
            let mut units = item.label.encode_utf16();
            // The upstream character-set builder uses label[0] as an object
            // key. An empty label contributes the key "undefined" (a word).
            first.add(units.next().unwrap_or(b'u'.into()));
            for unit in units {
                tail.add(unit);
            }
        }
        Self::Custom { first, tail }
    }

    fn first(&self, unit: u16) -> bool {
        match self {
            Self::Word => word(unit),
            Self::Custom { first, .. } => first.contains(unit),
        }
    }

    fn tail(&self, unit: u16) -> bool {
        match self {
            Self::Word => word(unit),
            Self::Custom { tail, .. } => tail.contains(unit),
        }
    }

    fn valid(&self, value: &str) -> bool {
        let mut units = value.encode_utf16();
        match self {
            Self::Word => units.all(word),
            Self::Custom { .. } => {
                units.next().is_some_and(|unit| self.first(unit))
                    && units.all(|unit| self.tail(unit))
            }
        }
    }

    fn match_before(&self, text: &Text, position: usize) -> Option<usize> {
        let line = text.line_at(position)?;
        let end = position.min(line.range.end) - line.range.start;
        let mut starts = None;
        let mut remaining = 250;
        // Search backward for the earliest start with a valid tail. CodeMirror
        // bounds the search in UTF-16 units. A match that starts in the middle
        // of a surrogate pair starts at that scalar's byte boundary here.
        for (byte, ch) in line.text[..end].char_indices().rev() {
            for &unit in ch.encode_utf16(&mut [0; 2]).iter().rev() {
                if remaining == 0 {
                    return starts;
                }
                remaining -= 1;
                if self.first(unit) {
                    starts = Some(line.range.start + byte);
                }
                if !self.tail(unit) {
                    return starts;
                }
            }
        }
        starts
    }
}

#[derive(Debug, Default)]
struct CharSet {
    word: bool,
    other: HashSet<u16>,
}

impl CharSet {
    fn add(&mut self, unit: u16) {
        if word(unit) {
            self.word = true;
        } else {
            self.other.insert(unit);
        }
    }

    fn contains(&self, unit: u16) -> bool {
        (self.word && word(unit)) || self.other.contains(&unit)
    }
}
