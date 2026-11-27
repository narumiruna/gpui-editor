// Scalar scans from @lezer/yaml 1.0.4 src/tokens.js.

use super::{ParseError, TokenInput, line_break, peek, separator, space, three};

// One class for each UTF-16 unit from U+0021 to U+007D: safe, indicator, flow.
const CHAR_TABLE: &[u8] = b"iiisiiissisfissssssssssssisssiiissssssssssssssssssssssssssfsfssissssssssssssssssssssssssssfif";

pub(super) fn char_tag(ch: Option<char>) -> u8 {
    match ch {
        None | Some('\0'..=' ') => b'u',
        Some(ch @ '!'..='}') => CHAR_TABLE[ch as usize - '!' as usize],
        Some(_) => b's',
    }
}

fn safe(ch: Option<char>, flow: bool) -> bool {
    let tag = char_tag(ch);
    tag != b'u' && !(flow && tag == b'f')
}

fn read_uri_char(input: &mut TokenInput<'_>, quoted: bool) -> Result<bool, ParseError> {
    match peek(input, 0)? {
        Some('%') => {
            input.advance(1)?;
            for _ in 0..2 {
                if !peek(input, 0)?.is_some_and(|ch| ch.is_ascii_hexdigit()) {
                    break;
                }
                input.advance(1)?;
            }
        }
        Some(',') if quoted => {
            input.advance(1)?;
        }
        Some(ch @ '!'..='~')
            if !matches!(
                ch,
                '"' | ',' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}'
            ) =>
        {
            input.advance(1)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(super) fn read_tag(input: &mut TokenInput<'_>) -> Result<(), ParseError> {
    input.advance(1)?;
    let quoted = peek(input, 0)? == Some('<');
    if quoted {
        input.advance(1)?;
    }
    while read_uri_char(input, quoted)? {}
    if quoted && peek(input, 0)? == Some('>') {
        input.advance(1)?;
    }
    Ok(())
}

pub(super) fn read_anchor(input: &mut TokenInput<'_>) -> Result<(), ParseError> {
    input.advance(1)?;
    loop {
        let next = peek(input, 0)?;
        if separator(next) || char_tag(next) == b'f' {
            return Ok(());
        }
        input.advance(1)?;
    }
}

pub(super) fn read_quoted(input: &mut TokenInput<'_>, scan: bool) -> Result<bool, ParseError> {
    let quote = peek(input, 0)?.expect("opening quote");
    let start = input.position();
    let mut breaks = false;
    input.advance(1)?;
    while let Some(ch) = peek(input, 0)? {
        input.advance(1)?;
        if ch == quote {
            if ch == '\'' && peek(input, 0)? == Some(ch) {
                input.advance(1)?;
            } else {
                break;
            }
        } else if ch == '\\' && quote == '"' {
            if peek(input, 0)?.is_some() {
                input.advance(1)?;
            }
        } else if line_break(Some(ch)) {
            if scan {
                return Ok(false);
            }
            breaks = true;
        } else if scan && input.position() >= start + 1024 {
            return Ok(false);
        }
    }
    Ok(!breaks)
}

pub(super) fn read_plain(
    input: &mut TokenInput<'_>,
    scan: bool,
    flow: bool,
    indent: isize,
) -> Result<bool, ParseError> {
    let first = peek(input, 0)?;
    if !(char_tag(first) == b's'
        || matches!(first, Some('?' | ':' | '-')) && safe(peek(input, 1)?, flow))
    {
        return Ok(false);
    }
    input.advance(1)?;
    let start = input.position();
    loop {
        let mut next = peek(input, 0)?;
        let mut offset = 0;
        let mut column = indent + 1;
        while space(next) {
            if line_break(next) {
                if scan {
                    return Ok(false);
                }
                column = 0;
            } else {
                column += 1;
            }
            offset += 1;
            next = peek(input, offset)?;
        }
        let valid = match next {
            Some(':') => safe(peek(input, offset + 1)?, flow),
            Some('#') => peek(input, offset - 1)? != Some(' '),
            _ => safe(next, flow),
        };
        if !valid
            || !flow
                && (column <= indent
                    || column == 0 && (three(input, '-', offset)? || three(input, '.', offset)?))
        {
            break;
        }
        if scan && char_tag(next) == b'f' {
            return Ok(false);
        }
        input.advance(offset as usize + 1)?;
        if scan && input.position() > start + 1024 {
            return Ok(false);
        }
    }
    Ok(true)
}
