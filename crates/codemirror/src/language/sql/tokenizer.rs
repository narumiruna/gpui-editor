// External tokenizer from @codemirror/lang-sql 6.10.0.
// See docs/licenses/codemirror-lang-sql.txt.

use std::ops::Range;

use super::{SqlNodeKind, dialect::LexConfig, tables::WHITESPACE};
use crate::language::lezer::{ExternalToken, ParseError, TokenInput};

const SPACE: &[u8] = b" \t\r\n";

pub(super) fn external(
    input: &mut TokenInput<'_>,
    config: &LexConfig,
) -> Result<Option<ExternalToken>, ParseError> {
    let term = scan(&mut Input::Lezer(input), config)?;
    Ok(term.map(|term| input.token(term)))
}

/// The same scanner without LR state. Used when strict parsing cannot finish.
pub(super) fn lex(source: &str, config: &LexConfig) -> Vec<(SqlNodeKind, Range<usize>)> {
    let mut units = Vec::new();
    let mut bytes = Vec::new();
    for (byte, ch) in source.char_indices() {
        for &mut unit in ch.encode_utf16(&mut [0; 2]) {
            bytes.push(byte);
            units.push(unit);
        }
    }
    bytes.push(source.len());
    let mut tokens: Vec<(SqlNodeKind, Range<usize>)> = Vec::new();
    let mut pos = 0;
    while pos < units.len() {
        let mut input = Input::Local { units: &units, pos };
        let term = scan(&mut input, config).expect("local cursor has no work limit");
        let Input::Local { pos: end, .. } = input else {
            unreachable!()
        };
        let mut end = if term.is_some() { end } else { pos + 1 };
        let split_scalar = bytes[end] == bytes[end - 1];
        if split_scalar {
            end += 1;
        }
        if term != Some(WHITESPACE) {
            let kind = if split_scalar {
                SqlNodeKind::Error
            } else {
                term.map_or(SqlNodeKind::Error, SqlNodeKind::from_term)
            };
            let range = bytes[pos]..bytes[end];
            // Lezer merges consecutive skipped code units into one error node.
            if term.is_none()
                && let Some((SqlNodeKind::Error, prior)) = tokens.last_mut()
                && prior.end == range.start
            {
                prior.end = range.end;
            } else {
                tokens.push((kind, range));
            }
        }
        pos = end;
    }
    tokens
}

enum Input<'a, 'b> {
    Lezer(&'a mut TokenInput<'b>),
    Local { units: &'a [u16], pos: usize },
}

impl Input<'_, '_> {
    fn peek(&self, offset: isize) -> Result<Option<u16>, ParseError> {
        match self {
            Self::Lezer(input) => input.peek(offset),
            Self::Local { units, pos } => Ok(pos
                .checked_add_signed(offset)
                .and_then(|p| units.get(p))
                .copied()),
        }
    }

    fn next(&self) -> Result<Option<u16>, ParseError> {
        self.peek(0)
    }

    fn at(&self, byte: u8) -> Result<bool, ParseError> {
        Ok(self.next()? == Some(u16::from(byte)))
    }

    fn advance(&mut self, count: usize) -> Result<(), ParseError> {
        match self {
            Self::Lezer(input) => {
                input.advance(count)?;
            }
            Self::Local { units, pos } => *pos = (*pos + count).min(units.len()),
        }
        Ok(())
    }

