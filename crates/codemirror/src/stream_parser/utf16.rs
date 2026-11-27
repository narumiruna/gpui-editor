//! UTF-16 `StringStream` compatibility for `@codemirror/streamparser` 6.0.0.
//!
//! The package exports `StringStream` at runtime and `StreamParser` only as a
//! TypeScript interface. This module implements the runtime class. It does not
//! replace the parent module's byte stream or tokenizer. The caller supplies a
//! whole line; the stream does not split it or remove line endings.
//!
//! The public `string`, `indentUnit`, `pos`, and `start` fields map to Rust
//! fields. Methods use snake case, and the `eat`, `eatWhile`, and `match`
//! overloads use separate methods. String `match` and `skipTo` return `false`
//! instead of JavaScript `null` and `undefined` on failure.
//!
//! Positions are signed integers. The indent sizes and override are nonnegative
//! integers, and the tab size must be positive. JavaScript number coercion,
//! property reflection, and mutation of private fields are not modeled.

mod regexp;

use std::{cell::Cell, num::NonZeroUsize};

pub use regexp::{RegExp, RegExpCaptures, RegExpError, RegExpMatch};

/// A cursor on one line of UTF-16 code units, including unpaired surrogates.
#[derive(Debug)]
pub struct StringStream<'line> {
    pub string: &'line [u16],
    pub indent_unit: usize,
    pub pos: isize,
    pub start: isize,
    tab_size: NonZeroUsize,
    indentation_override: Option<usize>,
    column_cache: Cell<(isize, usize)>,
}

impl<'line> StringStream<'line> {
    pub fn new(string: &'line [u16], tab_size: NonZeroUsize, indent_unit: usize) -> Self {
        Self {
            string,
            indent_unit,
            pos: 0,
            start: 0,
            tab_size,
            indentation_override: None,
            column_cache: Cell::new((0, 0)),
        }
    }

    /// Set the optional fourth constructor argument, including zero.
    pub fn with_indentation_override(mut self, column: usize) -> Self {
        self.indentation_override = Some(column);
        self
    }

    /// Set the current token's start to the cursor.
    pub fn start_token(&mut self) {
        self.start = self.pos;
    }

    pub fn eol(&self) -> bool {
        self.pos >= self.string.len() as isize
    }

    pub fn sol(&self) -> bool {
        self.pos == 0
    }

