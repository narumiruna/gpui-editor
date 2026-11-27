use std::ops::{Range, RangeInclusive};

use gpui_editor_codemirror::view::bidi::{BidiIsolate, Direction};

use super::{MAX_SOURCE_BYTES, TextKitError};

const MAX_ISOLATES: usize = 1024;
// Each isolate can add two embedding levels. UAX #9 permits levels up to 125.
const MAX_ISOLATE_DEPTH: usize = 62;

/// The display string inserts controls but never removes or rewrites source.
/// Each source boundary maps to all display boundaries before, between, and
/// after any controls at that byte. Surrogate interiors have no reverse entry.
pub(super) struct Projection {
    pub(super) text: String,
    source: Box<str>,
    source_units: Vec<(usize, RangeInclusive<usize>)>,
    display_bytes: Vec<Option<usize>>,
}

impl Projection {
    pub(super) fn new(source: &str, isolates: &[BidiIsolate]) -> Result<Self, TextKitError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(TextKitError::Limit);
        }
        // Source embeddings and isolates can change or consume synthetic
        // boundaries. Do not rewrite source controls to prevent it.
        if source.is_empty()
            || (!isolates.is_empty()
                && source
                    .chars()
                    .any(|ch| matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')))
        {
            return Err(TextKitError::UnsupportedSource);
        }
        let mut events = Vec::new();
        Self::isolate_events(source, 0..source.len(), isolates, 0, &mut events)?;
        let mut events = events.into_iter().peekable();
        let mut result = Self {
            text: String::with_capacity(source.len() + events.len() * 3),
            source: source.into(),
            source_units: Vec::new(),
            display_bytes: vec![Some(0)],
        };
        for (byte, ch) in source
            .char_indices()
            .map(|(byte, ch)| (byte, Some(ch)))
            .chain([(source.len(), None)])
        {
            let first = result.units();
            while let Some(&(_, control)) = events.peek().filter(|(at, _)| *at == byte) {
                result.append(control, byte);
                events.next();
            }
            result.source_units.push((byte, first..=result.units()));
            if let Some(ch) = ch {
                result.append(ch, byte + ch.len_utf8());
            }
        }
        debug_assert!(events.next().is_none());
        Ok(result)
    }

    fn isolate_events(
        source: &str,
        parent: Range<usize>,
        isolates: &[BidiIsolate],
        depth: usize,
        events: &mut Vec<(usize, char)>,
    ) -> Result<(), TextKitError> {
        let mut previous = parent.start;
        for isolate in isolates {
            if depth >= MAX_ISOLATE_DEPTH || events.len() + depth >= 2 * MAX_ISOLATES {
                return Err(TextKitError::Limit);
            }
            let range = isolate.from..isolate.to;
            if range.is_empty() || range.start < previous || range.end > parent.end {
                return Err(TextKitError::InvalidIsolate);
            }
            let text = source
                .get(range.clone())
                .ok_or(TextKitError::InvalidIsolate)?;
            if text.chars().any(|ch| {
                matches!(
                    ch,
                    '\r' | '\n' | '\u{000b}' | '\u{000c}' | '\u{001c}'
                        ..='\u{001e}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
                )
            }) {
                return Err(TextKitError::InvalidIsolate);
            }
            events.push((
                range.start,
                match isolate.direction {
                    None => '\u{2068}',
                    Some(Direction::Ltr) => '\u{2066}',
                    Some(Direction::Rtl) => '\u{2067}',
                },
            ));
            previous = range.end;
            Self::isolate_events(source, range, &isolate.inner, depth + 1, events)?;
            events.push((previous, '\u{2069}'));
        }
        Ok(())
    }

    fn append(&mut self, ch: char, byte_after: usize) {
        self.text.push(ch);
        if ch.len_utf16() == 2 {
            self.display_bytes.push(None);
        }
        self.display_bytes.push(Some(byte_after));
    }

    pub(super) fn source(&self) -> &str {
        &self.source
    }

    pub(super) fn units(&self) -> usize {
        self.display_bytes.len() - 1
    }

    pub(super) fn byte_at(&self, utf16: usize) -> Option<usize> {
        self.display_bytes.get(utf16).copied().flatten()
    }

    pub(super) fn units_at(&self, byte: usize) -> Option<RangeInclusive<usize>> {
        let i = self
            .source_units
            .binary_search_by_key(&byte, |&(b, _)| b)
            .ok()?;
        Some(self.source_units[i].1.clone())
    }

    /// Exclude controls at both ends. Controls inside the range are invisible
    /// and do not need a separate attribute or selection range.
    pub(super) fn display_range(&self, bytes: Range<usize>) -> Option<Range<usize>> {
        if bytes.start > bytes.end {
            return None;
        }
        let start = *self.units_at(bytes.start)?.end();
        let end = *self.units_at(bytes.end)?.start();
        Some(start..end.max(start))
    }

    pub(super) fn is_crlf_middle(&self, byte: usize) -> bool {
        byte > 0
            && byte < self.source.len()
            && self.source.as_bytes().get(byte - 1..byte + 1) == Some(b"\r\n")
    }
}
