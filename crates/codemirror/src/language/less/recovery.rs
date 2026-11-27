//! Bounded repairs for incomplete editing input. No repair changes the source.

use std::ops::Range;

use crate::language::lezer::{ParseError, SyntaxTree};

use super::parser::parse_tree;

const MAX_REPAIRS: usize = 12;
const MAX_REPAIR_BYTES: usize = 256 * 1024;
const MAX_OPEN: usize = 128;

pub(super) struct Repair<'a> {
    original: &'a str,
    input: String,
    inserted: Vec<Range<usize>>,
    erased: Vec<Range<usize>>,
}

impl Repair<'_> {
    fn insert(&mut self, at: usize, text: &str) {
        self.input.insert_str(at, text);
        let i = self.inserted.partition_point(|edit| edit.end <= at);
        for edit in &mut self.inserted[i..] {
            edit.start += text.len();
            edit.end += text.len();
        }
        self.inserted.insert(i, at..at + text.len());
    }

    fn map(&self, position: usize) -> usize {
        let removed: usize = self
            .inserted
            .iter()
            .take_while(|edit| edit.start < position)
            .map(|edit| edit.end.min(position) - edit.start)
            .sum();
        self.original.floor_char_boundary(position - removed)
    }

    pub(super) fn map_range(&self, range: Range<usize>) -> Range<usize> {
        self.map(range.start)..self.map(range.end)
    }

    pub(super) fn errors_in(&self, range: &Range<usize>) -> &[Range<usize>] {
        let start = self
            .erased
            .partition_point(|error| error.start < range.start);
        let end = self.erased.partition_point(|error| error.end <= range.end);
        &self.erased[start..end.max(start)]
    }

    fn erase(&mut self, range: Range<usize>) {
        let source = self.map_range(range.clone());
        let i = self
            .erased
            .partition_point(|other| other.end <= source.start);
        self.erased.insert(i, source);
        self.input
            .replace_range(range.clone(), &" ".repeat(range.len()));
    }
}

pub(super) fn repair(source: &str, error: ParseError) -> Option<(SyntaxTree, Repair<'_>)> {
    if source.len() > MAX_REPAIR_BYTES {
        return None;
    }
    let mut offset = unexpected(error)?;
    let (mut suffix, open) = scan(source)?;
    suffix.extend(open.into_iter().rev().map(char::from));
    let mut repair = Repair {
        original: source,
        input: source.to_owned(),
        inserted: Vec::new(),
        erased: Vec::new(),
    };
    if !suffix.is_empty() {
        repair.insert(source.len(), &suffix);
        match parse_tree(&repair.input) {
            Ok(tree) => return Some((tree, repair)),
            Err(error) => offset = unexpected(error)?,
        }
    }
    for _ in 0..MAX_REPAIRS {
        if repair
            .inserted
            .iter()
            .any(|edit| edit.start < offset && offset < edit.end)
        {
            return None;
        }
        let trailing = repair.input.get(offset..)?;
        if let Some(text) = missing_text(&repair.input[..offset], trailing) {
            // Only keep an insertion if it completes the parse or moves the
            // failure past its new text. This prevents retrying the same gap.
            let mut candidate = repair.input.clone();
            candidate.insert_str(offset, text);
            match parse_tree(&candidate) {
                Ok(tree) => {
                    repair.insert(offset, text);
                    return Some((tree, repair));
                }
                Err(error) if unexpected(error).is_some_and(|next| next > offset + text.len()) => {
                    repair.insert(offset, text);
                    offset = unexpected(error)?;
                    continue;
                }
                _ => {}
            }
        }
        let ch = trailing.chars().next()?;
        let end = offset
            + match ch {
                '"' | '\'' => trailing.find('\n').unwrap_or(trailing.len()),
                '?' | '`' => trailing
                    .bytes()
                    .take_while(|&byte| byte == ch as u8)
                    .count(),
                _ => ch.len_utf8(),
            };
        let range = offset..end;
        // A closer or a placeholder supplied by this repair is not source text.
        if repair
            .inserted
            .iter()
            .any(|edit| edit.start < end && offset < edit.end)
        {
            return None;
        }
        repair.erase(range);
        match parse_tree(&repair.input) {
            Ok(tree) => return Some((tree, repair)),
            Err(error) => offset = unexpected(error)?,
        }
    }
    None
}

