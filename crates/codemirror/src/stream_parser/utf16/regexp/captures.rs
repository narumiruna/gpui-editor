//! Resolve capture names and decimal escapes before the backend's capture-count scan.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use regress::{Error, Flags, Regex};

use super::compat::{Rewrite, after_braces, after_digits, ascii, hex};

#[derive(Debug)]
pub(super) struct NamedCapture {
    pub(super) name: String,
    pub(super) numbers: Vec<usize>,
}

pub(super) struct CapturePattern<'a> {
    pub(super) source: Cow<'a, [u16]>,
    pub(super) names: Vec<NamedCapture>,
}

// Names can repeat across alternatives of the same disjunction, but two
// terms in the same alternative must have disjoint sets of possible names.
#[derive(Default)]
struct Disjunction {
    current: HashSet<usize>,
    previous: HashSet<usize>,
}

impl Disjunction {
    fn next_alternative(&mut self) {
        if self.current.len() > self.previous.len() {
            std::mem::swap(&mut self.current, &mut self.previous);
        }
        self.previous.extend(self.current.drain());
    }

    fn append_term(&mut self, mut names: HashSet<usize>) -> Result<(), Error> {
        if names.len() > self.current.len() {
            std::mem::swap(&mut self.current, &mut names);
        }
        for name in names {
            if !self.current.insert(name) {
                return Err(pattern_error(
                    "Duplicate capture group name in the same alternative",
                ));
            }
        }
        Ok(())
    }
}

enum Token {
    Name { start: usize, end: usize },
    NamedRef { start: usize, in_class: bool },
    Decimal { start: usize, end: usize },
}

pub(super) fn resolve(pattern: &[u16], flags: Flags) -> Result<CapturePattern<'_>, Error> {
    let mut names: Vec<NamedCapture> = Vec::new();
    let mut ids = HashMap::new();
    let mut tokens = Vec::new();
    let mut capture_count = 0;
    let mut frames = vec![Disjunction::default()];
    let mut classes = 0;
    let mut i = 0;
    while i < pattern.len() {
        match ascii(pattern, i) {
            Some(b'\\') => {
                let end = i + 2;
                match ascii(pattern, i + 1) {
                    Some(b'q')
                        if flags.unicode_sets
                            && classes != 0
                            && ascii(pattern, end) == Some(b'{') =>
                    {
                        i = after_braces(pattern, end);
                        continue;
                    }
                    Some(b'k') => tokens.push(Token::NamedRef {
                        start: i,
                        in_class: classes != 0,
                    }),
                    Some(b'1'..=b'9') if classes == 0 => {
                        let end = after_digits(pattern, end);
                        tokens.push(Token::Decimal { start: i, end });
                        i = end;
                        continue;
                    }
                    _ => {}
                }
                i = end;
                continue;
            }
            Some(b'[') if flags.unicode_sets || classes == 0 => classes += 1,
            Some(b']') if classes != 0 => classes -= 1,
            Some(b'(') if classes == 0 => {
                if ascii(pattern, i + 1) != Some(b'?') {
                    capture_count += 1;
                } else if ascii(pattern, i + 2) == Some(b'<')
                    && !matches!(ascii(pattern, i + 3), Some(b'=' | b'!'))
                {
                    capture_count += 1;
                    let (name, end) = read_name(pattern, i + 2)
                        .ok_or_else(|| pattern_error("Invalid capture group name"))?;
                    let id = *ids.entry(name.clone()).or_insert(names.len());
                    if id == names.len() {
                        names.push(NamedCapture {
                            name,
                            numbers: Vec::new(),
                        });
                    }
                    if !frames.last_mut().unwrap().current.insert(id) {
                        return Err(pattern_error(
                            "Duplicate capture group name in the same alternative",
                        ));
                    }
                    names[id].numbers.push(capture_count);
                    tokens.push(Token::Name { start: i + 1, end });
                    i = end - 1;
                }
                frames.push(Disjunction::default());
            }
            Some(b'|') if classes == 0 => frames.last_mut().unwrap().next_alternative(),
            Some(b')') if classes == 0 && frames.len() > 1 => {
                let mut term = frames.pop().unwrap();
                term.next_alternative();
                frames.last_mut().unwrap().append_term(term.previous)?;
            }
            _ => {}
        }
        i += 1;
    }

    let mut rewrite = Rewrite::new(pattern);
    for token in tokens {
        match token {
            Token::Name { start, end } => rewrite.replace(start, end, []),
            Token::NamedRef { start, in_class } if flags.unicode || !names.is_empty() => {
                if in_class {
                    return Err(pattern_error("Named backreference in character class"));
                }
                let (name, end) = read_name(pattern, start + 2)
                    .ok_or_else(|| pattern_error("Invalid named backreference"))?;
                let id = *ids
                    .get(&name)
                    .ok_or_else(|| pattern_error("Unknown capture group name"))?;
                // At most one of these captures can participate. A backreference
                // to a nonparticipating group consumes nothing. Keep the sequence
                // a single atom for quantifiers and to delimit the decimal escapes.
                let mut reference = String::from("(?:");
                for number in &names[id].numbers {
                    use std::fmt::Write;
                    write!(reference, "\\{number}").unwrap();
                }
                reference.push(')');
                rewrite.replace(start, end, reference.encode_utf16());
            }
            Token::Decimal { start, end } => {
                let number = pattern[start + 1..end].iter().fold(0usize, |n, digit| {
                    n.saturating_mul(10)
                        .saturating_add(usize::from(*digit - u16::from(b'0')))
                });
                if number > capture_count {
                    if flags.unicode {
                        return Err(pattern_error("Invalid decimal backreference"));
                    }
                    let (value, end) = legacy_decimal_escape(pattern, start + 1);
                    rewrite.replace(start, end, format!(r"\x{value:02x}").encode_utf16());
                }
            }
            Token::NamedRef { .. } => {}
        }
    }
    Ok(CapturePattern {
        source: rewrite.finish(),
        names,
    })
}

