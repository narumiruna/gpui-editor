// Token hooks from @lezer/php 1.0.6 src/tokens.js.

use super::super::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, Specialization, Specializer,
    StackView, TokenInput, Tokenizer,
};

pub(super) fn keyword_specializer() -> Specializer {
    use Specialization::{Extend, Replace};

    const KEYWORDS: &[(&str, u16)] = &[
        ("abstract", 4),
        ("and", 5),
        ("array", 6),
        ("as", 7),
        ("true", 8),
        ("false", 8),
        ("break", 9),
        ("case", 10),
        ("catch", 11),
        ("clone", 12),
        ("const", 13),
        ("continue", 14),
        ("declare", 16),
        ("default", 15),
        ("do", 17),
        ("echo", 18),
        ("else", 19),
        ("elseif", 20),
        ("enddeclare", 21),
        ("endfor", 22),
        ("endforeach", 23),
        ("endif", 24),
        ("endswitch", 25),
        ("endwhile", 26),
        ("enum", 27),
        ("extends", 28),
        ("final", 29),
        ("finally", 30),
        ("fn", 31),
        ("for", 32),
        ("foreach", 33),
        ("from", 34),
        ("function", 35),
        ("global", 36),
        ("goto", 37),
        ("if", 38),
        ("implements", 39),
        ("include", 40),
        ("include_once", 41),
        ("instanceof", 42),
        ("insteadof", 43),
        ("interface", 44),
        ("list", 45),
        ("match", 46),
        ("namespace", 47),
        ("new", 48),
        ("null", 49),
        ("or", 50),
        ("print", 51),
        ("readonly", 52),
        ("require", 53),
        ("require_once", 54),
        ("return", 55),
        ("switch", 56),
        ("throw", 57),
        ("trait", 58),
        ("try", 59),
        ("unset", 60),
        ("use", 61),
        ("var", 62),
        ("public", 63),
        ("private", 63),
        ("protected", 63),
        ("while", 64),
        ("xor", 65),
        ("yield", 66),
    ];
    // Replacement keywords are case-insensitive, unlike the four extensions.
    // Expand them once so both top rules share a checkpoint-safe text table.
    let mut variants = Vec::new();
    for &(keyword, term) in KEYWORDS {
        keyword_variants(keyword.as_bytes(), &mut String::new(), term, &mut variants);
    }
    let mut entries: Vec<_> = variants
        .iter()
        .map(|(text, term)| (text.as_str(), Replace(*term)))
        .collect();
    entries.extend([
        ("static", Extend(162)),
        ("STATIC", Extend(162)),
        ("class", Extend(175)),
        ("CLASS", Extend(175)),
    ]);
    Specializer::context_free(285, &entries)
}

fn keyword_variants(
    remaining: &[u8],
    text: &mut String,
    term: u16,
    variants: &mut Vec<(String, u16)>,
) {
    let Some((&byte, rest)) = remaining.split_first() else {
        variants.push((text.clone(), term));
        return;
    };
    text.push(char::from(byte));
    keyword_variants(rest, text, term, variants);
    text.pop();
    if byte.is_ascii_lowercase() {
        text.push(char::from(byte.to_ascii_uppercase()));
        keyword_variants(rest, text, term, variants);
        text.pop();
    }
}

const CAST_OPEN: u16 = 1;
const HEREDOC: u16 = 2;
const ESCAPE: u16 = 3;
const CONTENT: u16 = 276;
const AFTER_INTERPOLATION: u16 = 277;
const AUTOMATIC_SEMICOLON: u16 = 278;
const TEMPLATE_EOF: u16 = 279;

type Scan = Result<Option<ExternalToken>, ParseError>;

pub(super) fn tokenizers() -> [Tokenizer; 8] {
    let contextual = ExternalOptions {
        contextual: true,
        ..ExternalOptions::default()
    };
    [
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| expression(input),
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            contextual,
            interpolated,
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            contextual,
            |input, stack| {
                Ok((input.peek(0)? == Some(63)
                    && input.peek(1)? == Some(62)
                    && stack.can_shift(AUTOMATIC_SEMICOLON)?)
                .then(|| input.token(AUTOMATIC_SEMICOLON)))
            },
        )),
        0.into(),
        1.into(),
        2.into(),
        3.into(),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| Ok(input.peek(0)?.is_none().then(|| input.token(TEMPLATE_EOF))),
        )),
    ]
}

fn identifier_start(ch: Option<u16>) -> bool {
    matches!(ch, Some(65..=90 | 95 | 97..=122 | 128..))
}

