//! External tokenizers from the Liquid grammar in @codemirror/lang-liquid.

use super::super::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, TokenInput, Tokenizer,
};

type Token = Result<Option<ExternalToken>, ParseError>;

pub(super) fn tokenizers() -> [Tokenizer; 8] {
    fn external(scan: impl Fn(&mut TokenInput<'_>) -> Token + Send + Sync + 'static) -> Tokenizer {
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, _| scan(input),
        ))
    }
    [
        external(base),
        external(|input| raw(input, b"endraw", 184, 4)),
        external(|input| raw(input, b"endcomment", 185, 5)),
        external(inline_comment),
        0.into(),
        1.into(),
        2.into(),
        3.into(),
    ]
}

fn base(input: &mut TokenInput<'_>) -> Token {
    let start = input.position();
    while let Some(unit) = input.peek(0)? {
        if unit == u16::from(b'{') {
            match input.peek(1)? {
                Some(123 | 37) if input.position() > start => break,
                Some(123) => {
                    input.advance(2)?;
                    return Ok(Some(input.token(1)));
                }
                Some(37) => {
                    let (mut scan, mut size) = (2, 2);
                    let term = loop {
                        match input.peek(scan)? {
                            Some(32 | 10) => scan += 1,
                            Some(35) => {
                                scan += 1;
                                while !matches!(input.peek(scan)?, None | Some(10)) {
                                    scan += 1;
                                }
                            }
                            Some(45) if size == 2 => {
                                scan += 1;
                                size = scan;
                            }
                            Some(101)
                                if input.peek(scan + 1)? == Some(110)
                                    && input.peek(scan + 2)? == Some(100) =>
                            {
                                break 3;
                            }
                            _ => break 2,
                        }
                    };
                    input.advance(size as usize)?;
                    return Ok(Some(input.token(term)));
                }
                _ => {}
            }
        }
        input.advance(1)?;
        if unit == 10 {
            break;
        }
    }
    Ok((input.position() > start).then(|| input.token(183)))
}

fn raw(input: &mut TokenInput<'_>, end_word: &[u8], text: u16, close: u16) -> Token {
    let start = input.position();
    while let Some(unit) = input.peek(0)? {
        if unit == 123 && input.peek(1)? == Some(37) {
            let size = if input.peek(2)? == Some(45) { 3 } else { 2 };
            let mut scan = size;
            while matches!(input.peek(scan)?, Some(32 | 10)) {
                scan += 1;
            }
            let mut matches = true;
            for &letter in end_word {
                if input.peek(scan)? != Some(u16::from(letter)) {
                    matches = false;
                    break;
                }
                scan += 1;
            }
            if matches && !matches!(input.peek(scan)?, Some(65..=90 | 97..=122)) {
                if input.position() == start {
                    input.advance(size as usize)?;
                    return Ok(Some(input.token(close)));
                }
                break;
            }
        }
        input.advance(1)?;
        if unit == 10 {
            break;
        }
    }
    Ok((input.position() > start).then(|| input.token(text)))
}

fn inline_comment(input: &mut TokenInput<'_>) -> Token {
    if input.peek(0)? != Some(35) {
        return Ok(None);
    }
    input.advance(1)?;
    while let Some(unit) = input.peek(0)? {
        if unit == 10 || (matches!(unit, 37 | 125) && input.peek(1)? == Some(125)) {
            break;
        }
        input.advance(1)?;
    }
    Ok(Some(input.token(6)))
}