fn legacy_decimal_escape(pattern: &[u16], start: usize) -> (u8, usize) {
    let first = ascii(pattern, start).unwrap();
    let max_digits = match first {
        b'8' | b'9' => return (first, start + 1),
        b'4'..=b'7' => 2,
        _ => 3,
    };
    let mut value = first - b'0';
    let mut end = start + 1;
    while end < start + max_digits {
        let Some(digit @ b'0'..=b'7') = ascii(pattern, end) else {
            break;
        };
        value = value * 8 + (digit - b'0');
        end += 1;
    }
    (value, end)
}

fn read_name(pattern: &[u16], start: usize) -> Option<(String, usize)> {
    if ascii(pattern, start) != Some(b'<') {
        return None;
    }
    let mut name = String::new();
    let mut i = start + 1;
    loop {
        let (ch, end) = if ascii(pattern, i) == Some(b'\\') {
            escaped_name_char(pattern, i)?
        } else {
            let ch = char::decode_utf16(pattern.get(i..)?.iter().copied())
                .next()?
                .ok()?;
            (ch, i + ch.len_utf16())
        };
        if ch == '>' {
            return valid_name(&name).then_some((name, end));
        }
        if ch.is_ascii() && !ch.is_ascii_alphanumeric() && !matches!(ch, '$' | '_') {
            return None;
        }
        name.push(ch);
        i = end;
    }
}

fn escaped_name_char(pattern: &[u16], start: usize) -> Option<(char, usize)> {
    if ascii(pattern, start + 1) != Some(b'u') {
        return None;
    }
    let digits = start + 2;
    if ascii(pattern, digits) == Some(b'{') {
        let mut value = 0u32;
        let mut end = digits + 1;
        while let Some(digit) = hex(pattern, end, 1) {
            value = value.checked_mul(16)?.checked_add(u32::from(digit))?;
            end += 1;
        }
        return (end > digits + 1 && ascii(pattern, end) == Some(b'}'))
            .then_some((char::from_u32(value)?, end + 1));
    }
    let unit = hex(pattern, digits, 4)?;
    let end = digits + 4;
    if matches!(unit, 0xd800..=0xdbff)
        && ascii(pattern, end) == Some(b'\\')
        && ascii(pattern, end + 1) == Some(b'u')
    {
        let trail = hex(pattern, end + 2, 4)?;
        if matches!(trail, 0xdc00..=0xdfff) {
            let point = crate::utf16::code_point_at(&[unit, trail], 0)?;
            let ch = char::from_u32(point)?;
            return Some((ch, end + 6));
        }
    }
    Some((char::from_u32(u32::from(unit))?, end))
}

fn valid_name(name: &str) -> bool {
    if name.is_ascii() {
        // The scan already checked the remaining ASCII characters.
        return matches!(
            name.as_bytes().first(),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$')
        );
    }
    static IDENTIFIER: OnceLock<Regex> = OnceLock::new();
    IDENTIFIER
        .get_or_init(|| {
            Regex::with_flags(r"^[$_\p{ID_Start}][$_\u200c\u200d\p{ID_Continue}]*$", "u")
                .expect("valid identifier pattern")
        })
        .find(name)
        .is_some()
}

fn pattern_error(text: &str) -> Error {
    Error { text: text.into() }
}
