use std::{
    fmt,
    iter::FusedIterator,
    ops::Range,
    sync::{Arc, OnceLock},
};

use crate::EditError;

mod iter;
mod tree;

use iter::{Chunks, Lines};
pub use iter::{TextCursor, TextLineCursor};
use tree::Tree;

/// A persistent UTF-8 document. LF and CRLF end logical lines. A lone CR is content.
///
/// Positions and lengths use UTF-8 bytes, not CodeMirror's UTF-16 code units.
/// Checked ranges do not clip as CodeMirror's `slice` and `replace` do. Equality
/// compares all bytes. Clones share a snapshot. Edits can share unchanged subtrees.
///
/// CodeMirror's `EditorState` splits document and edit strings on CRLF, LF or
/// lone CR by default. Its `lineSeparator` facet uses the first configured value.
/// A nonempty value sets the exact string to split on and the separator for
/// `sliceDoc`. CodeMirror's `Text.toString()` and cursors still use LF.
///
/// Native `Text` has no separator option. [`Self::new`] keeps all bytes and
/// always treats LF and CRLF as endings. String conversion keeps these endings.
/// For explicit output conversion, use [`Self::slice_with_line_separator`].
#[derive(Clone, Default)]
pub struct Text(Arc<Snapshot>);

#[derive(Default)]
struct Snapshot {
    tree: Tree,
    flat: OnceLock<String>,
}

/// A one-based logical line. The byte range and text exclude the line ending.
#[derive(Debug, Eq, PartialEq)]
pub struct Line<'a> {
    pub number: usize,
    pub range: Range<usize>,
    pub text: &'a str,
}

impl Text {
    pub fn new(source: impl Into<String>) -> Self {
        Self::from_tree(Tree::new(source.into()))
    }

    fn from_tree(tree: Tree) -> Self {
        Self(Arc::new(Snapshot {
            tree,
            flat: OnceLock::new(),
        }))
    }

    /// Join the given lines with LF. Unlike CodeMirror's `Text.of`, this accepts
    /// an empty array and returns an empty buffer.
    ///
    /// The joined bytes determine the logical lines. This does not validate the
    /// line content. To keep the original separators, use [`Self::new`] with the
    /// original string instead.
    pub fn from_lines(lines: &[&str]) -> Self {
        Self::new(lines.join("\n"))
    }

    /// Borrow the whole document. A multi-chunk snapshot caches the contiguous string.
    pub fn as_str(&self) -> &str {
        self.0.tree.contiguous().unwrap_or_else(|| {
            self.0
                .flat
                .get_or_init(|| self.slice_string(0..self.len()).unwrap())
        })
    }

