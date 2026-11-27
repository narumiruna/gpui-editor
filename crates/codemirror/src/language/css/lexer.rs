use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Name,
    AtKeyword,
    Hash,
    String,
    BadString,
    Comment,
    Number { number_end: usize },
    UrlContent,
    Symbol(u8),
    Other,
}

pub(super) struct Token {
    pub kind: Kind,
    pub range: Range<usize>,
    /// Index of the close token, for a matched opening delimiter.
    pub close: Option<usize>,
}

pub(super) fn lex(source: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut open: Vec<(u8, usize)> = Vec::new();
    let last_paren = source.rfind(')');
    let mut at = 0;
    while let Some(ch) = source[at..].chars().next() {
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
            at += 1;
            continue;
        }
        let start = at;
        let rest = &source[at..];
        let bytes = rest.as_bytes();
        let kind = if let Some(comment) = rest.strip_prefix("/*") {
            at = comment.find("*/").map_or(source.len(), |i| at + 4 + i);
            Kind::Comment
        } else if matches!(ch, '"' | '\'') {
            let (end, closed) = string(rest, ch);
            at += end;
            if closed {
                Kind::String
            } else {
                Kind::BadString
            }
        } else if let Some(length) = number(bytes) {
            at += length;
            let number_end = at;
            at += if source[at..].starts_with('%') {
                1
            } else {
                name(&source[at..], false)
            };
            Kind::Number { number_end }
        } else if ch == '#' && name(&rest[1..], true) > 0 {
            at += 1 + name(&rest[1..], true);
            Kind::Hash
        } else if ch == '@' && name(&rest[1..], false) > 0 {
            at += 1 + name(&rest[1..], false);
            Kind::AtKeyword
        } else if let length @ 1.. = name(rest, false) {
            at += length;
            Kind::Name
        } else {
            at += ch.len_utf8();
            if ch.is_ascii_punctuation() {
                Kind::Symbol(bytes[0])
            } else {
                Kind::Other
            }
        };
        let index = tokens.len();
        if let Kind::Symbol(ch) = kind {
            match ch {
                b'(' | b'[' | b'{' => open.push((ch, index)),
                b')' | b']' | b'}' => {
                    let opener = match ch {
                        b')' => b'(',
                        b']' => b'[',
                        _ => b'{',
                    };
                    while let Some(&(prior, i)) = open.last() {
                        if prior == opener {
                            open.pop();
                            tokens[i].close = Some(index);
                            break;
                        }
                        // Preserve a rule boundary when an inner group is broken.
                        if prior == b'{' {
                            break;
                        }
                        open.pop();
                    }
                }
                _ => {}
            }
        }
        let unquoted_url = kind == Kind::Symbol(b'(')
            && last_paren.is_some_and(|end| end > at)
            && tokens.last().is_some_and(|previous| {
                previous.kind == Kind::Name
                    && previous.range.end == start
                    && raw_function(&source[previous.range.clone()])
            });
        tokens.push(Token {
            kind,
            range: start..at,
            close: None,
        });
        if unquoted_url {
            let raw = source[at..].trim_start_matches([' ', '\t', '\n', '\r', '\u{c}']);
            if !raw.starts_with(['\'', '"', ')'])
                && let Some(end) = raw.find(')')
            {
                let start = source.len() - raw.len();
                at = start + end;
                tokens.push(Token {
                    kind: Kind::UrlContent,
                    range: start..at,
                    close: None,
                });
            }
        }
    }
    tokens
}

pub(super) fn raw_function(name: &str) -> bool {
    matches!(name, "url" | "url-prefix" | "domain" | "regexp")
}

fn name(text: &str, digits: bool) -> usize {
    let mut at = 0;
    let mut digit_allowed = digits;
    for (index, ch) in text.char_indices() {
        if index < at {
            continue;
        }
        if ch.is_ascii_alphabetic()
            || ch >= '\u{a1}'
            || ch == '_'
            || ch == '-'
            || digit_allowed && ch.is_ascii_digit()
        {
            digit_allowed |= ch != '-' || index > 0;
            at = index + ch.len_utf8();
        } else if ch == '\\' && !text[index + 1..].starts_with(['\n', '\r']) {
            let tail = &text[index + 1..];
            let hex = tail.bytes().take_while(u8::is_ascii_hexdigit).count();
            at = index
                + 1
                + if hex == 0 {
                    tail.chars().next().map_or(0, char::len_utf8)
                } else {
                    hex + usize::from(tail.as_bytes().get(hex) == Some(&b' '))
                };
            digit_allowed = true;
        } else {
            break;
        }
    }
    if digit_allowed { at } else { 0 }
}

fn string(text: &str, quote: char) -> (usize, bool) {
    let mut chars = text.char_indices().skip(1);
    while let Some((at, ch)) = chars.next() {
        if ch == quote {
            return (at + 1, true);
        }
        if matches!(ch, '\n' | '\r' | '\u{c}') {
            return (at, false);
        }
        if ch == '\\'
            && let Some((i, '\r')) = chars.next()
            && text[i + 1..].starts_with('\n')
        {
            chars.next();
        }
    }
    (text.len(), false)
}

fn number(text: &[u8]) -> Option<usize> {
    let mut at = usize::from(matches!(text.first(), Some(b'+' | b'-')));
    let after_sign = at;
    at += text[at..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if text.get(at) == Some(&b'.') {
        let count = text[at + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if count > 0 || at > after_sign {
            at += 1 + count;
        }
    }
    if at == after_sign {
        return None;
    }
    if matches!(text.get(at), Some(b'e' | b'E')) {
        let exp = at + 1 + usize::from(matches!(text.get(at + 1), Some(b'+' | b'-')));
        let count = text[exp..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if count > 0 {
            at = exp + count;
        }
    }
    Some(at)
}
