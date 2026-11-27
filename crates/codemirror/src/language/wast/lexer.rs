use std::ops::Range;

use super::WastNodeKind::{self, *};

/// Fallback scanner based on the pinned `src/wast.grammar`. Return the longest
/// accepted prefix, even when the next byte is not a separator.
pub(super) struct Lexer<'a> {
    source: &'a str,
    position: usize,
    no_block_end: bool,
    no_string_end: bool,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            source,
            position: 0,
            no_block_end: false,
            no_string_end: false,
        }
    }

    fn token(&mut self, rest: &[u8]) -> (WastNodeKind, usize) {
        match rest[0] {
            b'(' => {
                if rest.starts_with(b"(;") && !self.no_block_end {
                    // This grammar does not nest block comments. The end
                    // delimiter cannot overlap the opening `(;`.
                    if let Some(i) = rest[2..].windows(2).position(|pair| pair == b";)") {
                        return (BlockComment, i + 4);
                    }
                    self.no_block_end = true;
                }
                (OpenParen, 1)
            }
            b')' => (CloseParen, 1),
            b';' if rest.starts_with(b";;") => {
                // The editor normalizes CRLF and CR before parsing. Stop at
                // the line ending without changing the original bytes.
                let end = rest
                    .iter()
                    .position(|&ch| matches!(ch, b'\r' | b'\n'))
                    .unwrap_or(rest.len());
                (LineComment, end)
            }
            b'"' if !self.no_string_end => {
                if let Some(end) = string_end(rest) {
                    return (String, end);
                }
                // All later quotes are escaped, so none can close a string.
                self.no_string_end = true;
                (Error, 1)
            }
            b'$' if rest.get(1).is_some_and(|&ch| id_char(ch)) => (
                Identifier,
                1 + rest[1..].iter().take_while(|&&ch| id_char(ch)).count(),
            ),
            b'a'..=b'z' => {
                let end = rest.iter().take_while(|&&ch| id_char(ch)).count();
                let kind = match &rest[..end] {
                    b"anyref" | b"dataref" | b"eqref" | b"externref" | b"i31ref" | b"funcref"
                    | b"i8" | b"i16" | b"i32" | b"i64" | b"f32" | b"f64" => Type,
                    _ => Keyword,
                };
                (kind, end)
            }
            b'+' | b'-' | b'0'..=b'9' => number_end(rest).map_or((Error, 1), |end| (Number, end)),
            _ => (
                Error,
                self.source[self.position..]
                    .chars()
                    .next()
                    .unwrap()
                    .len_utf8(),
            ),
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = (WastNodeKind, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.source.as_bytes();
        while bytes
            .get(self.position)
            .is_some_and(|&ch| matches!(ch, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.position += 1;
        }
        let start = self.position;
        let rest = bytes.get(start..).filter(|rest| !rest.is_empty())?;
        let (kind, len) = self.token(rest);
        self.position += len;
        Some((kind, start..self.position))
    }
}

fn id_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || b"!#$%&'*+-./:<=>?@^_`|~".contains(&ch)
}

fn string_end(bytes: &[u8]) -> Option<usize> {
    let mut i = 1;
    while let Some(&byte) = bytes.get(i) {
        match byte {
            b'"' => return Some(i + 1),
            // Only ASCII quote and backslash bytes can change the scan.
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

fn number_end(bytes: &[u8]) -> Option<usize> {
    let sign = |ch: &u8| matches!(ch, b'+' | b'-');
    let start = usize::from(sign(&bytes[0]));
    let hex = bytes[start..].starts_with(b"0x")
        && bytes.get(start + 2).is_some_and(u8::is_ascii_hexdigit);
    let is_digit = |ch: &u8| {
        if hex {
            ch.is_ascii_hexdigit()
        } else {
            ch.is_ascii_digit()
        }
    };
    let digits = |mut at: usize| {
        if !bytes.get(at).is_some_and(is_digit) {
            return at;
        }
        at += 1;
        while let Some(ch) = bytes.get(at) {
            if is_digit(ch) {
                at += 1;
            } else if *ch == b'_' && bytes.get(at + 1).is_some_and(is_digit) {
                at += 2;
            } else {
                break;
            }
        }
        at
    };
    let start = start + if hex { 2 } else { 0 };
    let mut end = digits(start);
    if end == start {
        return None;
    }
    if bytes.get(end) == Some(&b'.') {
        end = digits(end + 1);
    }
    if bytes.get(end).is_some_and(|ch| {
        if hex {
            matches!(ch, b'p' | b'P')
        } else {
            matches!(ch, b'e' | b'E')
        }
    }) {
        let exponent = end + 1 + usize::from(bytes.get(end + 1).is_some_and(sign));
        let after = digits(exponent);
        if after > exponent {
            end = after;
        }
    }
    Some(end)
}