    /// Return the unit at the cursor, or `None` if there is no unit there.
    pub fn peek(&self) -> Option<&'line [u16]> {
        let pos = usize::try_from(self.pos).ok()?;
        (pos < self.string.len()).then(|| &self.string[pos..pos + 1])
    }

    /// Advance one unit unless at or past the end. A negative cursor reads an
    /// empty slice and advances, as with JavaScript `charAt`.
    #[expect(clippy::should_implement_trait, reason = "matches the upstream method")]
    pub fn next(&mut self) -> Option<&'line [u16]> {
        if self.eol() {
            return None;
        }
        let text = self.peek().unwrap_or_default();
        self.pos += 1;
        Some(text)
    }

    /// Compare the string with JavaScript `charAt(pos)`. An empty pattern
    /// consumes once when there is no unit at the cursor.
    pub fn eat(&mut self, pattern: &[u16]) -> Option<&'line [u16]> {
        let text = self.peek().unwrap_or_default();
        if text != pattern {
            return None;
        }
        self.pos = self.pos.saturating_add(1);
        Some(text)
    }

    pub fn eat_if(&mut self, predicate: impl FnOnce(u16) -> bool) -> Option<&'line [u16]> {
        let unit = self.peek().filter(|unit| predicate(unit[0]))?;
        self.pos += 1;
        Some(unit)
    }

    /// Test the expression on just the next unit. Do not test it at end of line.
    pub fn eat_regex(&mut self, pattern: &mut RegExp) -> Option<&'line [u16]> {
        self.eat_if(|unit| pattern.test(&[unit]))
    }

    pub fn eat_while(&mut self, pattern: &[u16]) -> bool {
        let start = self.pos;
        while self.eat(pattern).is_some_and(|text| !text.is_empty()) {}
        self.pos > start
    }

    pub fn eat_while_if(&mut self, mut predicate: impl FnMut(u16) -> bool) -> bool {
        let start = self.pos;
        while self.eat_if(&mut predicate).is_some() {}
        self.pos > start
    }

    pub fn eat_while_regex(&mut self, pattern: &mut RegExp) -> bool {
        self.eat_while_if(|unit| pattern.test(&[unit]))
    }

    /// Consume ECMAScript whitespace and line terminators, including BOM and NBSP.
    pub fn eat_space(&mut self) -> bool {
        self.eat_while_if(is_space)
    }

    pub fn skip_to_end(&mut self) {
        self.pos = self.string.len() as isize;
    }

    /// Move to the next occurrence of `pattern`. As with `indexOf`, the search
    /// starts at the cursor clamped to the line. Return false if not found.
    pub fn skip_to(&mut self, pattern: &[u16]) -> bool {
        let start = self.pos.clamp(0, self.string.len() as isize) as usize;
        let offset = if pattern.is_empty() {
            0
        } else if let Some(offset) = self.string[start..]
            .windows(pattern.len())
            .position(|window| window == pattern)
        {
            offset
        } else {
            return false;
        };
        self.pos = (start + offset) as isize;
        true
    }

    /// Subtract units from the cursor without clamping to the line.
    pub fn back_up(&mut self, units: isize) {
        self.pos = self.pos.saturating_sub(units);
    }

    /// Return the column at the token start. Tabs move to the next tab stop;
    /// each other code unit adds one. The upstream cache only moves forward,
    /// so moving the token start back does not reset the cached column.
    pub fn column(&self) -> usize {
        let (from, column) = self.column_cache.get();
        if self.start <= from {
            return column;
        }
        let end = self.start.min(self.string.len() as isize);
        let column = self.count_columns(
            self.string
                .get(from as usize..end as usize)
                .unwrap_or_default()
                .iter()
                .copied(),
            column,
        );
        let column = column.saturating_add((self.start - from.max(end)) as usize);
        self.column_cache.set((self.start, column));
        column
    }

    /// Return the override or the column after the leading whitespace.
    pub fn indentation(&self) -> usize {
        self.indentation_override.unwrap_or_else(|| {
            self.count_columns(
                self.string
                    .iter()
                    .copied()
                    .take_while(|&unit| is_space(unit)),
                0,
            )
        })
    }

    /// Compare the UTF-16 prefix of the same length as `pattern` (or what is
    /// left of the line). Case-insensitive comparison lowercases each string
    /// independently, including contextual sigma and multi-unit expansions.
    /// A match advances by the original pattern length when `consume` is true.
    pub fn match_str(&mut self, pattern: &[u16], consume: bool, case_insensitive: bool) -> bool {
        let suffix = self.remaining();
        let prefix = &suffix[..pattern.len().min(suffix.len())];
        let matches = if case_insensitive {
            lowercase(prefix) == lowercase(pattern)
        } else {
            prefix == pattern
        };
        if matches && consume {
            self.pos = self.pos.saturating_add(pattern.len() as isize);
        }
        matches
    }

    /// Run JavaScript-style `String.match` on the suffix. A non-global match
    /// must start at zero, but a global result has no index to check. When
    /// consuming, advance by the length of the first match only.
    pub fn match_regex(
        &mut self,
        pattern: &mut RegExp,
        consume: bool,
    ) -> Option<RegExpMatch<'line>> {
        let matched = pattern.match_string(self.remaining())?;
        if let RegExpMatch::Single(captures) = &matched
            && captures.index() > 0
        {
            return None;
        }
        if consume {
            self.pos = self.pos.saturating_add(matched.first().len() as isize);
        }
        Some(matched)
    }

    /// Return the token using JavaScript `slice(start, pos)` bounds.
    pub fn current(&self) -> &'line [u16] {
        let from = slice_index(self.start, self.string.len());
        let to = slice_index(self.pos, self.string.len()).max(from);
        &self.string[from..to]
    }

    fn remaining(&self) -> &'line [u16] {
        &self.string[slice_index(self.pos, self.string.len())..]
    }

    fn count_columns(&self, units: impl Iterator<Item = u16>, start: usize) -> usize {
        let size = self.tab_size.get();
        units.fold(start, |column, unit| {
            let width = if unit == 9 { size - column % size } else { 1 };
            column.saturating_add(width)
        })
    }
}

fn slice_index(index: isize, len: usize) -> usize {
    if index < 0 {
        len.saturating_sub(index.unsigned_abs())
    } else {
        (index as usize).min(len)
    }
}

fn is_space(unit: u16) -> bool {
    matches!(unit, 0x09..=0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff)
}

fn lowercase(units: &[u16]) -> Vec<u16> {
    let mut result = Vec::with_capacity(units.len());
    let mut text = String::new();
    for decoded in char::decode_utf16(units.iter().copied()) {
        match decoded {
            Ok(scalar) => text.push(scalar),
            Err(error) => {
                // A lone surrogate is unchanged and ends any casing context.
                result.extend(text.to_lowercase().encode_utf16());
                result.push(error.unpaired_surrogate());
                text.clear();
            }
        }
    }
    result.extend(text.to_lowercase().encode_utf16());
    result
}