    fn eat_while(&mut self, matches: impl Fn(u16) -> bool) -> Result<(), ParseError> {
        while self.next()?.is_some_and(&matches) {
            self.advance(1)?;
        }
        Ok(())
    }
}

fn ascii(ch: u16, test: impl FnOnce(u8) -> bool) -> bool {
    u8::try_from(ch).is_ok_and(test)
}

fn space(ch: u16) -> bool {
    ascii(ch, |c| SPACE.contains(&c))
}

fn word_char(ch: u16) -> bool {
    ascii(ch, |c| c.is_ascii_alphanumeric() || c == b'_')
}

fn scan(input: &mut Input<'_, '_>, d: &LexConfig) -> Result<Option<u16>, ParseError> {
    use SqlNodeKind::*;
    let Some(first) = input.next()? else {
        return Ok(None);
    };
    input.advance(1)?;
    let ch = u8::try_from(first).unwrap_or(0);
    let kind = if space(first) {
        input.eat_while(space)?;
        return Ok(Some(WHITESPACE));
    } else if ch == b'$' && d.double_dollar_quoted_strings {
        let mut tag = Vec::new();
        while let Some(unit) = input.next()?.filter(|&c| word_char(c)) {
            tag.push(unit);
            input.advance(1)?;
        }
        if !input.at(b'$')? {
            return Ok(None);
        }
        input.advance(1)?;
        dollar_literal(input, &tag)?;
        String
    } else if ch == b'\'' || ch == b'"' && d.double_quoted_strings {
        literal(input, first, d.backslash_escapes)?;
        String
    } else if ch == b'#' && d.hash_comments
        || ch == b'/' && d.slash_comments && input.at(b'/')?
        || ch == b'-'
            && input.at(b'-')?
            && (!d.space_after_dashes || input.peek(1)? == Some(u16::from(b' ')))
    {
        input.eat_while(|c| c != u16::from(b'\n'))?;
        LineComment
    } else if ch == b'/' && input.at(b'*')? {
        input.advance(1)?;
        block_comment(input)?;
        BlockComment
    } else if matches!(ch, b'e' | b'E') && input.at(b'\'')? {
        input.advance(1)?;
        literal(input, u16::from(b'\''), true)?;
        String
    } else if matches!(ch, b'n' | b'N') && d.char_set_casts && input.at(b'\'')? {
        input.advance(1)?;
        literal(input, u16::from(b'\''), d.backslash_escapes)?;
        String
    } else if ch == b'_' && d.char_set_casts {
        let mut len = 0;
        while input
            .next()?
            .is_some_and(|c| ascii(c, |c| c.is_ascii_alphanumeric()))
        {
            len += 1;
            input.advance(1)?;
        }
        if len < 2 || !input.at(b'\'')? {
            return Ok(None);
        }
        input.advance(1)?;
        literal(input, u16::from(b'\''), d.backslash_escapes)?;
        String
    } else if d.plsql_quoting_mechanism
        && matches!(ch, b'q' | b'Q')
        && input.at(b'\'')?
        && let Some(open) = input.peek(1)?.filter(|&c| c > 0 && !space(c))
    {
        input.advance(2)?;
        plsql_literal(input, open)?;
        String
    } else if d.identifier_quotes.contains(&first) {
        literal(
            input,
            if ch == b'[' { u16::from(b']') } else { first },
            false,
        )?;
        QuotedIdentifier
    } else if let Some(punctuation) = match ch {
        b'(' => Some(OpenParen),
        b')' => Some(CloseParen),
        b'{' => Some(OpenBrace),
        b'}' => Some(CloseBrace),
        b'[' => Some(OpenBracket),
        b']' => Some(CloseBracket),
        b';' => Some(Semicolon),
        _ => None,
    } {
        punctuation
    } else if d.unquoted_bit_literals && ch == b'0' && input.at(b'b')? {
        input.advance(1)?;
        bits(input, None)?;
        Bits
    } else if matches!(ch, b'b' | b'B')
        && let Some(quote) = input
            .next()?
            .filter(|&c| ascii(c, |c| matches!(c, b'\'' | b'"')))
    {
        input.advance(1)?;
        if d.treat_bits_as_bytes {
            literal(input, quote, d.backslash_escapes)?;
            Bytes
        } else {
            bits(input, Some(quote))?;
            Bits
        }
    } else if ch == b'0' && (input.at(b'x')? || input.at(b'X')?)
        || matches!(ch, b'x' | b'X') && input.at(b'\'')?
    {
        let quoted = input.at(b'\'')?;
        input.advance(1)?;
        input.eat_while(|c| ascii(c, |c| c.is_ascii_hexdigit()))?;
        if quoted && input.at(b'\'')? {
            input.advance(1)?;
        }
        Number
    } else if ch == b'.'
        && input
            .next()?
            .is_some_and(|c| ascii(c, |c| c.is_ascii_digit()))
    {
        number(input, true)?;
        Number
    } else if ch == b'.' {
        Dot
    } else if ch.is_ascii_digit() {
        number(input, false)?;
        Number
    } else if d.operator_chars.contains(&first) {
        input.eat_while(|c| d.operator_chars.contains(&c))?;
        Operator
    } else if d.special_var.contains(&first) {
        if input.next()? == Some(first) {
            input.advance(1)?;
        }
        if let Some(quote) = input
            .next()?
            .filter(|&c| ascii(c, |c| matches!(c, b'\'' | b'"' | b'`')))
        {
            input.advance(1)?;
            literal(input, quote, false)?;
        } else {
            input.eat_while(word_char)?;
        }
        SpecialVar
    } else if matches!(ch, b':' | b',') {
        Punctuation
    } else if ch.is_ascii_alphanumeric() {
        let mut word = std::string::String::from(char::from(ch.to_ascii_lowercase()));
        while let Some(unit) = input.next()?.filter(|&c| word_char(c)) {
            word.push(char::from((unit as u8).to_ascii_lowercase()));
            input.advance(1)?;
        }
        if input.at(b'.')? || input.peek(-((word.len() + 1) as isize))? == Some(u16::from(b'.')) {
            Identifier
        } else {
            d.word_kind(&word)
        }
    } else {
        return Ok(None);
    };
    Ok(Some(kind as u16))
}

fn literal(input: &mut Input<'_, '_>, quote: u16, backslash: bool) -> Result<(), ParseError> {
    let mut escaped = false;
    while let Some(ch) = input.next()? {
        input.advance(1)?;
        if ch == quote && !escaped {
            break;
        }
        escaped = backslash && !escaped && ch == u16::from(b'\\');
    }
    Ok(())
}

fn dollar_literal(input: &mut Input<'_, '_>, tag: &[u16]) -> Result<(), ParseError> {
    'scan: while let Some(ch) = input.next()? {
        input.advance(1)?;
        if ch == u16::from(b'$') {
            for &unit in tag {
                if input.next()? != Some(unit) {
                    continue 'scan;
                }
                input.advance(1)?;
            }
            if input.at(b'$')? {
                input.advance(1)?;
                break;
            }
        }
    }
    Ok(())
}

