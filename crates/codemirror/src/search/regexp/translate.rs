use std::{fmt::Write, iter::Peekable, str::Chars};

use crate::search::SearchError;

const SPACE: &str = r" \t\n\r\x0B\x0C\x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

type PatternChars<'a> = Peekable<Chars<'a>>;

// Rewrite ECMAScript atoms for the linear Rust matcher. Do not let Rust-only
// escape or quantifier syntax through just because that matcher accepts it.
pub(super) fn translate(pattern: &str) -> Result<(String, bool), SearchError> {
    let mut rest = pattern.chars().peekable();
    let mut output = String::with_capacity(pattern.len());
    let mut in_class = false;
    let mut range_end = false;
    let mut word_boundary = false;
    while let Some(ch) = rest.next() {
        let class_atom = in_class && ch != ']';
        let class_escape = class_atom
            && ch == '\\'
            && matches!(
                rest.peek(),
                Some('d' | 'D' | 'w' | 'W' | 's' | 'S' | 'p' | 'P')
            );
        match ch {
            '[' if in_class => output.push_str(r"\["),
            '[' => {
                if rest.next_if_eq(&']').is_some() {
                    output.push_str("[a&&b]"); // The empty class never matches.
                    continue;
                }
                if rest.next_if_eq(&'^').is_some() {
                    if rest.next_if_eq(&']').is_some() {
                        output.push_str("(?s:.)");
                        continue;
                    }
                    output.push_str("[^");
                } else {
                    output.push('[');
                }
                in_class = true;
            }
            ']' if in_class => {
                in_class = false;
                output.push(']');
            }
            ']' | '}' if !in_class => return Err(invalid("unescaped closing bracket")),
            '(' if !in_class && rest.next_if_eq(&'?').is_some() => match rest.next() {
                Some(':') => output.push_str("(?:"),
                Some('<') if !matches!(rest.peek(), Some('=' | '!')) => output.push_str("(?<"),
                _ => return Err(invalid("unsupported group syntax")),
            },
            '.' if !in_class => output.push_str(r"[^\n\r\x{2028}\x{2029}]"),
            '^' | '$' if !in_class => {
                check_assertion(&mut rest)?;
                output.push(ch);
            }
            '*' | '+' | '?' | '{' if !in_class => quantifier(ch, &mut rest, &mut output)?,
            '-' | '&' | '~' if in_class => {
                output.push('\\');
                output.push(ch);
            }
            '\\' => match rest.next() {
                Some('d') => output.push_str("[0-9]"),
                Some('D') => output.push_str("[^0-9]"),
                Some('w') => output.push_str("[A-Za-z0-9_]"),
                Some('W') => output.push_str("[^A-Za-z0-9_]"),
                Some(ch @ ('s' | 'S')) => {
                    output.push('[');
                    if ch == 'S' {
                        output.push('^');
                    }
                    output.push_str(SPACE);
                    output.push(']');
                }
                Some('b') if in_class => output.push_str(r"\x08"),
                Some(ch @ ('b' | 'B')) if !in_class => {
                    word_boundary = true;
                    check_assertion(&mut rest)?;
                    write!(output, "(?-u:\\{ch})").expect("writing to a string succeeds");
                }
                Some('0') if !rest.peek().is_some_and(char::is_ascii_digit) => {
                    output.push_str(r"\x00");
                }
                Some('/') => output.push('/'),
                Some('c') => {
                    let ch = rest
                        .next()
                        .filter(char::is_ascii_alphabetic)
                        .ok_or_else(|| invalid("invalid control escape"))?;
                    write!(output, "\\x{:02X}", ch as u32 & 31)
                        .expect("writing to a string succeeds");
                }
                Some('x') => {
                    let value = hex(&mut rest, 2)?;
                    write!(output, "\\x{value:02X}").expect("writing to a string succeeds");
                }
                Some('u') => unicode_escape(&mut rest, &mut output)?,
                Some(ch @ ('p' | 'P')) => property(ch, &mut rest, &mut output)?,
                Some(ch @ ('f' | 'n' | 'r' | 't' | 'v')) => {
                    output.push('\\');
                    output.push(ch);
                }
                Some(ch) if r"^$\.*+?()[]{}|".contains(ch) || (in_class && ch == '-') => {
                    output.push('\\');
                    output.push(ch);
                }
                _ => return Err(invalid("unsupported or invalid escape")),
            },
            _ => output.push(ch),
        }
        if class_atom {
            // Consume a range separator only after a first atom and before a
            // second one. Escape all other hyphens: Rust treats leading
            // hyphens as literals and `--` elsewhere as set difference.
            let range_start = !range_end
                && rest.peek() == Some(&'-')
                && rest.clone().nth(1).is_some_and(|next| next != ']');
            if class_escape && (range_start || range_end) {
                return Err(invalid("character class escape used as range endpoint"));
            }
            if range_start {
                rest.next();
                output.push('-');
            }
            range_end = range_start;
        }
    }
    Ok((output, word_boundary))
}

