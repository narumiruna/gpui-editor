//! Section transforms adapted from CodeMirror 6 state 6.7.6, `src/change.ts`.
//! See `docs/licenses/codemirror-state.txt` for the upstream MIT notice.

use std::{ops::Range, sync::Arc};

use super::{Assoc, Change, ChangeScope, ChangeSet, EditError, MapMode, ResolvedSpec};

/// How a change touches an old-document range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeTouch {
    None,
    Touch,
    Cover,
}

/// The old and new extents of changes, without the replacement text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeDesc {
    sections: Vec<Section<usize>>,
    length: usize,
    new_length: usize,
}

impl ChangeDesc {
    /// Create a description from old-document ranges and replacement byte lengths.
    pub fn new(length: usize, mut changes: Vec<(Range<usize>, usize)>) -> Result<Self, EditError> {
        changes.sort_by_key(|(range, _)| (range.start, range.end));
        let sections = from_ranges(length, changes.iter().map(|(range, size)| (range, *size)))?;
        Self::from_sections(length, sections)
    }

    fn from_sections(length: usize, sections: Vec<Section<usize>>) -> Result<Self, EditError> {
        let new_length = output_length(&sections)?;
        Ok(Self {
            sections,
            length,
            new_length,
        })
    }

    pub(crate) fn from_section_lengths(
        sections: Vec<(usize, Option<usize>)>,
    ) -> Result<Self, EditError> {
        let length = sections.iter().try_fold(0usize, |length, (old, _)| {
            length.checked_add(*old).ok_or(EditError::InvalidRange)
        })?;
        let sections = sections
            .into_iter()
            .map(|(old, insert)| Section { old, insert })
            .collect();
        Self::from_sections(length, sections)
    }

    pub(crate) fn section_lengths(&self) -> impl Iterator<Item = (usize, Option<usize>)> + '_ {
        self.sections
            .iter()
            .map(|section| (section.old, section.insert))
    }

    pub fn length(&self) -> usize {
        self.length
    }

    pub fn new_length(&self) -> usize {
        self.new_length
    }

    /// The canonical empty description has no sections or one unchanged
    /// section. A decoded wire description can retain split unchanged parts.
    pub fn is_empty(&self) -> bool {
        sections_are_empty(&self.sections)
    }

    /// Return the corresponding old and new ranges for sections without changes.
    pub fn gaps(&self) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
        self.ranges(false)
    }

    /// Return the corresponding old and new ranges for individual changes.
    pub fn changed_ranges(&self) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
        self.ranges(true)
    }

    fn ranges(&self, changed: bool) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
        self.sections
            .iter()
            .scan((0, 0), move |(old, new), section| {
                let ends = (*old + section.old, *new + section.result_len());
                let range = (*old..ends.0, *new..ends.1);
                (*old, *new) = ends;
                Some((section.insert.is_some() == changed).then_some(range))
            })
            .flatten()
    }

    pub fn touches_range(&self, range: Range<usize>) -> RangeTouch {
        for (old, _) in self
            .changed_ranges()
            .take_while(|(old, _)| old.start <= range.end)
        {
            if old.end >= range.start {
                return if old.start < range.start && old.end > range.end {
                    RangeTouch::Cover
                } else {
                    RangeTouch::Touch
                };
            }
        }
        RangeTouch::None
    }

    /// Map a position. Return `None` if it is out of range or its tracked context was deleted.
    pub fn map_pos(&self, pos: usize, assoc: Assoc, mode: MapMode) -> Option<usize> {
        if pos > self.length {
            return None;
        }
        let mut old = 0;
        let mut new = 0;
        for section in &self.sections {
            let end = old + section.old;
            if let Some(insert) = section.insert {
                if mode.removes(pos, old, end) {
                    return None;
                }
                if end > pos || section.old == 0 && pos == end && assoc == Assoc::Before {
                    return Some(if pos == old || assoc == Assoc::Before {
                        new
                    } else {
                        new + insert
                    });
                }
            } else if end > pos {
                return Some(new + (pos - old));
            }
            old = end;
            new += section.result_len();
        }
        Some(new)
    }

    pub fn inverted(&self) -> Self {
        let sections = self
            .sections
            .iter()
            .map(|section| match section.insert {
                Some(insert) => Section {
                    old: insert,
                    insert: Some(section.old),
                },
                None => section.clone(),
            })
            .collect();
        Self {
            sections,
            length: self.new_length,
            new_length: self.length,
        }
    }

    /// Describe these changes followed by `after`.
    pub fn compose(&self, after: &Self) -> Result<Self, EditError> {
        if self.new_length != after.length {
            return Err(EditError::MismatchedLength);
        }
        if self.is_empty() {
            return Ok(after.clone());
        }
        if after.is_empty() {
            return Ok(self.clone());
        }
        Self::from_sections(self.length, compose(&self.sections, &after.sections)?)
    }

    /// Rebase these changes onto a concurrent change description from the same document.
    pub fn map(&self, over: &Self, before: bool) -> Result<Self, EditError> {
        if self.length != over.length {
            return Err(EditError::MismatchedLength);
        }
        if over.is_empty() {
            return Ok(self.clone());
        }
        Self::from_sections(
            over.new_length,
            rebase(&self.sections, &over.sections, before)?,
        )
    }
}

