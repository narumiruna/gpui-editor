use std::ops;

use serde_json::{Value, json};

use crate::Text;

use super::LspError;

const MAX_UINTEGER: u32 = i32::MAX as u32;

/// A zero-based line and a UTF-16 code unit offset within that line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// A half-open range in LSP coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl TryFrom<&Value> for Position {
    type Error = LspError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let invalid = LspError::InvalidFormat("Position");
        let field = |name| {
            value
                .get(name)
                .and_then(Value::as_u64)
                .and_then(|number| u32::try_from(number).ok())
                .filter(|&number| number <= MAX_UINTEGER)
                .ok_or(invalid.clone())
        };
        Ok(Self {
            line: field("line")?,
            character: field("character")?,
        })
    }
}

impl TryFrom<&Value> for Range {
    type Error = LspError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let invalid = LspError::InvalidFormat("Range");
        Ok(Self {
            start: value
                .get("start")
                .ok_or_else(|| invalid.clone())?
                .try_into()?,
            end: value.get("end").ok_or(invalid)?.try_into()?,
        })
    }
}

impl Range {
    pub(super) fn to_json(self) -> Value {
        let position = |pos: Position| json!({"line": pos.line, "character": pos.character});
        json!({"start": position(self.start), "end": position(self.end)})
    }
}

/// An index of one text snapshot for converting between byte and LSP positions.
///
/// LSP lines end at LF, CRLF, or a lone CR. This includes a final empty line
/// after a line ending. Unlike [`Text`], the index treats a lone CR as an ending.
#[derive(Debug)]
pub struct PositionIndex<'a> {
    source: &'a str,
    lines: Vec<ops::Range<usize>>,
}

impl<'a> PositionIndex<'a> {
    pub fn new(text: &'a Text) -> Self {
        let source = text.as_str();
        let mut breaks = source.match_indices(['\r', '\n']).peekable();
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some((at, ending)) = breaks.next() {
            lines.push(start..at);
            start = at + 1;
            if ending == "\r" && breaks.peek() == Some(&(start, "\n")) {
                breaks.next();
                start += 1;
            }
        }
        lines.push(start..source.len());
        Self { source, lines }
    }

    /// Return a UTF-8 byte offset. Reject columns past the end of a line or
    /// inside a surrogate pair. Do not clamp a position.
    pub fn offset(&self, position: Position) -> Result<usize, LspError> {
        let invalid = LspError::InvalidPosition(position);
        if position.line > MAX_UINTEGER || position.character > MAX_UINTEGER {
            return Err(invalid);
        }
        let Some(line) = self.lines.get(position.line as usize) else {
            return Err(invalid);
        };
        let mut remaining = position.character as usize;
        for (offset, ch) in self.source[line.clone()].char_indices() {
            if remaining == 0 {
                return Ok(line.start + offset);
            }
            let Some(rest) = remaining.checked_sub(ch.len_utf16()) else {
                return Err(invalid);
            };
            remaining = rest;
        }
        if remaining == 0 {
            Ok(line.end)
        } else {
            Err(invalid)
        }
    }

    /// Return an LSP position. Reject offsets inside a CRLF or a UTF-8 character.
    pub fn position(&self, offset: usize) -> Result<Position, LspError> {
        let number = self.lines.partition_point(|line| line.start <= offset) - 1;
        let line = &self.lines[number];
        let prefix = self.source[line.clone()]
            .get(..offset - line.start)
            .ok_or(LspError::InvalidByteOffset(offset))?;
        let coordinate = |value| {
            u32::try_from(value)
                .ok()
                .filter(|&number| number <= MAX_UINTEGER)
                .ok_or(LspError::PositionOverflow)
        };
        Ok(Position {
            line: coordinate(number)?,
            character: coordinate(prefix.encode_utf16().count())?,
        })
    }
}
