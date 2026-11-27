use std::{ops::Range, sync::Arc};

use regex::Regex;

use crate::{ChangeDesc, EditorState};

use super::super::{Completion, CompletionSection, SectionRank};

/// Owned completion data that can be sent across threads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub display_label: Option<String>,
    pub sort_text: Option<String>,
    pub detail: Option<String>,
    pub kind: Option<String>,
    pub info: Option<String>,
    pub insert: Option<String>,
    pub boost: i32,
    pub section: Option<(String, SectionRank)>,
    /// If present, override the source's commit characters.
    pub commit_characters: Option<Vec<char>>,
}

impl CompletionItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            display_label: None,
            sort_text: None,
            detail: None,
            kind: None,
            info: None,
            insert: None,
            boost: 0,
            section: None,
            commit_characters: None,
        }
    }

    pub fn as_completion(&self) -> Completion<'_> {
        Completion {
            label: &self.label,
            display_label: self.display_label.as_deref(),
            sort_text: self.sort_text.as_deref(),
            detail: self.detail.as_deref(),
            kind: self.kind.as_deref(),
            info: self.info.as_deref(),
            insert: self.insert.as_deref(),
            boost: self.boost,
            section: self
                .section
                .as_ref()
                .map(|(name, rank)| CompletionSection { name, rank: *rank }),
        }
    }
}

impl From<Completion<'_>> for CompletionItem {
    fn from(item: Completion<'_>) -> Self {
        Self {
            label: item.label.to_owned(),
            display_label: item.display_label.map(str::to_owned),
            sort_text: item.sort_text.map(str::to_owned),
            detail: item.detail.map(str::to_owned),
            kind: item.kind.map(str::to_owned),
            info: item.info.map(str::to_owned),
            insert: item.insert.map(str::to_owned),
            boost: item.boost,
            section: item
                .section
                .map(|section| (section.name.to_owned(), section.rank)),
            commit_characters: None,
        }
    }
}

type MatchRanges =
    dyn Fn(&CompletionItem, Option<&[Range<usize>]>) -> Vec<Range<usize>> + Send + Sync;
type ValidFor = dyn Fn(&str, Range<usize>, &EditorState) -> bool + Send + Sync;
type MapResult = dyn Fn(&SourceResult, &ChangeDesc) -> Option<SourceResult> + Send + Sync;
type UpdateResult =
    dyn Fn(&SourceResult, Range<usize>, &EditorState) -> Option<SourceResult> + Send + Sync;

/// One language source's result. Offsets refer to the query's immutable state.
#[derive(Clone)]
pub struct SourceResult {
    pub from: usize,
    pub to: Option<usize>,
    pub options: Arc<[CompletionItem]>,
    pub filter: bool,
    pub commit_characters: Vec<char>,
    valid_for: Option<Arc<ValidFor>>,
    pub(super) match_ranges: Option<Arc<MatchRanges>>,
    pub(super) map: Option<Arc<MapResult>>,
    pub(super) update: Option<Arc<UpdateResult>>,
}

impl SourceResult {
    pub fn new(from: usize, options: impl Into<Arc<[CompletionItem]>>) -> Self {
        Self {
            from,
            to: None,
            options: options.into(),
            filter: true,
            commit_characters: Vec::new(),
            valid_for: None,
            match_ranges: None,
            map: None,
            update: None,
        }
    }

    /// Keep the result after typing or deletion while the whole query text matches.
    pub fn with_valid_for_regex(self, pattern: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(&format!(r"\A(?:{pattern})\z"))?;
        Ok(self.with_valid_for(move |value, _, _| regex.is_match(value)))
    }

    pub fn with_valid_for(
        mut self,
        valid: impl Fn(&str, Range<usize>, &EditorState) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.valid_for = Some(Arc::new(valid));
        self
    }

    /// Supply highlight ranges for unfiltered options (`None` input), or map
    /// the label matches of a filtered option with a nonempty display label
    /// (`Some` input). Return UTF-8 byte ranges in the displayed text. The
    /// callback can run each time the session ranks the options.
    pub fn with_match_ranges(
        mut self,
        ranges: impl Fn(&CompletionItem, Option<&[Range<usize>]>) -> Vec<Range<usize>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.match_ranges = Some(Arc::new(ranges));
        self
    }

    /// Map any source-owned configuration through changes. Returning `None`
    /// invalidates the result. The session maps the active replacement range.
    pub fn with_map(
        mut self,
        map: impl Fn(&SourceResult, &ChangeDesc) -> Option<SourceResult> + Send + Sync + 'static,
    ) -> Self {
        self.map = Some(Arc::new(map));
        self
    }

    /// Update an invalid result synchronously before requesting the source again.
    pub fn with_update(
        mut self,
        update: impl Fn(&SourceResult, Range<usize>, &EditorState) -> Option<SourceResult>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.update = Some(Arc::new(update));
        self
    }

    pub(super) fn valid_in(&self, state: &EditorState, range: Range<usize>) -> bool {
        let Some(text) = state.text().slice(range.clone()) else {
            return false;
        };
        self.filter
            && self
                .valid_for
                .as_ref()
                .is_some_and(|check| check(text, range, state))
    }
}
