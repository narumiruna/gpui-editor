// Scanner hooks from @lezer/css 1.3.9, src/tokens.js. The local comment
// group below implements the three rules from src/css.grammar.
use super::super::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, StackView, TokenInput, Tokenizer,
};

type ScanResult = Result<Option<ExternalToken>, ParseError>;

pub(super) fn external(
    scan: fn(&mut TokenInput<'_>, &StackView<'_>) -> ScanResult,
    contextual: bool,
) -> Tokenizer {
    Tokenizer::External(ExternalTokenizer::new_deterministic(
        ExternalOptions {
            contextual,
            ..ExternalOptions::default()
        },
        scan,
    ))
}

fn alpha(unit: u16) -> bool {
    (u16::from(b'A')..=u16::from(b'Z')).contains(&unit)
        || (u16::from(b'a')..=u16::from(b'z')).contains(&unit)
        || unit >= 161
}

fn digit(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| byte.is_ascii_digit())
}

fn hex(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| byte.is_ascii_hexdigit())
}

fn space(unit: u16) -> bool {
    matches!(unit, 9..=13 | 32 | 133 | 160 | 5760 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
}

pub(super) fn identifier(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> ScanResult {
    identifier_tokens(input, stack, [149, 2, 150])
}

pub(super) fn query_identifier(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> ScanResult {
    identifier_tokens(input, stack, [151, 3, 4])
}

fn identifier_tokens(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
    [name, variable, callee]: [u16; 3],
) -> ScanResult {
    let mut inside = false;
    let mut dashes = 0;
    let mut next = input.peek(0)?;
    for index in 0.. {
        let byte = next.and_then(|unit| u8::try_from(unit).ok());
        if next.is_some_and(alpha)
            || matches!(byte, Some(b'-' | b'_'))
            || inside && next.is_some_and(digit)
        {
            inside |= byte != Some(b'-') || index > 0;
            if dashes == index && byte == Some(b'-') {
                dashes += 1;
            }
            next = input.advance(1)?;
        } else if byte == Some(b'\\') && input.peek(1)? != Some(10) {
            next = input.advance(1)?;
            if next.is_some_and(hex) {
                while next.is_some_and(hex) {
                    next = input.advance(1)?;
                }
                if next == Some(32) {
                    next = input.advance(1)?;
                }
            } else if next.is_some() {
                next = input.advance(1)?;
            }
            inside = true;
        } else if inside {
            let term = if dashes >= 2 && stack.can_shift(2)? {
                variable
            } else if byte == Some(b'(') {
                callee
            } else {
                name
            };
            return Ok(Some(input.token(term)));
        } else {
            return Ok(None);
        }
    }
    unreachable!()
}

pub(super) fn descendant(input: &mut TokenInput<'_>, _: &StackView<'_>) -> ScanResult {
    if !input.peek(-1)?.is_some_and(space) {
        return Ok(None);
    }
    let next = input.peek(0)?;
    let byte = next.and_then(|unit| u8::try_from(unit).ok());
    let starts_selector = next.is_some_and(alpha)
        || matches!(byte, Some(b'_' | b'#' | b'.' | b'*' | b'[' | b'-' | b'&'))
        || byte == Some(b':') && input.peek(1)?.is_some_and(alpha);
    Ok(starts_selector.then(|| input.token(148)))
}

pub(super) fn unit(input: &mut TokenInput<'_>, _: &StackView<'_>) -> ScanResult {
    if input.peek(-1)?.is_some_and(space) {
        return Ok(None);
    }
    let next = input.peek(0)?;
    if next == Some(u16::from(b'%')) {
        input.advance(1)?;
    } else if next.is_some_and(alpha) {
        while input
            .advance(1)?
            .is_some_and(|unit| alpha(unit) || digit(unit))
        {}
    } else {
        return Ok(None);
    }
    Ok(Some(input.token(1)))
}

pub(super) fn comment(input: &mut TokenInput<'_>, _: &StackView<'_>) -> ScanResult {
    let start = input.position();
    loop {
        let next = input.peek(0)?;
        let end = next.is_none() || next == Some(42) && input.peek(1)? == Some(47);
        if end || next == Some(10) {
            let term = if input.position() > start {
                156 // commentContent (the local group's else token)
            } else if end {
                if next.is_some() {
                    input.advance(2)?;
                }
                158 // commentEnd
            } else {
                input.advance(1)?;
                157 // commentLineBreak
            };
            return Ok(Some(input.token(term)));
        }
        input.advance(1)?;
    }
}

pub(super) const COLOR_LENGTHS: [usize; 4] = [4, 5, 7, 9];

pub(super) fn is_color(text: &str) -> bool {
    COLOR_LENGTHS.contains(&text.len())
        && text
            .strip_prefix('#')
            .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