    /// The length in UTF-8 bytes, including line endings.
    pub fn len(&self) -> usize {
        self.0.tree.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of logical lines. An empty buffer has one empty line.
    pub fn line_count(&self) -> usize {
        self.0.tree.breaks() + 1
    }

    pub(crate) fn is_char_boundary(&self, position: usize) -> bool {
        position == self.len()
            || self
                .0
                .tree
                .chunk_at(position)
                .is_some_and(|chunk| chunk.is_char_boundary(position))
    }

    pub(crate) fn is_valid_range(&self, range: Range<usize>) -> bool {
        range.start <= range.end
            && self.is_char_boundary(range.start)
            && (range.is_empty() || self.is_char_boundary(range.end))
    }

    /// Borrow a byte range. Return `None` for an invalid range or UTF-8 boundary.
    pub fn slice(&self, range: Range<usize>) -> Option<&str> {
        self.is_valid_range(range.clone())
            .then(|| self.borrow(range))
    }

    /// Copy a byte range, replacing each complete LF or CRLF ending with `separator`.
    /// Preserve a partial CRLF at either endpoint and keep lone CR as content.
    /// An empty separator removes complete endings. Use the same checks as [`Self::slice`].
    ///
    /// This is the output counterpart to CodeMirror's `sliceString` separator
    /// argument and `EditorState.sliceDoc`. It does not change the document or
    /// set a separator for input, edits, or later output.
    pub fn slice_with_line_separator(
        &self,
        range: Range<usize>,
        separator: &str,
    ) -> Option<String> {
        self.is_valid_range(range.clone())
            .then(|| iter::slice_with_line_separator(self, range, separator))
    }

    fn borrow(&self, range: Range<usize>) -> &str {
        self.chunk_slice(range.clone())
            .unwrap_or_else(|| &self.as_str()[range])
    }

    fn chunk_slice(&self, range: Range<usize>) -> Option<&str> {
        if range.is_empty() {
            Some("")
        } else {
            self.0.tree.chunk_at(range.start)?.slice(range)
        }
    }

    /// Return the requested bytes as a new document that can share storage.
    /// This uses the same checks as [`Self::slice`].
    pub fn slice_owned(&self, range: Range<usize>) -> Option<Self> {
        if !self.is_valid_range(range.clone()) {
            return None;
        }
        if range == (0..self.len()) {
            return Some(self.clone());
        }
        Some(Self::from_tree(self.0.tree.subtree(range)))
    }

    /// Copy only the requested range, without caching a contiguous document.
    pub(crate) fn slice_string(&self, range: Range<usize>) -> Option<String> {
        if !self.is_valid_range(range.clone()) {
            return None;
        }
        if let Some(chunk) = self.chunk_slice(range.clone()) {
            return Some(chunk.to_owned());
        }
        let mut result = String::with_capacity(range.len());
        for chunk in self.0.tree.chunks(range) {
            result.push_str(chunk);
        }
        Some(result)
    }

    pub(crate) fn range_eq(&self, range: Range<usize>, expected: &str) -> bool {
        if range.len() != expected.len() || !self.is_valid_range(range.clone()) {
            return false;
        }
        if let Some(chunk) = self.chunk_slice(range.clone()) {
            return chunk == expected;
        }
        let mut remaining = expected.as_bytes();
        for chunk in self.0.tree.chunks(range) {
            let Some(tail) = remaining.strip_prefix(chunk.as_bytes()) else {
                return false;
            };
            remaining = tail;
        }
        true
    }

    /// Return a new buffer with a byte range replaced. Do not change the original.
    ///
    /// # Errors
    ///
    /// Return [`EditError::InvalidRange`] for a reversed or out-of-bounds range,
    /// or a range with an endpoint inside a UTF-8 character. Do not clip the range.
    /// Return [`EditError::LengthOverflow`] if the result would exceed `isize::MAX` bytes.
    pub fn replace(&self, range: Range<usize>, insert: impl AsRef<str>) -> Result<Self, EditError> {
        if !self.is_valid_range(range.clone()) {
            return Err(EditError::InvalidRange);
        }
        let insert = insert.as_ref();
        Self::checked_edit_len(self.len(), range.len(), insert.len())?;
        let mut text = self.clone();
        text.replace_range(range, insert);
        Ok(text)
    }

    /// Replace a byte range with an existing document. Share unaffected
    /// storage from both inputs without converting the insert to a string.
    /// Return [`EditError::InvalidRange`] under the same rules as [`Self::replace`].
    /// Return [`EditError::LengthOverflow`] if the result would exceed `isize::MAX` bytes.
    pub fn replace_text(&self, range: Range<usize>, insert: &Self) -> Result<Self, EditError> {
        if !self.is_valid_range(range.clone()) {
            return Err(EditError::InvalidRange);
        }
        Self::checked_edit_len(self.len(), range.len(), insert.len())?;
        if range.is_empty() && insert.is_empty() {
            return Ok(self.clone());
        }
        if range == (0..self.len()) {
            return Ok(insert.clone());
        }
        Ok(Self::from_tree(
            self.0.tree.replace_tree(range, insert.0.tree.clone()),
        ))
    }

    pub(crate) fn checked_edit_len(
        len: usize,
        removed: usize,
        insert: usize,
    ) -> Result<usize, EditError> {
        (len - removed)
            .checked_add(insert)
            .filter(|len| *len <= isize::MAX as usize)
            .ok_or(EditError::LengthOverflow)
    }

    /// Replace a byte range that the caller has validated.
    pub(crate) fn replace_range(&mut self, range: Range<usize>, insert: &str) {
        debug_assert!(self.is_valid_range(range.clone()));
        if !self.range_eq(range.clone(), insert) {
            *self = Self::from_tree(self.0.tree.replace(range, insert));
        }
    }

    /// Return the concatenated bytes in a new buffer. Do not insert a separator.
    ///
    /// # Panics
    ///
    /// The result cannot exceed `isize::MAX` bytes. For a checked splice, use
    /// [`Self::append_text`] or [`Self::replace`].
    pub fn append(&self, other: impl AsRef<str>) -> Self {
        let end = self.len();
        self.replace(end..end, other)
            .expect("length of appended text")
    }

    /// Join two documents without inserting a separator or flattening either one.
    /// Return [`EditError::LengthOverflow`] if the result would exceed `isize::MAX` bytes.
    pub fn append_text(&self, other: &Self) -> Result<Self, EditError> {
        let end = self.len();
        self.replace_text(end..end, other)
    }

    /// Get a logical line by its one-based number, or `None` if it does not exist.
    pub fn line(&self, number: usize) -> Option<Line<'_>> {
        if !(1..=self.line_count()).contains(&number) {
            return None;
        }
        let range = self.line_start(number)?..self.line_end(number);
        Some(Line {
            number,
            text: self.borrow(range.clone()),
            range,
        })
    }