impl ChangeSet {
    pub(crate) fn from_specs<'a>(
        specs: impl IntoIterator<Item = ResolvedSpec<'a>>,
        length: usize,
    ) -> Result<Self, EditError> {
        let mut total = Vec::new();
        let mut group = Vec::new();
        let mut end = 0;
        let mut bound = false;
        for spec in specs {
            let Change { range, insert } = match spec {
                ResolvedSpec::Edit(change) => change,
                ResolvedSpec::Set(set) => {
                    flush_specs(length, &mut total, &mut group, end)?;
                    append_specs(&mut total, set.sections(length, str::to_owned)?)?;
                    bound |= set.scope.is_some();
                    end = 0;
                    continue;
                }
            };
            if range.start > range.end || range.end > length {
                return Err(EditError::InvalidRange);
            }
            if range.is_empty() && insert.is_empty() {
                continue;
            }
            if range.start < end {
                flush_specs(length, &mut total, &mut group, end)?;
                end = 0;
            }
            push(&mut group, range.start - end, None, false)?;
            push(&mut group, range.len(), Some(insert), false)?;
            end = range.end;
        }
        flush_specs(length, &mut total, &mut group, end)?;
        Self::from_sections(total, bound)
    }

    pub fn desc(&self, length: usize) -> Result<ChangeDesc, EditError> {
        ChangeDesc::from_sections(length, self.sections(length, str::len)?)
    }

    /// Combine these changes with changes measured in their resulting document.
    pub fn compose(&self, after: &Self, length: usize) -> Result<Self, EditError> {
        let first = self.sections(length, str::to_owned)?;
        let second = after.sections(output_length(&first)?, str::to_owned)?;
        let bound = self.scope.is_some() || after.scope.is_some();
        if self.is_empty() || after.is_empty() {
            let result = if self.is_empty() { after } else { self }.clone();
            return Ok(if bound {
                result.bind_length(length)
            } else {
                result
            });
        }
        Self::from_sections(compose(&first, &second)?, bound)
    }

    /// Rebase onto changes measured in the same document. `before` gives this set priority for ties.
    /// An empty description preserves the existing edit boundaries.
    pub fn map(&self, over: &ChangeDesc, before: bool) -> Result<Self, EditError> {
        if over.is_empty() {
            self.desc(over.length)?;
            return Ok(self.clone());
        }
        self.map_desc(over, before)
    }

    /// Rebase like [`Self::map`], but run the section transform even when the
    /// description is empty. The transform can join adjacent edit boundaries.
    pub fn map_desc(&self, over: &ChangeDesc, before: bool) -> Result<Self, EditError> {
        let first = self.sections(over.length, str::to_owned)?;
        Self::from_sections(
            rebase(&first, &over.sections, before)?,
            self.scope.is_some(),
        )
    }

    fn sections<T: Payload>(
        &self,
        length: usize,
        payload: impl Fn(&str) -> T,
    ) -> Result<Vec<Section<T>>, EditError> {
        self.check_length(length)?;
        let mut sections = Vec::new();
        let mut splits = self.split_gaps().iter().copied().peekable();
        let mut start = 0;
        for (range, text) in self.changes() {
            if range.end > length {
                return Err(EditError::InvalidRange);
            }
            append_gaps(&mut sections, &mut splits, start, range.start);
            sections.push(Section {
                old: range.len(),
                insert: Some(payload(text)),
            });
            start = range.end;
        }
        append_gaps(&mut sections, &mut splits, start, length);
        output_length(&sections)?;
        Ok(sections)
    }

    pub(crate) fn from_wire_sections(
        sections: Vec<(usize, Option<String>)>,
    ) -> Result<Self, EditError> {
        Self::from_sections(
            sections
                .into_iter()
                .map(|(old, insert)| Section { old, insert })
                .collect(),
            true,
        )
    }

    fn from_sections(sections: Vec<Section<String>>, bound: bool) -> Result<Self, EditError> {
        output_length(&sections)?;
        let mut start = 0usize;
        let mut changes = Vec::new();
        let mut split_gaps = Vec::new();
        let mut previous_gap = false;
        for section in sections {
            let end = start
                .checked_add(section.old)
                .ok_or(EditError::InvalidRange)?;
            if let Some(text) = section.insert {
                changes.push(Change::new(start..end, text));
                previous_gap = false;
            } else {
                if previous_gap {
                    split_gaps.push(start);
                }
                previous_gap = true;
            }
            start = end;
        }
        debug_assert!(bound || split_gaps.is_empty());
        Ok(Self {
            changes,
            scope: bound.then(|| {
                Arc::new(ChangeScope {
                    length: start,
                    split_gaps,
                })
            }),
        })
    }
}

