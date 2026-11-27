use std::{ops::Range, sync::LazyLock};

use super::super::{super::lezer::Specialization, RustNodeKind, RustNodeKind::*, tables};

pub(super) struct Token {
    pub kind: RustNodeKind,
    pub range: Range<usize>,
    pub closed: bool,
}

pub(super) fn tokens(source: &str) -> impl Iterator<Item = Token> + '_ {
    let mut at = 0;
    let source_start = source.len() - source.trim_start_matches('\u{feff}').len();
    std::iter::from_fn(move || {
        let rest = &source[at..];
        let skipped = rest.len() - rest.trim_start_matches(whitespace).len();
        at += skipped;
        let rest = &source[at..];
        if rest.is_empty() {
            return None;
        }
        let start = at;
        let (kind, len, closed) = scan(rest, at == source_start);
        at += len;
        Some(Token {
            kind,
            range: start..at,
            closed,
        })
    })
}

fn whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t'..='\r'
            | ' '
            | '\u{85}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{feff}'
    )
}

fn scan(text: &str, at_start: bool) -> (RustNodeKind, usize, bool) {
    if text.starts_with("//") {
        let len = text.find('\n').unwrap_or(text.len());
        return (
            LineComment,
            len - usize::from(text.as_bytes()[len - 1] == b'\r'),
            true,
        );
    }
    if text.starts_with("/*") {
        let (len, closed) = block_comment(text);
        return (BlockComment, len, closed);
    }
    if let Some((len, closed)) = raw_string(text) {
        return (RawString, len, closed);
    }
    let bytes = text.as_bytes();
    if let Some(quote) = match bytes {
        [b'"', ..] => Some(0),
        [b'b' | b'c', b'"', ..] => Some(1),
        _ => None,
    } {
        let (len, closed) = string(text, quote);
        return (String, len, closed);
    }
    if let Some(quote) = match bytes {
        [b'\'', ..] => Some(0),
        [b'b', b'\'', ..] => Some(1),
        _ => None,
    } {
        if let Some(len) = character(text, quote) {
            return (Char, len, true);
        }
        if quote == 0 {
            let len = identifier(&text[1..]);
            if len > 0 {
                let kind = if bytes.get(len + 1) == Some(&b':') && bytes.get(len + 2) != Some(&b':')
                {
                    LoopLabel
                } else {
                    Lifetime
                };
                return (kind, len + 1, true);
            }
        }
    }
    if let Some(rest) = text.strip_prefix("#!") {
        let spaces = rest.len() - rest.trim_start_matches(whitespace).len();
        if rest[spaces..].starts_with('[') {
            return (InnerAttribute, 2, true);
        }
        if at_start {
            let len = text.find('\n').unwrap_or(text.len());
            return (
                Shebang,
                len - usize::from(text.as_bytes()[len - 1] == b'\r'),
                true,
            );
        }
    }
    if let Some(rest) = text.strip_prefix('#')
        && rest.trim_start_matches(whitespace).starts_with('[')
    {
        return (Attribute, 1, true);
    }
    if bytes[0].is_ascii_digit() {
        let (kind, len) = number(text);
        return (kind, len, true);
    }
    let len = identifier(text);
    if len > 0 {
        return (word(&text[..len]), len, true);
    }
    if let Some(rest) = text.strip_prefix('$') {
        let len = identifier_base(rest);
        if len > 0 {
            return (Metavariable, len + 1, true);
        }
    }
    let (kind, len) = symbol(text);
    (kind, len, true)
}

fn block_comment(text: &str) -> (usize, bool) {
    let bytes = text.as_bytes();
    let (mut at, mut depth) = (2, 1);
    while at + 1 < bytes.len() {
        match &bytes[at..at + 2] {
            b"/*" => {
                depth += 1;
                at += 2;
            }
            b"*/" => {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return (at, true);
                }
            }
            _ => at += 1,
        }
    }
    (text.len(), false)
}

fn raw_string(text: &str) -> Option<(usize, bool)> {
    let prefix = if text.starts_with("br") {
        2
    } else if text.starts_with('r') {
        1
    } else {
        return None;
    };
    let bytes = text.as_bytes();
    let hashes = bytes[prefix..].iter().take_while(|&&b| b == b'#').count();
    let body = prefix + hashes + 1;
    if bytes.get(body - 1) != Some(&b'"') {
        return None;
    }
    for (index, b) in bytes.iter().enumerate().skip(body) {
        if *b == b'"'
            && bytes
                .get(index + 1..index + 1 + hashes)
                .is_some_and(|tail| tail.iter().all(|&b| b == b'#'))
        {
            return Some((index + 1 + hashes, true));
        }
    }
    Some((text.len(), false))
}

fn string(text: &str, quote: usize) -> (usize, bool) {
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate().skip(quote + 1) {
        if !escaped && b == b'"' {
            return (i + 1, true);
        }
        escaped = !escaped && b == b'\\';
    }
    (text.len(), false)
}

