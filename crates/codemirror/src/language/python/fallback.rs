//! Lexical highlights and end-of-input repairs for incomplete editor text.

use super::{
    super::{HighlightSpan, lezer::Specialization},
    highlight::{Style, push, style},
    parser::name,
    tables,
};

#[derive(Clone, Copy)]
enum Kind {
    Space,
    Comment,
    Word,
    Number,
    String {
        format: bool,
        missing: Option<&'static str>,
    },
    Symbol,
}

fn lex(text: &str) -> (Kind, usize) {
    let first = text.chars().next().expect("nonempty input");
    if first.is_whitespace() {
        (
            Kind::Space,
            text.find(|c: char| !c.is_whitespace())
                .unwrap_or(text.len()),
        )
    } else if first == '#' {
        (Kind::Comment, text.find(['\n', '\r']).unwrap_or(text.len()))
    } else if let Some((prefix, delimiter)) = string_start(text) {
        let body = prefix + delimiter.len();
        let mut at = body;
        while at < text.len() {
            let rest = &text[at..];
            if rest.starts_with(delimiter) {
                return (
                    Kind::String {
                        format: text[..prefix].contains(['f', 'F']),
                        missing: None,
                    },
                    at + delimiter.len(),
                );
            }
            let ch = rest.chars().next().unwrap();
            if delimiter.len() == 1 && matches!(ch, '\n' | '\r') {
                break;
            }
            at += ch.len_utf8();
            if ch == '\\'
                && let Some(escaped) = text[at..].chars().next()
            {
                at += escaped.len_utf8();
                if escaped == '\r' && text[at..].starts_with('\n') {
                    at += 1;
                }
            }
        }
        (
            Kind::String {
                format: text[..prefix].contains(['f', 'F']),
                missing: Some(delimiter),
            },
            at,
        )
    } else if first.is_ascii_digit()
        || first == '.' && text.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
    {
        let mut end = 0;
        let mut previous = 0;
        for &ch in text.as_bytes() {
            if !(ch.is_ascii_alphanumeric()
                || matches!(ch, b'_' | b'.')
                || matches!(ch, b'+' | b'-') && matches!(previous, b'e' | b'E'))
            {
                break;
            }
            previous = ch;
            end += 1;
        }
        (Kind::Number, end)
    } else if first.is_alphabetic() || first == '_' || !first.is_ascii() {
        let end = first.len_utf8();
        (
            Kind::Word,
            end + text[end..]
                .find(|c: char| {
                    c.is_ascii() && !(c.is_ascii_alphanumeric() || c == '_') || c.is_whitespace()
                })
                .unwrap_or(text.len() - end),
        )
    } else {
        let op = [
            "**=", "//=", "<<=", ">>=", "...", "**", "//", "<<", ">>", ":=", "==", "!=", "<=",
            ">=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "@=", "->",
        ];
        (
            Kind::Symbol,
            op.iter()
                .find(|s| text.starts_with(**s))
                .map_or(first.len_utf8(), |s| s.len()),
        )
    }
}

fn string_start(text: &str) -> Option<(usize, &'static str)> {
    let first_quote = text
        .as_bytes()
        .iter()
        .take(3)
        .position(|&ch| matches!(ch, b'\'' | b'"'))?;
    let prefix = text.get(..first_quote)?;
    if first_quote > 0
        && !["f", "fr", "rf", "r", "u", "b", "br", "rb"]
            .iter()
            .any(|s| prefix.eq_ignore_ascii_case(s))
    {
        return None;
    }
    let quoted = &text[first_quote..];
    ["'''", "\"\"\"", "'", "\""]
        .into_iter()
        .find(|s| quoted.starts_with(s))
        .map(|s| (first_quote, s))
}

pub(super) fn highlights(spans: &mut Vec<HighlightSpan<'static>>, source: &str, start: usize) {
    let mut at = start;
    let mut previous = "";
    while at < source.len() {
        let (kind, len) = lex(&source[at..]);
        let end = at + len;
        let word = &source[at..end];
        let tag = match kind {
            Kind::Space => {
                if word.contains(['\n', '\r']) {
                    previous = "";
                }
                at = end;
                continue;
            }
            Kind::Comment => Some(Style::Comment),
            Kind::Number => Some(Style::Number),
            Kind::String { format, .. } => Some(if format { Style::Format } else { Style::String }),
            Kind::Word => {
                let rest = source[end..].trim_start_matches([' ', '\t']);
                let call = rest.starts_with('(');
                match previous {
                    "def" => Some(Style::Function),
                    "class" => Some(Style::Class),
                    "." => Some(if call { Style::Method } else { Style::Property }),
                    _ => match tables::specialize(word) {
                        Some(Specialization::Replace(term)) => style(name(term), None, None),
                        Some(Specialization::Extend(term))
                            if previous.is_empty()
                                && rest != &source[end..]
                                && !rest.starts_with(['=', '(', '.']) =>
                        {
                            style(name(term), None, None)
                        }
                        _ => Some(if call { Style::Call } else { Style::Variable }),
                    },
                }
            }
            Kind::Symbol => match word {
                "=" | ":=" => Some(Style::Assign),
                "==" | "!=" | "<" | "<=" | ">" | ">=" => Some(Style::Compare),
                "+" | "-" | "*" | "**" | "/" | "//" | "%" => Some(Style::Arith),
                "~" | "&" | "|" | "^" | "<<" | ">>" => Some(Style::Bit),
                "@" => Some(if previous.is_empty() {
                    Style::Meta
                } else {
                    Style::Arith
                }),
                op if op.ends_with('=') => Some(Style::Update),
                "..." => Some(Style::Punct),
                _ => style(word, None, None),
            },
        };
        push(spans, at..end, tag);
        at = end;
        previous = word;
    }
}

// Use this only on a size-limited prefix. Repair missing delimiters, final
// strings, and suites that are still empty at end of input.
pub(super) fn suffix(source: &str) -> String {
    let mut at = 0;
    let mut closes = Vec::new();
    let mut quote = "";
    let mut significant = 0..0;
    let mut in_comment = false;
    while at < source.len() {
        let (kind, len) = lex(&source[at..]);
        let end = at + len;
        let word = &source[at..end];
        in_comment = matches!(kind, Kind::Comment);
        if !matches!(kind, Kind::Space | Kind::Comment) {
            significant = at..end;
        }
        match kind {
            Kind::String {
                missing: Some(missing),
                ..
            } if end == source.len() => quote = missing,
            Kind::Symbol => match word {
                "(" => closes.push(')'),
                "[" => closes.push(']'),
                "{" => closes.push('}'),
                ")" | "]" | "}" if closes.last().is_some_and(|ch| word.starts_with(*ch)) => {
                    closes.pop();
                }
                _ => {}
            },
            _ => {}
        }
        at = end;
    }
    if closes.len() > 256 {
        return String::new();
    }
    let mut suffix = quote.to_owned();
    if in_comment && !closes.is_empty() {
        suffix.push('\n');
    }
    if closes.is_empty() && &source[significant.clone()] == ":" {
        let header = source[..significant.start]
            .rsplit('\n')
            .next()
            .unwrap_or_default();
        suffix.push('\n');
        suffix.extend(header.chars().take_while(|ch| matches!(ch, ' ' | '\t')));
        suffix.push_str("    pass");
    } else {
        suffix.extend(closes.into_iter().rev());
    }
    suffix
}