fn append_gaps<T>(
    sections: &mut Vec<Section<T>>,
    splits: &mut std::iter::Peekable<impl Iterator<Item = usize>>,
    mut start: usize,
    end: usize,
) {
    while let Some(split) = splits.next_if(|&pos| pos < end) {
        debug_assert!(start < split);
        sections.push(Section {
            old: split - start,
            insert: None,
        });
        start = split;
    }
    if start < end {
        sections.push(Section {
            old: end - start,
            insert: None,
        });
    }
}

fn flush_specs(
    length: usize,
    total: &mut Vec<Section<String>>,
    group: &mut Vec<Section<String>>,
    end: usize,
) -> Result<(), EditError> {
    if group.is_empty() {
        return Ok(());
    }
    push(group, length - end, None, false)?;
    append_specs(total, std::mem::take(group))
}

fn append_specs(
    total: &mut Vec<Section<String>>,
    next: Vec<Section<String>>,
) -> Result<(), EditError> {
    if sections_are_empty(total) {
        *total = next;
    } else {
        let mapped = rebase(&next, total, false)?;
        if !sections_are_empty(&mapped) {
            *total = compose(total, &mapped)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Section<T> {
    old: usize,
    insert: Option<T>,
}

fn sections_are_empty<T>(sections: &[Section<T>]) -> bool {
    matches!(sections, [] | [Section { insert: None, .. }])
}

impl<T: Payload> Section<T> {
    fn result_len(&self) -> usize {
        self.insert.as_ref().map_or(self.old, Payload::size)
    }
}

// Transform descriptions with byte lengths, and change sets with their text.
trait Payload: Clone + Default {
    fn size(&self) -> usize;
    fn part(&self, range: Range<usize>) -> Result<Self, EditError>;
    fn append(&mut self, other: Self) -> Result<(), EditError>;
}

impl Payload for usize {
    fn size(&self) -> usize {
        *self
    }

    fn part(&self, range: Range<usize>) -> Result<Self, EditError> {
        Ok(range.len())
    }

    fn append(&mut self, other: Self) -> Result<(), EditError> {
        *self = self.checked_add(other).ok_or(EditError::InvalidRange)?;
        Ok(())
    }
}

impl Payload for String {
    fn size(&self) -> usize {
        self.len()
    }

    fn part(&self, range: Range<usize>) -> Result<Self, EditError> {
        self.get(range)
            .map(str::to_owned)
            .ok_or(EditError::InvalidRange)
    }

    fn append(&mut self, other: Self) -> Result<(), EditError> {
        self.push_str(&other);
        Ok(())
    }
}

fn output_length<T: Payload>(sections: &[Section<T>]) -> Result<usize, EditError> {
    sections.iter().try_fold(0usize, |length, section| {
        length
            .checked_add(section.result_len())
            .ok_or(EditError::InvalidRange)
    })
}

fn from_ranges<'a, T: Payload>(
    length: usize,
    ranges: impl IntoIterator<Item = (&'a Range<usize>, T)>,
) -> Result<Vec<Section<T>>, EditError> {
    let mut result = Vec::new();
    let mut end = 0;
    for (range, insert) in ranges {
        if range.start > range.end || range.end > length {
            return Err(EditError::InvalidRange);
        }
        if range.start < end {
            return Err(EditError::OverlappingChanges);
        }
        push(&mut result, range.start - end, None, false)?;
        push(&mut result, range.len(), Some(insert), false)?;
        end = range.end;
    }
    push(&mut result, length - end, None, false)?;
    output_length(&result)?;
    Ok(result)
}

fn push<T: Payload>(
    result: &mut Vec<Section<T>>,
    old: usize,
    insert: Option<T>,
    join: bool,
) -> Result<(), EditError> {
    let size = insert.as_ref().map(Payload::size);
    if old == 0 && size.unwrap_or(0) == 0 {
        return Ok(());
    }
    if let Some(last) = result.last_mut() {
        let last_size = last.insert.as_ref().map(Payload::size);
        if size == last_size && size.unwrap_or(0) == 0 || old == 0 && last.old == 0 || join {
            last.old = last.old.checked_add(old).ok_or(EditError::InvalidRange)?;
            if let Some(insert) = insert {
                last.insert.get_or_insert_with(T::default).append(insert)?;
            }
            return Ok(());
        }
    }
    result.push(Section { old, insert });
    Ok(())
}

struct Cursor<'a, T> {
    sections: &'a [Section<T>],
    index: usize,
    old: usize,
    insert: Option<usize>,
    offset: usize,
}

impl<'a, T: Payload> Cursor<'a, T> {
    fn new(sections: &'a [Section<T>]) -> Self {
        let mut cursor = Self {
            sections,
            index: 0,
            old: 0,
            insert: None,
            offset: 0,
        };
        cursor.load();
        cursor
    }

    fn load(&mut self) {
        let section = self.sections.get(self.index);
        self.old = section.map_or(0, |section| section.old);
        self.insert = section.and_then(|section| section.insert.as_ref().map(Payload::size));
        self.offset = 0;
    }

    fn next(&mut self) {
        self.index += 1;
        self.load();
    }

    fn done(&self) -> bool {
        self.index == self.sections.len()
    }

    fn result_len(&self) -> usize {
        self.insert.unwrap_or(self.old)
    }

    fn payload(&self) -> Option<&'a T> {
        self.sections.get(self.index)?.insert.as_ref()
    }

    fn advance(&mut self, len: usize) {
        if len == self.old {
            self.next();
        } else {
            self.old -= len;
            self.offset += len;
        }
    }

    fn advance_result(&mut self, len: usize) {
        match self.insert {
            None => self.advance(len),
            Some(insert) if len == insert => self.next(),
            Some(insert) => {
                self.insert = Some(insert - len);
                self.offset += len;
            }
        }
    }
}

fn compose<T: Payload>(
    first: &[Section<T>],
    second: &[Section<T>],
) -> Result<Vec<Section<T>>, EditError> {
    let mut result = Vec::new();
    let mut a = Cursor::new(first);
    let mut b = Cursor::new(second);
    let mut join = false;
    loop {
        if a.done() && b.done() {
            return Ok(result);
        }
        if a.insert == Some(0) {
            push(&mut result, a.old, Some(T::default()), join)?;
            a.next();
        } else if b.old == 0 && !b.done() {
            push(&mut result, 0, b.payload().cloned(), join)?;
            b.next();
        } else if a.done() || b.done() {
            return Err(EditError::MismatchedLength);
        } else {
            let len = a.result_len().min(b.old);
            let count = result.len();
            let b_insert = || {
                b.payload().map(|text| {
                    if b.offset == 0 {
                        text.clone()
                    } else {
                        T::default()
                    }
                })
            };
            if let Some(payload) = a.payload() {
                let old = if a.offset == 0 { a.old } else { 0 };
                let insert = if b.insert.is_none() {
                    Some(payload.part(a.offset..a.offset + len)?)
                } else {
                    b_insert()
                };
                push(&mut result, old, insert, join)?;
            } else {
                push(&mut result, len, b_insert(), join)?;
            }
            join = (a.insert.is_some_and(|size| size > len) || b.insert.is_some() && b.old > len)
                && (join || result.len() > count);
            a.advance_result(len);
            b.advance(len);
        }
    }
}

fn rebase<T: Payload, U: Payload>(
    first: &[Section<T>],
    second: &[Section<U>],
    before: bool,
) -> Result<Vec<Section<T>>, EditError> {
    let mut result = Vec::new();
    let mut a = Cursor::new(first);
    let mut b = Cursor::new(second);
    let mut inserted = None;
    loop {
        if a.done() && b.done() {
            return Ok(result);
        }
        if a.done() && b.old > 0 || b.done() && a.old > 0 {
            return Err(EditError::MismatchedLength);
        }
        if a.insert.is_none() && b.insert.is_none() {
            let len = a.old.min(b.old);
            push(&mut result, len, None, false)?;
            a.advance(len);
            b.advance(len);
        } else if let Some(size) = b.insert
            && (a.insert.is_none()
                || inserted == Some(a.index)
                || a.offset == 0 && (b.old < a.old || b.old == a.old && !before))
        {
            let mut left = b.old;
            push(&mut result, size, None, false)?;
            while left > 0 {
                if a.done() {
                    return Err(EditError::MismatchedLength);
                }
                let len = a.old.min(left);
                if a.insert.is_some() && inserted != Some(a.index) && a.old <= len {
                    push(&mut result, 0, a.payload().cloned(), false)?;
                    inserted = Some(a.index);
                }
                a.advance(len);
                left -= len;
            }
            b.next();
        } else if let Some(payload) = a.payload() {
            let mut len = 0;
            let mut left = a.old;
            while left > 0 {
                match b.insert {
                    None if !b.done() => {
                        let part = left.min(b.old);
                        len += part;
                        left -= part;
                        b.advance(part);
                    }
                    Some(0) if b.old < left => {
                        left -= b.old;
                        b.next();
                    }
                    _ => break,
                }
            }
            let payload = if inserted == Some(a.index) {
                T::default()
            } else {
                payload.clone()
            };
            push(&mut result, len, Some(payload), false)?;
            inserted = Some(a.index);
            a.advance(a.old - left);
        } else {
            return Err(EditError::MismatchedLength);
        }
    }
}