fn unexpected(error: ParseError) -> Option<usize> {
    match error {
        ParseError::UnexpectedToken { offset, .. } => Some(offset),
        _ => None,
    }
}

fn missing_text(before: &str, after: &str) -> Option<&'static str> {
    let before = before.trim_end();
    let next = after.as_bytes().first().copied();
    if before.ends_with(':') {
        return match next {
            Some(b'{') => Some("x "),        // Pseudo-class without a name.
            None | Some(b'}') => Some("{}"), // Detached ruleset without a block.
            Some(b')' | b']' | b';') => Some("x"),
            _ => None,
        };
    }
    if matches!(next, Some(b';' | b'}' | b']'))
        && scan(before).is_some_and(|(_, open)| open.last() == Some(&b')'))
    {
        return Some(")");
    }
    if next.is_none() || next == Some(b'}') {
        let start = before
            .trim_end_matches(|ch: char| ch.is_ascii_alphanumeric() || ch == '-')
            .len();
        let head = before[..start].trim_end_matches('@');
        if head.len() < start
            && (head.trim_end().is_empty() || head.trim_end().ends_with(['{', '}', ';']))
        {
            return Some(": {}"); // A partially typed declaration name.
        }
        if before.ends_with(['"', '\'']) {
            return Some(";");
        }
    }
    None
}

fn scan(source: &str) -> Option<(String, Vec<u8>)> {
    let source = source.as_bytes();
    let mut input = source.iter().copied().enumerate().peekable();
    let mut open = Vec::new();
    let mut suffix = String::new();
    while let Some((i, byte)) = input.next() {
        match byte {
            b'/' if input.peek().is_some_and(|&(_, next)| next == b'/') => {
                let ended = input.by_ref().any(|(_, byte)| byte == b'\n');
                suffix.extend((!ended && !open.is_empty()).then_some('\n'));
            }
            b'/' if input.peek().is_some_and(|&(_, next)| next == b'*') => {
                input.next();
                let mut star = false;
                loop {
                    match input.next() {
                        Some((_, b'/')) if star => break,
                        Some((_, byte)) => star = byte == b'*',
                        None => {
                            suffix.push_str(if star { "/" } else { "*/" });
                            break;
                        }
                    }
                }
            }
            b'\'' | b'"' => loop {
                match input.next() {
                    Some((_, next)) if next == byte || next == b'\n' => break,
                    Some((_, b'\\')) => {
                        if input.next().is_none() {
                            suffix.push(' ');
                        }
                    }
                    Some(_) => {}
                    None => {
                        suffix.push(char::from(byte));
                        break;
                    }
                }
            },
            b'(' if let Some(end) = literal_end(source, i) => {
                input.by_ref().find(|&(i, _)| i == end);
            }
            b'{' | b'(' | b'[' => {
                if open.len() == MAX_OPEN {
                    return None;
                }
                open.push(match byte {
                    b'{' => b'}',
                    b'(' => b')',
                    _ => b']',
                });
            }
            // Discard an unterminated inner group. Repair it at the first
            // offending token, not at EOF after later valid statements.
            b')' | b']' | b'}' if let Some(i) = open.iter().rposition(|&close| close == byte) => {
                open.truncate(i);
            }
            _ => {}
        }
    }
    Some((suffix, open))
}

// In an unquoted call literal, slashes can be content rather than comments.
// Skip only a complete single-line literal. The table parser validates it.
fn literal_end(source: &[u8], open: usize) -> Option<usize> {
    let name = source[..open]
        .rsplit(|&byte| !byte.is_ascii_alphanumeric() && byte != b'-')
        .next()?;
    if !matches!(name, b"url" | b"url-prefix" | b"domain" | b"regexp") {
        return None;
    }
    let after = &source[open + 1..];
    let content = after.trim_ascii_start();
    if matches!(content.first(), Some(b'\'' | b'"'))
        || content.starts_with(b"/*")
        || content.starts_with(b"//")
    {
        return None;
    }
    let end = after
        .iter()
        .position(|&byte| matches!(byte, b')' | b'\n'))?;
    (after[end] == b')').then_some(open + 1 + end)
}