fn plsql_literal(input: &mut Input<'_, '_>, open: u16) -> Result<(), ParseError> {
    let close = match open {
        91 => 93,   // []
        123 => 125, // {}
        60 => 62,   // <>
        40 => 41,   // ()
        other => other,
    };
    while let Some(ch) = input.next()? {
        if ch == close && input.peek(1)? == Some(u16::from(b'\'')) {
            input.advance(2)?;
            break;
        }
        input.advance(1)?;
    }
    Ok(())
}

fn block_comment(input: &mut Input<'_, '_>) -> Result<(), ParseError> {
    let mut depth = 1;
    while let Some(ch) = input.next()? {
        input.advance(1)?;
        if ch == u16::from(b'*') && input.at(b'/')? {
            input.advance(1)?;
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if ch == u16::from(b'/') && input.at(b'*')? {
            input.advance(1)?;
            depth += 1;
        }
    }
    Ok(())
}

fn bits(input: &mut Input<'_, '_>, quote: Option<u16>) -> Result<(), ParseError> {
    input.eat_while(|c| ascii(c, |c| matches!(c, b'0' | b'1')))?;
    if quote.is_some() && input.next()? == quote {
        input.advance(1)?;
    }
    Ok(())
}

fn number(input: &mut Input<'_, '_>, mut saw_dot: bool) -> Result<(), ParseError> {
    while let Some(ch) = input.next()? {
        if ch == u16::from(b'.') && !saw_dot {
            saw_dot = true;
        } else if !ascii(ch, |c| c.is_ascii_digit()) {
            break;
        }
        input.advance(1)?;
    }
    if input.at(b'e')? || input.at(b'E')? {
        input.advance(1)?;
        if input.at(b'+')? || input.at(b'-')? {
            input.advance(1)?;
        }
        input.eat_while(|c| ascii(c, |c| c.is_ascii_digit()))?;
    }
    Ok(())
}