    /// The line at a byte boundary. Both bytes of CRLF belong to the preceding line.
    pub fn line_at(&self, position: usize) -> Option<Line<'_>> {
        let breaks = if position == self.len() {
            self.0.tree.breaks()
        } else {
            let chunk = self.0.tree.chunk_at(position)?;
            if !chunk.is_char_boundary(position) {
                return None;
            }
            chunk.breaks_before(position)
        };
        self.line(breaks + 1)
    }

    fn line_start(&self, number: usize) -> Option<usize> {
        match number {
            0 => None,
            1 => Some(0),
            _ => self.0.tree.break_at(number - 2).map(|lf| lf + 1),
        }
    }

    fn line_end(&self, number: usize) -> usize {
        self.0
            .tree
            .break_at(number - 1)
            .map_or(self.len(), |lf| self.ending_start(lf))
    }

    fn ending_start(&self, lf: usize) -> usize {
        lf - usize::from(
            lf.checked_sub(1)
                .and_then(|cr| self.0.tree.chunk_at(cr).map(|chunk| chunk.byte(cr)))
                == Some(b'\r'),
        )
    }

    /// Start a forward cursor over nonempty line content and line endings.
    pub fn cursor(&self) -> TextCursor<'_> {
        TextCursor::new(self, 0, self.len())
    }

    /// Start a cursor between two byte offsets. If `from > to`, walk in reverse
    /// chunk order without reversing bytes. Equal endpoints give no chunks.
    /// Both offsets must be UTF-8 boundaries in the document. An offset inside
    /// CRLF is valid and limits the line-ending chunk to the included byte.
    ///
    /// This is the directional counterpart to CodeMirror's `iterRange`. For a
    /// double-ended Rust iterator over an ascending range, use [`Self::iter_range`].
    pub fn cursor_range(&self, from: usize, to: usize) -> Option<TextCursor<'_>> {
        self.is_valid_range(from.min(to)..from.max(to))
            .then(|| TextCursor::new(self, from, to))
    }

    /// Iterate over nonempty line content and LF or CRLF endings as separate chunks.
    /// Join the chunks in forward order to recover the original bytes.
    /// Use `rev()` to reverse the order of the chunks, not their bytes.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &str> + FusedIterator {
        Chunks::new(self, 0..self.len())
    }

    /// Iterate over a byte range with the same checks as [`Self::slice`].
    /// Use `rev()` for reverse order. An endpoint inside CRLF limits the
    /// line-ending chunk to the included byte.
    pub fn iter_range(
        &self,
        range: Range<usize>,
    ) -> Option<impl DoubleEndedIterator<Item = &str> + FusedIterator> {
        self.is_valid_range(range.clone())
            .then(|| Chunks::new(self, range))
    }

    /// Start a forward cursor over line content, including any final empty line.
    /// This is the signed byte-skip counterpart to CodeMirror's `iterLines()`.
    /// For a double-ended iterator over complete lines, use [`Self::iter_lines`].
    pub fn line_cursor(&self) -> TextLineCursor<'_> {
        TextLineCursor::new(self, None)
    }

    /// Start a signed byte-skip cursor over a one-based range of lines.
    /// The end line is excluded and may be `line_count() + 1`. The start
    /// must be an existing line and the range must be ascending or empty.
    /// An empty range yields one empty item, as in CodeMirror. Once the
    /// cursor reaches the end of the range it cannot rewind.
    pub fn line_cursor_range(&self, range: Range<usize>) -> Option<TextLineCursor<'_>> {
        let lines = self.line_count();
        if range.start > range.end || range.end > lines + 1 || range.start > lines {
            return None;
        }
        let start = self.line_start(range.start)?;
        let end = if range.is_empty() {
            start
        } else {
            self.line_end(range.end - 1)
        };
        Some(TextLineCursor::new(self, Some(start..end)))
    }

    /// Iterate over line content without endings, including any final empty line.
    pub fn iter_lines(
        &self,
    ) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator + FusedIterator {
        Lines::new(self, 1..self.line_count() + 1)
    }

    /// Iterate over a one-based, end-exclusive range of logical lines without endings.
    /// Both endpoints must be in `1..=line_count() + 1` and in ascending order.
    /// Empty lines yield `""`. Equal endpoints yield no items, unlike CodeMirror's
    /// `iterLines` cursor, which can yield an empty string for a collapsed range.
    pub fn iter_line_range(
        &self,
        range: Range<usize>,
    ) -> Option<impl DoubleEndedIterator<Item = &str> + ExactSizeIterator + FusedIterator> {
        (range.start > 0 && range.start <= range.end && range.end <= self.line_count() + 1)
            .then(|| Lines::new(self, range))
    }

    /// Collect the same borrowed line content as [`Self::iter_lines`].
    /// Unlike CodeMirror's `toJSON`, rebuilding from these lines can lose
    /// information: the strings do not record the original line endings.
    pub fn to_lines(&self) -> Vec<&str> {
        self.iter_lines().collect()
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        if self.len() != other.len() || self.line_count() != other.line_count() {
            return false;
        }
        self.0.tree.same_bytes(&other.0.tree)
    }
}

impl Eq for Text {}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Text")
            .field("len", &self.len())
            .field("line_count", &self.line_count())
            .finish_non_exhaustive()
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = self
            .0
            .tree
            .contiguous()
            .or_else(|| self.0.flat.get().map(String::as_str))
        {
            return f.pad(source);
        }
        if f.width().is_some() || f.precision().is_some() {
            return f.pad(&self.slice_string(0..self.len()).unwrap());
        }
        for chunk in self.0.tree.chunks(0..self.len()) {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
