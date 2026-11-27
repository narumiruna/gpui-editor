//! Word categories and language data from `@codemirror/state` 6.7.6.

use std::{ops::Range, sync::Arc};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Assoc, EditorState, Extension,
    language::{LanguageData, language_data_provider},
};

/// Supply extra word characters at a UTF-8 byte position. The first provider
/// that returns a value wins. Return `None` outside its scope. An all-space
/// cluster remains a boundary. This extension does not change regex `\b`.
pub fn word_characters(
    provider: impl Fn(&EditorState, usize) -> Option<Arc<str>> + Send + Sync + 'static,
) -> Extension {
    language_data_provider(move |state, position, _| {
        provider(state, position)
            .into_iter()
            .map(|characters| Arc::new(LanguageData::default().with("wordChars", characters)))
            .collect()
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharCategory {
    Word,
    Space,
    Other,
}

/// A snapshot of the word rule at one document position. Use it to classify
/// each grapheme cluster in a word operation.
#[derive(Clone, Debug, Default)]
pub struct CharCategorizer {
    extra: Option<Arc<str>>,
}

impl CharCategorizer {
    /// The pinned helpers sometimes classify one UTF-16 unit, not a scalar.
    pub(crate) fn is_word_unit(&self, unit: u16) -> bool {
        if let Some(ch) = char::from_u32(unit.into()) {
            self.category(ch.encode_utf8(&mut [0; 4])) == CharCategory::Word
        } else {
            // Only a configured extra character can match a surrogate half.
            self.extra
                .as_deref()
                .is_some_and(|extra| extra.encode_utf16().any(|part| part == unit))
        }
    }

    pub fn category(&self, cluster: &str) -> CharCategory {
        if cluster
            .chars()
            .all(|ch| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
        {
            CharCategory::Space
        } else if cluster.chars().any(|ch| ch.is_alphanumeric() || ch == '_')
            || self
                .extra
                .as_deref()
                .is_some_and(|extra| matches_extra(cluster, extra))
        {
            CharCategory::Word
        } else {
            CharCategory::Other
        }
    }
}

fn matches_extra(cluster: &str, extra: &str) -> bool {
    if extra.is_ascii() || cluster.is_ascii() {
        return cluster
            .bytes()
            .any(|byte| byte.is_ascii() && extra.as_bytes().contains(&byte));
    }
    // Upstream matches individual UTF-16 units across the whole cluster.
    // Two different astral scalars can share either half of a surrogate pair.
    cluster
        .encode_utf16()
        .any(|unit| extra.encode_utf16().any(|candidate| candidate == unit))
}

impl EditorState {
    /// Resolve word characters at a byte position. Use default word rules at
    /// an invalid position, without calling a provider.
    pub fn char_categorizer(&self, position: usize) -> CharCategorizer {
        let extra = self
            .language_data_objects(position, Assoc::Before)
            .into_iter()
            .find_map(|data| {
                data.get::<Arc<str>>("wordChars")
                    .cloned()
                    .or_else(|| data.get::<&str>("wordChars").map(|value| Arc::from(*value)))
                    .or_else(|| {
                        data.get::<String>("wordChars")
                            .map(|value| Arc::from(value.as_str()))
                    })
            });
        CharCategorizer { extra }
    }

    /// Return the word around the given byte offset on one logical line, or
    /// `None` when no adjacent cluster has category `Word`. Return `None` at
    /// an invalid UTF-8 offset or inside a CRLF ending.
    pub fn word_range_at(&self, position: usize) -> Option<Range<usize>> {
        let line = self.text().line_at(position)?;
        let local = position.checked_sub(line.range.start)?;
        if local > line.text.len() {
            return None;
        }
        let cat = self.char_categorizer(position);
        let text = line.text;
        let mut clusters = ClusterBreaks::new(text);
        let (mut start, mut end) = (local, local);
        while start > 0 {
            let prev = clusters.previous(start, true);
            if cat.category(&text[prev..start]) != CharCategory::Word {
                break;
            }
            start = prev;
        }
        while end < text.len() {
            let next = clusters.next(end, true);
            if cat.category(&text[end..next]) != CharCategory::Word {
                break;
            }
            end = next;
        }
        (start != end).then_some(line.range.start + start..line.range.start + end)
    }
}

/// Find the next cluster break in the selected direction. Return the string
/// edge if there is no break in that direction. Reject invalid byte offsets.
pub fn find_cluster_break(
    text: &str,
    at: usize,
    direction: Assoc,
    include_extending: bool,
) -> Option<usize> {
    text.is_char_boundary(at).then(|| {
        let mut clusters = ClusterBreaks::new(text);
        match direction {
            Assoc::Before => clusters.previous(at, include_extending),
            Assoc::After => clusters.next(at, include_extending),
        }
    })
}

/// Reuse the count at an adjacent flag boundary during one scan of a string.
pub(crate) struct ClusterBreaks<'a> {
    text: &'a str,
    regional: Option<(usize, usize)>,
}

impl<'a> ClusterBreaks<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text,
            regional: None,
        }
    }

    pub(crate) fn next(&mut self, at: usize, include_extending: bool) -> usize {
        let text = self.text;
        let mut chars = text[at..].char_indices();
        let Some((_, mut left)) = chars.next() else {
            return at;
        };
        for (offset, right) in chars {
            if !self.joins(at + offset, left, right, include_extending) {
                return at + offset;
            }
            left = right;
        }
        text.len()
    }

    fn previous(&mut self, at: usize, include_extending: bool) -> usize {
        // Upstream probes forward until a break falls strictly before the
        // input. At scalar boundaries we can find it by scanning backward.
        let mut chars = self.text[..at].char_indices().rev();
        let Some((mut at, mut right)) = chars.next() else {
            return 0;
        };
        for (offset, left) in chars {
            if !self.joins(at, left, right, include_extending) {
                return at;
            }
            (at, right) = (offset, left);
        }
        0
    }

    fn joins(&mut self, at: usize, left: char, right: char, include_extending: bool) -> bool {
        const ZWJ: char = '\u{200d}';
        left == ZWJ
            || right == ZWJ
            || include_extending && is_extending(right)
            || (is_regional_indicator(right) && is_regional_indicator(left) && self.flag_pair(at))
    }

    fn flag_pair(&mut self, at: usize) -> bool {
        // Each regional indicator uses four UTF-8 bytes. Two such boundaries
        // one scalar apart belong to the same run.
        let count = match self.regional {
            Some((last, count)) if last == at => count,
            Some((last, count)) if at.checked_sub(4) == Some(last) => count + 1,
            Some((last, count)) if last.checked_sub(4) == Some(at) => count - 1,
            _ => self.text[..at]
                .chars()
                .rev()
                .take_while(|&ch| is_regional_indicator(ch))
                .count(),
        };
        self.regional = Some((at, count));
        !count.is_multiple_of(2)
    }
}

fn is_regional_indicator(ch: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&ch)
}

fn is_extending(ch: char) -> bool {
    // find-cluster-break 1.0.4 uses Extend and SpacingMark from Unicode 17,
    // with only code points below U+30000. After a plain dot, these are the
    // only classes that attach, apart from ZWJ (already handled by the caller).
    if !('\u{300}'..'\u{30000}').contains(&ch) {
        return false;
    }
    let mut bytes = [b'.'; 5];
    let end = 1 + ch.encode_utf8(&mut bytes[1..]).len();
    let pair = str::from_utf8(&bytes[..end]).expect("dot and one Unicode scalar");
    pair.graphemes(true).next() == Some(pair)
}
