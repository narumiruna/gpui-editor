use std::ops::Range;

use super::JsonNodeKind::{self, *};

#[derive(Clone, Copy)]
pub(super) struct Issue {
    pub at: usize,
    pub kind: IssueKind,
}

#[derive(Clone, Copy)]
pub(super) enum IssueKind {
    Literal,
    Escape,
    UnicodeEscape,
    ControlCharacter,
    UnterminatedString,
    MissingInteger,
    MissingFraction,
    MissingExponent,
}

impl Issue {
    fn new(at: usize, kind: IssueKind) -> Self {
        Self { at, kind }
    }

    pub fn recovery_message(self) -> &'static str {
        match self.kind {
            IssueKind::Literal => "Invalid JSON literal",
            IssueKind::Escape => "Invalid escape in JSON string",
            IssueKind::UnicodeEscape => "Invalid Unicode escape in JSON string",
            IssueKind::ControlCharacter => "Unescaped control character in JSON string",
            IssueKind::UnterminatedString => "Unterminated JSON string",
            IssueKind::MissingInteger => "Expected a digit after '-' in JSON number",
            IssueKind::MissingFraction => "Expected a digit after '.' in JSON number",
            IssueKind::MissingExponent => "Expected a digit in JSON exponent",
        }
    }
}

pub(super) struct Token {
    pub kind: JsonNodeKind,
    pub range: Range<usize>,
    pub issue: Option<Issue>,
}

pub(super) struct Lexer<'a> {
    source: &'a str,
    at: usize,
    failed_string: Option<Issue>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            at: 0,
            failed_string: None,
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        let bytes = self.source.as_bytes();
        let at = skip_space(bytes, self.at);
        let mut end = at + 1;
        let mut issue = None;
        let kind = match *bytes.get(at)? {
            b'{' => OpenBrace,
            b'}' => CloseBrace,
            b'[' => OpenBracket,
            b']' => CloseBracket,
            b':' => Colon,
            b',' => Comma,
            b'"' => {
                // An earlier string scan can only have passed escaped quotes
                // before failing. Starting at one of them fails the same way.
                let result = self
                    .failed_string
                    .filter(|failure| at < failure.at)
                    .map_or_else(|| string(bytes, at + 1), Err);
                match result {
                    Ok(to) => {
                        end = to;
                        String
                    }
                    Err(failure) => {
                        issue = Some(failure);
                        self.failed_string = issue;
                        Error
                    }
                }
            }
            b'-' | b'0'..=b'9' => {
                (end, issue) = number(bytes, at);
                if end == at {
                    end = at + 1;
                    Error
                } else {
                    Number
                }
            }
            b't' | b'f' | b'n' => match literal(bytes, at) {
                Ok((to, kind)) => {
                    end = to;
                    kind
                }
                Err(failure) => {
                    issue = Some(failure);
                    Error
                }
            },
            byte => {
                // Advance past an entire scalar even when no token matches.
                if !byte.is_ascii() {
                    end = at + self.source[at..].chars().next()?.len_utf8();
                }
                Error
            }
        };
        self.at = end;
        Some(Token {
            kind,
            range: at..end,
            issue,
        })
    }
}

pub(super) fn skip_space(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        at += 1;
    }
    at
}

pub(super) fn literal(bytes: &[u8], at: usize) -> Result<(usize, JsonNodeKind), Issue> {
    let (word, kind) = match bytes[at] {
        b't' => (b"true".as_slice(), True),
        b'f' => (b"false".as_slice(), False),
        b'n' => (b"null".as_slice(), Null),
        _ => unreachable!("only JSON literal prefixes are scanned"),
    };
    for (offset, byte) in word.iter().enumerate() {
        if bytes.get(at + offset) != Some(byte) {
            return Err(Issue::new(at + offset, IssueKind::Literal));
        }
    }
    Ok((at + word.len(), kind))
}

pub(super) fn string(bytes: &[u8], mut at: usize) -> Result<usize, Issue> {
    loop {
        match bytes.get(at) {
            Some(b'"') => return Ok(at + 1),
            Some(b'\\') => {
                at += 1;
                match bytes.get(at) {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                    Some(b'u') => {
                        for _ in 0..4 {
                            at += 1;
                            if !bytes.get(at).is_some_and(u8::is_ascii_hexdigit) {
                                return Err(Issue::new(at, IssueKind::UnicodeEscape));
                            }
                        }
                    }
                    _ => return Err(Issue::new(at, IssueKind::Escape)),
                }
            }
            Some(0..=31) => return Err(Issue::new(at, IssueKind::ControlCharacter)),
            None => return Err(Issue::new(at, IssueKind::UnterminatedString)),
            _ => {}
        }
        at += 1;
    }
}

pub(super) fn number(bytes: &[u8], start: usize) -> (usize, Option<Issue>) {
    let mut at = start + usize::from(bytes[start] == b'-');
    match bytes.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => at = digits(bytes, at + 1),
        _ => {
            return (start, Some(Issue::new(at, IssueKind::MissingInteger)));
        }
    }
    if bytes.get(at) == Some(&b'.') {
        let end = digits(bytes, at + 1);
        if end == at + 1 {
            return (at, Some(Issue::new(end, IssueKind::MissingFraction)));
        }
        at = end;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let mut exponent = at + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        let end = digits(bytes, exponent);
        if end == exponent {
            return (at, Some(Issue::new(end, IssueKind::MissingExponent)));
        }
        at = end;
    }
    (at, None)
}

fn digits(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    at
}