fn character(text: &str, quote: usize) -> Option<usize> {
    let body = &text[quote + 1..];
    let mut chars = body.chars();
    let ch = chars.next()?;
    let len = match ch {
        '\'' => 0,
        '\\' => match chars.next()? {
            'x' => 4,
            'u' if body.starts_with("\\u{") => {
                let end = 3 + body[3..].bytes().take_while(u8::is_ascii_hexdigit).count();
                if end == 3 || body.as_bytes().get(end) != Some(&b'}') {
                    return None;
                }
                end + 1
            }
            'u' => 6,
            escape => 1 + escape.len_utf8(),
        },
        ch => ch.len_utf8(),
    };
    (body.as_bytes().get(len) == Some(&b'\'')).then_some(quote + len + 2)
}

fn identifier(text: &str) -> usize {
    text.strip_prefix("r#").map_or_else(
        || identifier_base(text),
        |rest| {
            let len = identifier_base(rest);
            if len == 0 { 1 } else { 2 + len }
        },
    )
}

fn identifier_base(text: &str) -> usize {
    let mut chars = text.char_indices();
    if !chars
        .next()
        .is_some_and(|(_, ch)| ch == '_' || ch.is_alphabetic())
    {
        return 0;
    }
    chars
        .find(|&(_, ch)| ch != '_' && !ch.is_alphanumeric())
        .map_or(text.len(), |(i, _)| i)
}

fn word(text: &str) -> RustNodeKind {
    static KINDS: LazyLock<Box<[RustNodeKind]>> = LazyLock::new(|| {
        tables::NODE_NAMES
            .split(' ')
            .map(RustNodeKind::from_name)
            .collect()
    });
    match tables::keyword(text) {
        Some(Specialization::Replace(term) | Specialization::Extend(term)) => {
            KINDS[usize::from(term)]
        }
        None => Identifier,
    }
}

fn number(text: &str) -> (RustNodeKind, usize) {
    let bytes = text.as_bytes();
    let scan = |start: usize, digit: fn(u8) -> bool| {
        start
            + bytes[start..]
                .iter()
                .take_while(|&&b| digit(b) || b == b'_')
                .count()
    };
    let mut end = match bytes {
        [b'0', b'x', ..] => scan(2, |b| b.is_ascii_hexdigit()),
        [b'0', b'b', ..] => scan(2, |b| matches!(b, b'0'..=b'1')),
        [b'0', b'o', ..] => scan(2, |b| matches!(b, b'0'..=b'7')),
        _ => 0,
    };
    let mut float = false;
    let decimal = end == 0;
    if decimal {
        end = scan(0, |b| b.is_ascii_digit());
        if bytes.get(end) == Some(&b'.') {
            let next = bytes.get(end + 1).copied();
            if next.is_some_and(|b| b.is_ascii_digit()) {
                end = scan(end + 1, |b| b.is_ascii_digit());
                float = true;
            } else if next.is_none_or(|b| {
                b.is_ascii() && !b.is_ascii_alphanumeric() && !matches!(b, b'.' | b'_')
            }) {
                end += 1;
                float = true;
            }
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            let first = end + 1 + usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
            let digits = scan(first, |b| b.is_ascii_digit());
            if digits > first {
                end = digits;
                float = true;
            }
        }
        if text[end..].starts_with("f32") || text[end..].starts_with("f64") {
            return (Float, end + 3);
        }
    }
    if !float && matches!(bytes.get(end), Some(b'i' | b'u')) {
        for suffix in ["size", "128", "64", "32", "16", "8"] {
            if text[end + 1..].starts_with(suffix) {
                end += 1 + suffix.len();
                break;
            }
        }
    }
    (if float { Float } else { Integer }, end)
}

fn symbol(text: &str) -> (RustNodeKind, usize) {
    for (kind, spellings) in [
        (
            UpdateOp,
            &["<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "^=", "&=", "|="][..],
        ),
        (CompareOp, &["<=", ">=", "==", "!="]),
        (BitOp, &["<<", ">>"]),
        (LogicOp, &["&&", "||"]),
        (TripleDot, &["..."]),
        (DoubleDot, &[".."]),
        (DoubleColon, &["::"]),
        (Arrow, &["->"]),
        (FatArrow, &["=>"]),
    ] {
        if let Some(spelling) = spellings.iter().find(|&&s| text.starts_with(s)) {
            return (kind, spelling.len());
        }
    }
    let ch = text.chars().next().unwrap();
    let kind = match ch {
        '(' => OpenParen,
        ')' => CloseParen,
        '[' => OpenBracket,
        ']' => CloseBracket,
        '{' => OpenBrace,
        '}' => CloseBrace,
        '=' => Equal,
        ':' => Colon,
        '.' => DerefOp,
        ',' => Comma,
        ';' => Semicolon,
        '!' => Bang,
        '&' => Ampersand,
        '+' | '-' | '*' | '/' | '%' => ArithOp,
        '|' | '^' => BitOp,
        '<' | '>' => CompareOp,
        _ => Error,
    };
    (kind, ch.len_utf8())
}
