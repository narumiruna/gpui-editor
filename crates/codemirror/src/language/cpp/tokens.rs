use super::super::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, TokenInput, Tokenizer,
};

const RAW_STRING: u16 = 1;
const TEMPLATE_END: u16 = 2;
const MACRO_NAME: u16 = 3;
const BLOCK_COMMENT_CONTENT: u16 = 356;
const BLOCK_COMMENT_LINE_BREAK: u16 = 357;
const BLOCK_COMMENT_CLOSE: u16 = 358;

pub(super) fn tokenizers() -> Vec<Tokenizer> {
    // These scanners use only tracked input reads and fixed token IDs.
    let mut slots = vec![
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| raw_string(input),
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions {
                extend: true,
                ..ExternalOptions::default()
            },
            |input, _| fallback(input),
        )),
    ];
    slots.extend((1..=10).map(Tokenizer::Internal));
    // Adapt the grammar's one local token group to the shared scanner API.
    slots.push(Tokenizer::External(ExternalTokenizer::new_deterministic(
        ExternalOptions::default(),
        |input, _| block_comment(input),
    )));
    slots
}

const R: u16 = b'R' as u16;
const L: u16 = b'L' as u16;
const BIG_U: u16 = b'U' as u16;
const SMALL_U: u16 = b'u' as u16;
const QUOTE: u16 = b'"' as u16;
const OPEN: u16 = b'(' as u16;
const CLOSE: u16 = b')' as u16;
const GT: u16 = b'>' as u16;
const LF: u16 = b'\n' as u16;
const STAR: u16 = b'*' as u16;
const SLASH: u16 = b'/' as u16;

fn raw_string(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    let mut next = input.peek(0)?;
    if matches!(next, Some(L | BIG_U)) {
        next = input.advance(1)?;
    } else if next == Some(SMALL_U) {
        next = input.advance(1)?;
        if next == Some(u16::from(b'8')) {
            next = input.advance(1)?;
        }
    }
    if next != Some(R) || input.advance(1)? != Some(QUOTE) {
        return Ok(None);
    }
    let mut marker = Vec::new();
    while let Some(unit) = input.advance(1)? {
        if unit == OPEN {
            break;
        }
        if unit <= 13 || unit == u16::from(b' ') || unit == CLOSE {
            return Ok(None);
        }
        marker.push(unit);
    }
    if input.peek(0)?.is_none() {
        return Ok(None);
    }
    while let Some(unit) = input.advance(1)? {
        if unit == CLOSE {
            let mut matched = true;
            for (i, &unit) in marker.iter().enumerate() {
                if input.peek(i as isize + 1)? != Some(unit) {
                    matched = false;
                    break;
                }
            }
            if matched && input.peek(marker.len() as isize + 1)? == Some(QUOTE) {
                input.advance(marker.len() + 2)?;
                break;
            }
        }
    }
    // The grammar accepts raw string content that reaches EOF.
    Ok(Some(input.token(RAW_STRING)))
}

fn fallback(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    let start = input.position();
    let mut next = input.peek(0)?;
    if next == Some(GT) {
        if input.advance(1)? == Some(GT) {
            return Ok(Some(input.token(TEMPLATE_END)));
        }
        return Ok(None);
    }
    let mut saw_letter = false;
    while let Some(unit) = next.and_then(|unit| u8::try_from(unit).ok()) {
        if unit.is_ascii_uppercase() {
            saw_letter = true;
        } else if unit != b'_' && !unit.is_ascii_digit() {
            if unit.is_ascii_lowercase() {
                return Ok(None);
            }
            break;
        }
        next = input.advance(1)?;
    }
    Ok((saw_letter && input.position() - start > 1).then(|| input.token(MACRO_NAME)))
}

fn block_comment(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    let start = input.position();
    let mut next = input.peek(0)?;
    while let Some(unit) = next {
        if unit == LF || (unit == STAR && input.peek(1)? == Some(SLASH)) {
            if input.position() != start {
                break;
            }
            let (width, term) = if unit == LF {
                (1, BLOCK_COMMENT_LINE_BREAK)
            } else {
                (2, BLOCK_COMMENT_CLOSE)
            };
            input.advance(width)?;
            return Ok(Some(input.token(term)));
        }
        next = input.advance(1)?;
    }
    Ok((input.position() != start).then(|| input.token(BLOCK_COMMENT_CONTENT)))
}
