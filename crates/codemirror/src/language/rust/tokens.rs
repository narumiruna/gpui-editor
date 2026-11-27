// Port of @lezer/rust 1.0.3 src/tokens.js.
// See docs/licenses/codemirror-lang-rust.txt.
use super::super::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, TokenInput, Tokenizer,
};

const CLOSURE_PARAM: u16 = 1;
const TYPE_OPEN: u16 = 2;
const TYPE_CLOSE: u16 = 3;
const RAW_STRING: u16 = 4;
const FLOAT: u16 = 5;
const AND_AND: u16 = 301;

pub(super) fn tokenizers() -> [Tokenizer; 8] {
    // The scans depend only on tracked input reads and fixed token IDs.
    let external = |scan: fn(&mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError>| {
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, _| scan(input),
        ))
    };
    [
        external(|input| {
            accept(
                input,
                (input.peek(0)? == unit(b'|')).then_some((CLOSURE_PARAM, 1)),
            )
        }),
        external(|input| {
            let term = match input.peek(0)? {
                Some(60) if input.peek(1)? != unit(b'=') => Some((TYPE_OPEN, 1)),
                Some(62) => Some((TYPE_CLOSE, 1)),
                _ => None,
            };
            accept(input, term)
        }),
        external(literal),
        // The && scan also probes the current LR stack, which the checkpoint restores.
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions {
                contextual: true,
                ..ExternalOptions::default()
            },
            |input, stack| {
                let matched = input.peek(0)? == unit(b'&')
                    && input.peek(1)? == unit(b'&')
                    && stack.can_shift(AND_AND)?;
                accept(input, matched.then_some((AND_AND, 2)))
            },
        )),
        0.into(),
        1.into(),
        2.into(),
        3.into(),
    ]
}

fn accept(
    input: &mut TokenInput<'_>,
    token: Option<(u16, usize)>,
) -> Result<Option<ExternalToken>, ParseError> {
    if let Some((term, len)) = token {
        input.advance(len)?;
        Ok(Some(input.token(term)))
    } else {
        Ok(None)
    }
}

const fn unit(ascii: u8) -> Option<u16> {
    Some(ascii as u16)
}

fn is_digit(ch: Option<u16>) -> bool {
    matches!(ch, Some(48..=57))
}

fn literal(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    match input.peek(0)? {
        ch if is_digit(ch) => float(input),
        Some(98 | 114) => raw_string(input),
        _ => Ok(None),
    }
}

fn float(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    let digits = |input: &mut TokenInput<'_>| -> Result<Option<u16>, ParseError> {
        let mut ch = input.advance(1)?;
        while is_digit(ch) || ch == unit(b'_') {
            ch = input.advance(1)?;
        }
        Ok(ch)
    };
    let mut is_float = false;
    let mut ch = digits(input)?;
    if ch == unit(b'.') {
        is_float = true;
        ch = input.advance(1)?;
        if is_digit(ch) {
            ch = digits(input)?;
        } else if matches!(ch, Some(46 | 65..=90 | 95 | 97..=122 | 128..)) {
            return Ok(None);
        }
    }
    if matches!(ch, Some(69 | 101)) {
        is_float = true;
        ch = input.advance(1)?;
        if matches!(ch, Some(43 | 45)) {
            ch = input.advance(1)?;
        }
        if !is_digit(ch) && ch != unit(b'_') {
            return Ok(None);
        }
        ch = digits(input)?;
    }
    if ch == unit(b'f') {
        if !matches!(
            (input.peek(1)?, input.peek(2)?),
            (Some(51), Some(50)) | (Some(54), Some(52))
        ) {
            return Ok(None);
        }
        is_float = true;
        input.advance(3)?;
    }
    Ok(is_float.then(|| input.token(FLOAT)))
}

fn raw_string(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    if input.peek(0)? == unit(b'b') {
        input.advance(1)?;
    }
    if input.peek(0)? != unit(b'r') {
        return Ok(None);
    }
    let mut ch = input.advance(1)?;
    let mut hashes = 0;
    while ch == unit(b'#') {
        hashes += 1;
        ch = input.advance(1)?;
    }
    if ch != unit(b'"') {
        return Ok(None);
    }
    ch = input.advance(1)?;
    'content: while let Some(before) = ch {
        ch = input.advance(1)?;
        if before == u16::from(b'"') {
            for _ in 0..hashes {
                if ch != unit(b'#') {
                    continue 'content;
                }
                ch = input.advance(1)?;
            }
            return Ok(Some(input.token(RAW_STRING)));
        }
    }
    Ok(None)
}
