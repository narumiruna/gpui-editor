mod bounded;
mod folded_input;
mod translate;

use std::{iter::FusedIterator, ops::Range};

use regex::{Captures, Regex, RegexBuilder};

use super::{
    CursorOptions, SearchError, SearchMatch, check_range, filter::MatchFilter, take_units,
};
use crate::Text;
use folded_input::FoldedInput;
use translate::translate;

const FIRST_CHUNK_UNITS: usize = 5000;
const CHUNK_END_MARGIN_UNITS: usize = 10;

#[derive(Clone, Debug)]
pub(super) struct RegExpPattern {
    matcher: Matcher,
    multiline: bool,
    fold_word_boundary: bool,
}

impl RegExpPattern {
    pub(super) fn new(pattern: &str, ignore_case: bool) -> Result<Self, SearchError> {
        // CodeMirror tests the raw source, not whether a parsed atom can match
        // a line terminator. For example, `\n` switches modes but `\x0A` does not.
        let multiline = pattern.contains(['\n', '\r'])
            || pattern.as_bytes().windows(2).any(|pair| {
                matches!(
                    pair,
                    [b'\\', b's' | b'W' | b'D' | b'n' | b'r'] | [b'[', b'^']
                )
            });
        let (matcher, fold_word_boundary) = match translate(pattern) {
            Ok((translated, word_boundary)) => {
                let regex = RegexBuilder::new(&translated)
                    .multi_line(true)
                    .crlf(true)
                    .case_insensitive(ignore_case)
                    .build()
                    .map_err(|err| SearchError::InvalidRegExp(err.to_string()))?;
                (Matcher::Linear(regex), ignore_case && word_boundary)
            }
            Err(err) => match bounded::compile(pattern, ignore_case) {
                Ok(regex) => (Matcher::Bounded(regex), false),
                Err(_) => return Err(err),
            },
        };
        Ok(Self {
            matcher,
            multiline,
            fold_word_boundary,
        })
    }
}

#[derive(Clone, Debug)]
enum Matcher {
    Linear(Regex),
    Bounded(regress::Regex),
}

impl Matcher {
    fn captures_at<'a>(&self, input: &'a str, offset: usize) -> Option<MatchRanges<'a>> {
        match self {
            Self::Linear(regex) => regex.captures_at(input, offset).map(MatchRanges::Linear),
            Self::Bounded(regex) => regex
                .find_from(input, offset)
                .next()
                .map(MatchRanges::Bounded),
        }
    }
}

enum MatchRanges<'a> {
    Linear(Captures<'a>),
    Bounded(regress::Match),
}

impl MatchRanges<'_> {
    fn range(&self, group: usize) -> Option<Range<usize>> {
        match self {
            Self::Linear(captures) => captures.get(group).map(|matched| matched.range()),
            Self::Bounded(matched) => matched.group(group),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Linear(captures) => captures.len(),
            Self::Bounded(matched) => matched.captures.len() + 1,
        }
    }
}

/// Iterate over non-overlapping ECMAScript matches and captures in UTF-8 bytes.
/// Lookaround and backreferences require a small static work bound. Patterns
/// without them use the linear matcher.
///
/// This does not support lone surrogates or all ECMAScript group syntax.
/// With the linear matcher, anchors treat CRLF as one break and do not
/// recognize U+2028 or U+2029. Capture values inside repeated groups and
/// some Unicode property matches (including `\P` with case folding) can differ.
pub struct RegExpCursor<'a> {
    source: &'a str,
    pattern: RegExpPattern,
    limit: usize,
    input: Range<usize>,
    folded: Option<Box<FoldedInput>>,
    next_line: Option<usize>,
    position: Option<usize>,
    last_end: Option<usize>,
    filter: MatchFilter,
}

impl<'a> RegExpCursor<'a> {
    pub fn new(text: &'a Text, pattern: &str) -> Result<Self, SearchError> {
        Self::in_range(text, pattern, 0..text.len(), CursorOptions::default())
    }

    pub fn in_range(
        text: &'a Text,
        pattern: &str,
        range: Range<usize>,
        options: CursorOptions,
    ) -> Result<Self, SearchError> {
        check_range(text, &range)?;
        Ok(Self::with_pattern(
            text,
            range,
            RegExpPattern::new(pattern, options.ignore_case)?,
            MatchFilter::new(options.whole_word),
        ))
    }