fn expression(input: &mut TokenInput<'_>) -> Scan {
    match input.peek(0)? {
        Some(40) => cast(input),
        Some(60) if input.peek(1)? == Some(60) && input.peek(2)? == Some(60) => heredoc(input),
        _ => Ok(None),
    }
}

fn cast(input: &mut TokenInput<'_>) -> Scan {
    input.advance(1)?;
    let token = input.token(CAST_OPEN);
    let mut next = input.peek(0)?;
    while matches!(next, Some(9 | 10 | 13 | 32)) {
        next = input.advance(1)?;
    }
    let mut word = [0; 7];
    let mut len = 0;
    while let Some(ch @ (65..=90 | 97..=122)) = next {
        if len == word.len() {
            return Ok(None);
        }
        word[len] = (ch as u8).to_ascii_lowercase();
        len += 1;
        next = input.advance(1)?;
    }
    while matches!(next, Some(9 | 10 | 13 | 32)) {
        next = input.advance(1)?;
    }
    Ok((next == Some(41)
        && matches!(
            &word[..len],
            b"int"
                | b"integer"
                | b"bool"
                | b"boolean"
                | b"float"
                | b"double"
                | b"real"
                | b"string"
                | b"array"
                | b"object"
                | b"unset"
        ))
    .then_some(token))
}

fn heredoc(input: &mut TokenInput<'_>) -> Scan {
    let mut next = input.advance(3)?;
    while matches!(next, Some(9 | 32)) {
        next = input.advance(1)?;
    }
    let quoted = next == Some(39);
    if quoted {
        next = input.advance(1)?;
    }
    if !identifier_start(next) {
        return Ok(None);
    }
    let mut tag = Vec::new();
    while let Some(ch) = next.filter(|&ch| identifier_start(Some(ch)) || (48..=55).contains(&ch)) {
        tag.push(ch);
        next = input.advance(1)?;
    }
    if quoted {
        if next != Some(39) {
            return Ok(None);
        }
        next = input.advance(1)?;
    }
    if !matches!(next, Some(10 | 13)) {
        return Ok(None);
    }
    while let Some(ch) = next {
        next = input.advance(1)?;
        if matches!(ch, 10 | 13) {
            while matches!(next, Some(9 | 32)) {
                next = input.advance(1)?;
            }
            let mut count = 0;
            while tag.get(count).is_some_and(|&unit| Some(unit) == next) {
                count += 1;
                next = input.advance(1)?;
            }
            if count == tag.len() {
                return Ok(Some(input.token(HEREDOC)));
            }
        }
    }
    Ok(None)
}

fn escape_len(input: &TokenInput<'_>) -> Result<usize, ParseError> {
    let hex = |ch| matches!(ch, Some(48..=55 | 65..=70 | 97..=102));
    match input.peek(1)? {
        Some(34 | 36 | 92 | 101 | 102 | 110 | 114 | 116 | 118 | 123) => Ok(2),
        Some(48..=55) => {
            let mut len = 2;
            while len < 5 && matches!(input.peek(len)?, Some(48..=55)) {
                len += 1;
            }
            Ok(len as usize)
        }
        Some(120) if hex(input.peek(2)?) => Ok(if hex(input.peek(3)?) { 4 } else { 3 }),
        Some(117) if input.peek(2)? == Some(123) => {
            for at in 3.. {
                let next = input.peek(at)?;
                if next == Some(125) {
                    return Ok(at as usize + 1);
                }
                if !hex(next) {
                    break;
                }
            }
            Ok(0)
        }
        _ => Ok(0),
    }
}

fn interpolated(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> Scan {
    let start = input.position();
    while let Some(ch) = input.peek(0)? {
        let content = input.position() > start;
        if ch == 34
            || (ch == 36 && (identifier_start(input.peek(1)?) || input.peek(1)? == Some(123)))
            || (ch == 123 && input.peek(1)? == Some(36))
        {
            break;
        }
        if ch == 92 {
            let size = escape_len(input)?;
            if size > 0 {
                if content {
                    break;
                }
                input.advance(size)?;
                return Ok(Some(input.token(ESCAPE)));
            }
        } else if !content
            && (ch == 91
                || (ch == 45 && input.peek(1)? == Some(62) && identifier_start(input.peek(2)?))
                || (ch == 63
                    && input.peek(1)? == Some(45)
                    && input.peek(2)? == Some(62)
                    && identifier_start(input.peek(3)?)))
            && stack.can_shift(AFTER_INTERPOLATION)?
        {
            break;
        }
        input.advance(1)?;
    }
    Ok((input.position() > start).then(|| input.token(CONTENT)))
}
