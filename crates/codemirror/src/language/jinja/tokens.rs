use super::super::lezer::{ExternalToken, ExternalTokenizer, ParseError, TokenInput, Tokenizer};

pub(super) fn tokenizers() -> [Tokenizer; 8] {
    [
        external(base),
        external(raw),
        1.into(),
        2.into(),
        3.into(),
        4.into(),
        5.into(),
        external(comment),
    ]
}

type ScanResult = Result<Option<ExternalToken>, ParseError>;

fn external(scan: fn(&mut TokenInput<'_>) -> ScanResult) -> Tokenizer {
    Tokenizer::External(ExternalTokenizer::new_deterministic(
        Default::default(),
        move |input, _| scan(input),
    ))
}

fn base(input: &mut TokenInput<'_>) -> ScanResult {
    let start = input.position();
    while let Some(next) = input.peek(0)? {
        if next == u16::from(b'{') {
            let term = match input.peek(1)? {
                Some(123) => 1, // {{
                Some(35) => 2,  // {#
                Some(37) => 3,  // {%
                _ => 0,
            };
            if term != 0 {
                if input.position() != start {
                    break;
                }
                let size = if term == 3 {
                    tag_start_length(input)?
                } else {
                    2
                };
                input.advance(size)?;
                return Ok(Some(input.token(term)));
            }
        }
        input.advance(1)?;
        if next == u16::from(b'\n') {
            break;
        }
    }
    Ok((input.position() > start).then(|| input.token(155)))
}

fn tag_start_length(input: &TokenInput<'_>) -> Result<usize, ParseError> {
    let (mut scan, mut size) = (2, 2);
    loop {
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
            _ => return Ok(size as usize),
        }
    }
}

fn raw(input: &mut TokenInput<'_>) -> ScanResult {
    let start = input.position();
    while let Some(next) = input.peek(0)? {
        if next == u16::from(b'{') && input.peek(1)? == Some(u16::from(b'%')) && endraw(input)? {
            if input.position() != start {
                break;
            }
            input.advance(2)?;
            return Ok(Some(input.token(4)));
        }
        input.advance(1)?;
        if next == u16::from(b'\n') {
            break;
        }
    }
    Ok((input.position() > start).then(|| input.token(156)))
}

fn endraw(input: &TokenInput<'_>) -> Result<bool, ParseError> {
    let mut scan = 2;
    while matches!(input.peek(scan)?, Some(32 | 10)) {
        scan += 1;
    }
    for byte in b"endraw" {
        if input.peek(scan)? != Some(u16::from(*byte)) {
            return Ok(false);
        }
        scan += 1;
    }
    Ok(!matches!(input.peek(scan)?, Some(65..=90 | 97..=122)))
}

// The bundled LocalTokenGroup has one recognized token (#}) and one
// anonymous fallback term for the comment body. Keep the original slot and
// term numbers without requiring local token support in the shared runtime.
fn comment(input: &mut TokenInput<'_>) -> ScanResult {
    let start = input.position();
    while let Some(next) = input.peek(0)? {
        if next == u16::from(b'#') && input.peek(1)? == Some(u16::from(b'}')) {
            if input.position() == start {
                input.advance(2)?;
                return Ok(Some(input.token(140)));
            }
            break;
        }
        input.advance(1)?;
    }
    Ok((input.position() > start).then(|| input.token(173)))
}