fn invalid(reason: &str) -> SearchError {
    SearchError::InvalidRegExp(reason.into())
}

fn starts_quantifier(rest: &mut PatternChars<'_>) -> bool {
    matches!(rest.peek(), Some('*' | '+' | '?' | '{'))
}

fn check_assertion(rest: &mut PatternChars<'_>) -> Result<(), SearchError> {
    if starts_quantifier(rest) {
        return Err(invalid("quantified assertion"));
    }
    Ok(())
}

fn quantifier(
    ch: char,
    rest: &mut PatternChars<'_>,
    output: &mut String,
) -> Result<(), SearchError> {
    output.push(ch);
    if ch == '{' {
        let start = output.len();
        while let Some(digit) = rest.next_if(char::is_ascii_digit) {
            output.push(digit);
        }
        if output.len() == start {
            return Err(invalid("missing quantifier count"));
        }
        if rest.next_if_eq(&',').is_some() {
            output.push(',');
            while let Some(digit) = rest.next_if(char::is_ascii_digit) {
                output.push(digit);
            }
        }
        if rest.next_if_eq(&'}').is_none() {
            return Err(invalid("invalid quantifier count"));
        }
        output.push('}');
    }
    if rest.next_if_eq(&'?').is_some() {
        output.push('?');
    }
    if starts_quantifier(rest) {
        return Err(invalid("consecutive quantifiers"));
    }
    Ok(())
}

fn hex(rest: &mut PatternChars<'_>, count: usize) -> Result<u32, SearchError> {
    let mut value = 0;
    for _ in 0..count {
        let digit = rest
            .next()
            .and_then(|ch| ch.to_digit(16))
            .ok_or_else(|| invalid("invalid hexadecimal escape"))?;
        value = value * 16 + digit;
    }
    Ok(value)
}

fn unicode_escape(rest: &mut PatternChars<'_>, output: &mut String) -> Result<(), SearchError> {
    let value = if rest.next_if_eq(&'{').is_some() {
        if !rest.peek().is_some_and(char::is_ascii_hexdigit) {
            return Err(invalid("empty Unicode code point escape"));
        }
        let mut value = 0;
        while rest.peek().is_some_and(char::is_ascii_hexdigit) {
            value = value * 16 + hex(rest, 1)?;
            if value > 0x10FFFF {
                return Err(invalid("invalid Unicode code point"));
            }
        }
        if rest.next_if_eq(&'}').is_none() {
            return Err(invalid("unclosed Unicode code point escape"));
        }
        value
    } else {
        let high = hex(rest, 4)?;
        if (0xD800..=0xDBFF).contains(&high) {
            if rest.next() != Some('\\') || rest.next() != Some('u') {
                return Err(invalid("lone surrogate escape"));
            }
            let low = hex(rest, 4)?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(invalid("invalid surrogate pair"));
            }
            0x10000 + (high - 0xD800) * 0x400 + low - 0xDC00
        } else {
            high
        }
    };
    if char::from_u32(value).is_none() {
        return Err(invalid("lone surrogate escape"));
    }
    write!(output, "\\x{{{value:X}}}").expect("writing to a string succeeds");
    Ok(())
}

fn property(ch: char, rest: &mut PatternChars<'_>, output: &mut String) -> Result<(), SearchError> {
    let start = output.len();
    if rest.next_if_eq(&'{').is_none() {
        return Err(invalid("missing Unicode property name"));
    }
    output.push_str(r"\p{");
    while let Some(ch) = rest.next_if(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '=')) {
        output.push(ch);
    }
    if rest.next_if_eq(&'}').is_none() {
        return Err(invalid("invalid Unicode property name"));
    }
    output.push('}');
    // Rust accepts loose aliases that JavaScript rejects. Use the ECMAScript
    // parser on one positive property atom only. Never run its matcher.
    regress::Regex::with_flags(&output[start..], "u")
        .map_err(|_| invalid("invalid Unicode property name"))?;
    if ch == 'P' {
        output.replace_range(start + 1..start + 2, "P");
    }
    Ok(())
}