    pub(super) fn with_pattern(
        text: &'a Text,
        range: Range<usize>,
        pattern: RegExpPattern,
        filter: MatchFilter,
    ) -> Self {
        let source = text.as_str();
        let from = range.start;
        let mut cursor = Self {
            source,
            pattern,
            limit: range.end,
            input: range,
            folded: None,
            next_line: None,
            position: Some(from),
            last_end: None,
            filter,
        };
        if cursor.pattern.multiline {
            cursor.set_chunk(FIRST_CHUNK_UNITS);
        } else {
            cursor.set_line(source[..from].rfind('\n').map_or(0, |at| at + 1));
        }
        cursor
    }

    fn set_input(&mut self, range: Range<usize>) {
        self.folded = if self.pattern.fold_word_boundary {
            FoldedInput::new(&self.source[range.clone()])
        } else {
            None
        };
        self.input = range;
    }

    fn set_line(&mut self, start: usize) {
        let end = self.source[start..self.limit]
            .find('\n')
            .map_or(self.limit, |offset| start + offset);
        self.next_line = (end < self.limit).then_some(end + 1);
        self.set_input(start..self.line_end(start, end));
    }

    fn set_chunk(&mut self, units: usize) {
        let start = self.input.start;
        let source = &self.source[start..self.limit];
        // Use the line end at the UTF-16 cut, or the range end if it comes first.
        let (bytes, _) = take_units(source.chars(), units);
        let end = source[bytes..].find('\n').map_or(self.limit, |offset| {
            self.line_end(start, start + bytes + offset)
        });
        self.set_input(start..end);
    }

    fn line_end(&self, start: usize, end: usize) -> usize {
        let strip_cr = self.source.as_bytes().get(end) == Some(&b'\n')
            && self.source[start..end].ends_with('\r');
        end - usize::from(strip_cr)
    }

    fn advance_input(&mut self) {
        if self.pattern.multiline && self.input.end < self.limit {
            let units = self.source[self.input.clone()].encode_utf16().count();
            self.set_chunk(units.saturating_mul(2));
        } else {
            self.position = self.next_line;
            if let Some(start) = self.next_line {
                self.set_line(start);
            }
        }
    }

    fn captures(&self, source: &'a str, offset: usize) -> Option<MatchRanges<'_>> {
        if offset > source.len() {
            return None;
        }
        let (input, at) = self.folded.as_ref().map_or((source, offset), |folded| {
            (folded.text.as_str(), folded.folded_offset(offset))
        });
        let captures = self.pattern.matcher.captures_at(input, at)?;
        if self.pattern.multiline && captures.range(0) == Some(at..at) {
            // The second attempt moves by one UTF-16 unit. In ECMAScript u
            // mode a position inside a pair resolves to the start of that pair.
            let ch = input[at..].chars().next()?;
            let step = if ch.len_utf16() == 2 {
                0
            } else {
                ch.len_utf8()
            };
            self.pattern.matcher.captures_at(input, at + step)
        } else {
            Some(captures)
        }
    }

    fn source_range(&self, range: Range<usize>) -> Range<usize> {
        match &self.folded {
            Some(folded) => folded.source_range(range),
            None => range,
        }
    }
}

impl<'a> Iterator for RegExpCursor<'a> {
    type Item = SearchMatch<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(position) = self.position {
            let input = &self.source[self.input.clone()];
            let Some(captures) = self.captures(input, position - self.input.start) else {
                self.advance_input();
                continue;
            };
            let entire = captures.range(0).expect("capture zero is the full match");
            let range = self.source_range(entire);
            if self.pattern.multiline
                && self.input.end < self.limit
                && input[range.end..]
                    .encode_utf16()
                    .take(CHUNK_END_MARGIN_UNITS)
                    .count()
                    < CHUNK_END_MARGIN_UNITS
            {
                // A larger window may let the match extend or choose another branch.
                self.advance_input();
                continue;
            }
            let from = self.input.start + range.start;
            let to = self.input.start + range.end;
            let eligible = if from == to {
                self.pattern.multiline || self.last_end != Some(from)
            } else {
                self.filter.matches_word(input, range.clone())
            };
            let found = eligible.then(|| SearchMatch {
                range: from..to,
                precise: true,
                text: &input[range],
                captures: (0..captures.len())
                    .map(|group| {
                        captures
                            .range(group)
                            .map(|range| &input[self.source_range(range)])
                    })
                    .collect(),
            });
            if from == self.input.end && !self.pattern.multiline {
                self.advance_input();
            } else if from == to {
                self.position = self.source[to..]
                    .chars()
                    .next()
                    .map(|ch| to + ch.len_utf8());
            } else {
                self.position = Some(to);
            }
            if let Some(found) = found.filter(|found| self.filter.accepts(found)) {
                self.last_end = Some(to);
                return Some(found);
            }
        }
        None
    }
}

impl FusedIterator for RegExpCursor<'_> {}
