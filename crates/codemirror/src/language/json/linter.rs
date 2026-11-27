use crate::lint::{Diagnostic, Severity};

use super::lexer::{Issue, IssueKind, literal, number, skip_space, string};

pub(super) fn diagnostic(source: &str) -> Option<Diagnostic> {
    let (byte, message) = validate(source).err()?.describe(source);
    Some(Diagnostic::new(byte..byte, Severity::Error, message))
}

#[derive(Clone, Copy)]
enum Expect {
    Value,
    End,
    ArrayStart,
    ArraySeparator,
    ObjectStart,
    ObjectKey,
    ObjectColon,
    ObjectSeparator,
}

enum Failure {
    Located(usize, &'static str),
    Unexpected(usize),
}

// Stop at the first error. The pending expectations use space proportional
// to nesting depth, without recursion or a syntax tree.
fn validate(source: &str) -> Result<(), Failure> {
    use Expect::*;
    use Failure::*;

    let bytes = source.as_bytes();
    let mut pending = vec![End, Value];
    let mut at = 0;
    while let Some(expect) = pending.pop() {
        at = skip_space(bytes, at);
        at = match (expect, bytes.get(at).copied()) {
            (Value, Some(b'[')) => {
                pending.push(ArrayStart);
                at + 1
            }
            (Value, Some(b'{')) => {
                pending.push(ObjectStart);
                at + 1
            }
            (Value, Some(b't' | b'f' | b'n')) => {
                literal(bytes, at)
                    .map_err(|issue| Failure::lexical(source, issue))?
                    .0
            }
            (Value, Some(b'-' | b'0'..=b'9')) => {
                let (end, issue) = number(bytes, at);
                if let Some(issue) = issue {
                    return Err(Failure::lexical(source, issue));
                }
                // The number scanner stops at a second digit after a leading zero.
                if bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    return Err(Located(end, "Unexpected number in JSON"));
                }
                end
            }
            (Value | ObjectStart | ObjectKey, Some(b'"')) => {
                if !matches!(expect, Value) {
                    pending.extend([ObjectSeparator, Value, ObjectColon]);
                }
                string(bytes, at + 1).map_err(|issue| Failure::lexical(source, issue))?
            }
            (ArrayStart | ArraySeparator, Some(b']'))
            | (ObjectStart | ObjectSeparator, Some(b'}'))
            | (ObjectColon, Some(b':')) => at + 1,
            (ArrayStart, _) => {
                pending.extend([ArraySeparator, Value]);
                at
            }
            (ArraySeparator, Some(b',')) => {
                pending.extend([ArraySeparator, Value]);
                at + 1
            }
            (ObjectSeparator, Some(b',')) => {
                pending.push(ObjectKey);
                at + 1
            }
            (End, None) => return Ok(()),
            (Value, _) => return Err(Unexpected(at)),
            (expect, _) => {
                let message = match expect {
                    End => "Unexpected non-whitespace character after JSON",
                    ArraySeparator => "Expected ',' or ']' after array element in JSON",
                    ObjectStart => "Expected property name or '}' in JSON",
                    ObjectKey => "Expected double-quoted property name in JSON",
                    ObjectColon => "Expected ':' after property name in JSON",
                    ObjectSeparator => "Expected ',' or '}' after property value in JSON",
                    Value | ArrayStart => unreachable!("handled above"),
                };
                return Err(Located(at, message));
            }
        };
    }
    unreachable!("validation ends at End or at the first error")
}

impl Failure {
    fn lexical(source: &str, Issue { at, kind }: Issue) -> Self {
        let message = match kind {
            IssueKind::Literal => return Self::Unexpected(at),
            IssueKind::Escape => {
                // V8 reports EOF and non-Latin-1 escapes as unexpected tokens.
                if !matches!(source[at..].chars().next(), Some('\0'..='\u{ff}')) {
                    return Self::Unexpected(at);
                }
                "Bad escaped character in JSON"
            }
            IssueKind::UnicodeEscape => "Bad Unicode escape in JSON",
            IssueKind::ControlCharacter => "Bad control character in string literal in JSON",
            IssueKind::UnterminatedString => "Unterminated string in JSON",
            IssueKind::MissingInteger => "No number after minus sign in JSON",
            IssueKind::MissingFraction => "Unterminated fractional number in JSON",
            IssueKind::MissingExponent => "Exponent part is missing a number in JSON",
        };
        Self::Located(at, message)
    }

    fn describe(self, source: &str) -> (usize, String) {
        let (byte, message) = match self {
            Self::Located(byte, message) => (byte, message),
            Self::Unexpected(byte) => match source.as_bytes().get(byte) {
                Some(b'-' | b'0'..=b'9') => (byte, "Unexpected number in JSON"),
                Some(b'"') => (byte, "Unexpected string in JSON"),
                Some(_) => return (0, unexpected_token(source, byte)),
                None => return (0, "Unexpected end of JSON input".into()),
            },
        };
        let (position, line, column) = coordinates(&source[..byte]);
        (
            byte,
            format!("{message} at position {position} (line {line} column {column})"),
        )
    }
}

// V8 prints a UTF-16 index and a one-based line and UTF-16 column.
fn coordinates(prefix: &str) -> (usize, usize, usize) {
    let (mut position, mut line, mut line_start) = (0, 1, 0);
    let mut follows_cr = false;
    for ch in prefix.chars() {
        position += ch.len_utf16();
        if matches!(ch, '\r' | '\n') {
            line += usize::from(ch == '\r' || !follows_cr);
            line_start = position;
        }
        follows_cr = ch == '\r';
    }
    (position, line, position - line_start + 1)
}

fn unexpected_token(source: &str, byte: usize) -> String {
    if matches!(source, "undefined" | "NaN" | "Infinity" | "[object Object]") {
        return format!("\"{source}\" is not valid JSON");
    }
    // V8 shows one UTF-16 code unit for the token. Rust strings cannot store
    // a lone surrogate, including one at either end of the context snippet.
    let token = char::from_u32(u32::from(source[byte..].encode_utf16().next().unwrap()))
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    const CONTEXT: usize = 10;
    let length = source.encode_utf16().count();
    if length <= 2 * CONTEXT {
        return format!("Unexpected token '{token}', \"{source}\" is not valid JSON");
    }
    let position = source[..byte].encode_utf16().count();
    let start = position.saturating_sub(CONTEXT);
    let end = (position + CONTEXT).min(length);
    let left = if position >= CONTEXT { "..." } else { "" };
    let right = if end < length { "..." } else { "" };
    let context: String = char::decode_utf16(source.encode_utf16().skip(start).take(end - start))
        .map(|ch| ch.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    format!("Unexpected token '{token}', {left}\"{context}\"{right} is not valid JSON")
}
