use std::{
    collections::{BTreeSet, HashSet},
    ops::Range,
    sync::Arc,
};

use regex::Regex;

use crate::Assoc;

use super::{CompletionItem, CompletionQuery, SourceResult};

/// A suffix match in the query document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryMatch<'a> {
    pub range: Range<usize>,
    pub text: &'a str,
}

impl CompletionQuery {
    /// Match a Rust regex directly before the caret on the same logical line.
    /// Search at most 250 UTF-16 units, rounded to a valid UTF-8 boundary.
    pub fn match_before(&self, expression: &str) -> Result<Option<QueryMatch<'_>>, regex::Error> {
        let regex = Regex::new(&format!(r"(?:{expression})\z"))?;
        Ok(self.find_before(&regex))
    }

    fn find_before(&self, regex: &Regex) -> Option<QueryMatch<'_>> {
        let (start, prefix) = self.line_prefix()?;
        let match_at = regex.find(prefix)?.start();
        Some(QueryMatch {
            range: start + match_at..start + prefix.len(),
            text: &prefix[match_at..],
        })
    }

    fn line_prefix(&self) -> Option<(usize, &str)> {
        let line = self.state().text().line_at(self.position)?;
        let end = self.position.min(line.range.end);
        let prefix = &line.text[..end - line.range.start];
        let mut units = 0;
        let start = prefix
            .char_indices()
            .rev()
            .take_while(|(_, ch)| {
                units += ch.len_utf16();
                units <= 250
            })
            .last()
            .map_or(prefix.len(), |(at, _)| at);
        Some((line.range.start + start, &prefix[start..]))
    }
}

/// The native equivalent of a fixed-list completion source. Precompiles the
/// prefix and validity expressions once for all calls.
#[derive(Clone, Debug)]
pub struct ListSource {
    options: Arc<[CompletionItem]>,
    before: Option<Regex>,
    valid_for: Option<Regex>,
}

impl ListSource {
    pub fn new(options: impl Into<Arc<[CompletionItem]>>) -> Result<Self, regex::Error> {
        let options = options.into();
        let (prefix, valid) = if options
            .iter()
            .all(|item| !item.label.is_empty() && item.label.chars().all(word))
        {
            (
                Some("[A-Za-z0-9_]+".to_owned()),
                Some("[A-Za-z0-9_]*".to_owned()),
            )
        } else {
            // The pin uses label[0] as a key. An empty label becomes
            // "undefined", which adds ASCII word characters to the first class.
            let first = charset(
                options
                    .iter()
                    .map(|item| item.label.chars().next().unwrap_or('_')),
            );
            let tail = charset(options.iter().flat_map(|item| item.label.chars().skip(1)));
            let pattern = first
                .map(|first| tail.map_or_else(|| first.clone(), |tail| format!("{first}{tail}*")));
            (pattern.clone(), pattern)
        };
        let before = prefix
            .map(|pattern| Regex::new(&format!(r"(?:{pattern})\z")))
            .transpose()?;
        let valid_for = valid
            .map(|pattern| Regex::new(&format!(r"\A(?:{pattern})\z")))
            .transpose()?;
        Ok(Self {
            options,
            before,
            valid_for,
        })
    }

    pub fn complete(&self, query: &CompletionQuery) -> Option<SourceResult> {
        let found = self
            .before
            .as_ref()
            .and_then(|before| query.find_before(before));
        if found.is_none() && !query.explicit {
            return None;
        }
        let from = found.map_or(query.position, |matched| matched.range.start);
        let result = SourceResult::new(from, self.options.clone());
        Some(match &self.valid_for {
            Some(valid) => {
                let valid = valid.clone();
                result.with_valid_for(move |value, _, _| valid.is_match(value))
            }
            None => result,
        })
    }
}

/// Complete distinct words from the document, in order of first occurrence.
/// Use Unicode alphabetic and number characters, `_`, and the first
/// `wordChars` language value at the query position. Skip the word at the
/// replacement start, but include a copy at any other position.
/// Return at most 2,000 options per query.
pub fn complete_any_word(query: &CompletionQuery) -> Option<SourceResult> {
    let state = query.state();
    let word_chars = state
        .language_data_objects(query.position, Assoc::Before)
        .into_iter()
        .find_map(|data| {
            data.get::<Arc<str>>("wordChars")
                .map(|value| regex::escape(value))
                .or_else(|| {
                    data.get::<&str>("wordChars")
                        .map(|value| regex::escape(value))
                })
                .or_else(|| {
                    data.get::<String>("wordChars")
                        .map(|value| regex::escape(value))
                })
        })
        .unwrap_or_default();
    let words = Regex::new(&format!(r"[\p{{Alphabetic}}\p{{Number}}_{word_chars}]+")).ok()?;
    let (start, prefix) = query.line_prefix()?;
    let from = match words.find_iter(prefix).last() {
        Some(word) if word.end() == prefix.len() => start + word.start(),
        _ if query.explicit => query.position,
        _ => return None,
    };

    let mut position = 0;
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    let mut doc = state.text().cursor();
    'lines: while let Some(line) = doc.next() {
        if !doc.line_break() {
            for word in words.find_iter(line) {
                if position + word.start() != from && seen.insert(word.as_str()) {
                    let mut item = CompletionItem::new(word.as_str());
                    item.kind = Some("text".into());
                    options.push(item);
                    if options.len() == 2_000 {
                        break 'lines;
                    }
                }
            }
        }
        position += line.len();
    }
    // Upstream validity is anchored at the start, not at the end.
    Some(
        SourceResult::new(from, options).with_valid_for(move |text, _, _| {
            words.find(text).is_some_and(|word| word.start() == 0)
        }),
    )
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn charset(chars: impl Iterator<Item = char>) -> Option<String> {
    let chars: BTreeSet<_> = chars.collect();
    if chars.is_empty() {
        return None;
    }
    let mut class = String::from("[");
    if chars.iter().copied().any(word) {
        class.push_str("A-Za-z0-9_");
    }
    for ch in chars.into_iter().filter(|ch| !word(*ch)) {
        class.push_str(&regex::escape(&ch.to_string()));
    }
    class.push(']');
    Some(class)
}
