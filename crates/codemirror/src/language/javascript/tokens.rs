// Tokenizers from @lezer/javascript 1.5.7. See the upstream license notice.
use super::super::lezer::{
    ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, StackView,
    TokenInput, Tokenizer,
};
use super::tables;

const INCDEC: u16 = 1;
const INCDEC_PREFIX: u16 = 2;
const QUESTION_DOT: u16 = 3;
const JSX_START_TAG: u16 = 4;
const NO_SEMI: u16 = 317;
const NO_SEMI_TYPE: u16 = 318;
const INSERT_SEMI: u16 = 319;
const LINE_COMMENT: u16 = 5;
const BLOCK_COMMENT: u16 = 6;
const SPACES: u16 = 321;
const NEWLINE: u16 = 322;

pub(super) fn track_newline() -> ContextTracker<bool> {
    ContextTracker::new(false).with_shift(|&after_newline, term, _, _| {
        let next = match term {
            LINE_COMMENT | BLOCK_COMMENT | SPACES => after_newline,
            _ => term == NEWLINE,
        };
        Ok((next != after_newline).then_some(next))
    })
}

fn after_newline(stack: &StackView<'_>) -> bool {
    stack.context::<bool>().copied().unwrap_or(false)
}

pub(super) fn tokenizers(jsx_enabled: bool) -> [Tokenizer; 20] {
    let contextual = ExternalOptions {
        contextual: true,
        ..ExternalOptions::default()
    };
    [
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            contextual,
            no_semicolon,
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            contextual,
            |input, stack| {
                Ok((input.peek(0)? == unit(b'[') && !after_newline(stack))
                    .then(|| input.token(NO_SEMI_TYPE)))
            },
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(contextual, operator)),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, _| {
                if jsx_enabled { jsx(input) } else { Ok(None) }
            },
        )),
        2.into(),
        3.into(),
        4.into(),
        5.into(),
        6.into(),
        7.into(),
        8.into(),
        9.into(),
        10.into(),
        11.into(),
        12.into(),
        13.into(),
        14.into(),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions {
                fallback: true,
                ..contextual
            },
            |input, stack| {
                Ok(
                    (matches!(input.peek(0)?, None | Some(125)) || after_newline(stack))
                        .then(|| input.token(INSERT_SEMI)),
                )
            },
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| local(input, tables::TEMPLATE_DFA, 141, 341),
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| local(input, tables::COMMENT_DFA, 25, 324),
        )),
    ]
}

const fn unit(ascii: u8) -> Option<u16> {
    Some(ascii as u16)
}

fn space(ch: Option<u16>) -> bool {
    matches!(
        ch,
        Some(9..=13 | 32 | 133 | 160 | 5760 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
    )
}

fn no_semicolon(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
) -> Result<Option<ExternalToken>, ParseError> {
    let ch = input.peek(0)?;
    if space(ch)
        || (ch == unit(b'/') && matches!(input.peek(1)?, Some(42 | 47)))
        || matches!(ch, None | Some(59 | 125))
        || after_newline(stack)
    {
        Ok(None)
    } else {
        Ok(Some(input.token(NO_SEMI)))
    }
}

fn operator(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
) -> Result<Option<ExternalToken>, ParseError> {
    let ch = input.peek(0)?;
    let term = if matches!(ch, Some(43 | 45)) && input.peek(1)? == ch {
        if !after_newline(stack) && stack.can_shift(INCDEC)? {
            INCDEC
        } else {
            INCDEC_PREFIX
        }
    } else if ch == unit(b'?')
        && input.peek(1)? == unit(b'.')
        && !matches!(input.peek(2)?, Some(48..=57))
    {
        QUESTION_DOT
    } else {
        return Ok(None);
    };
    input.advance(2)?;
    Ok(Some(input.token(term)))
}

fn identifier(ch: Option<u16>, start: bool) -> bool {
    matches!(ch, Some(65..=90 | 95 | 97..=122 | 192..)) || (!start && matches!(ch, Some(48..=57)))
}

fn jsx(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    if input.peek(0)? != unit(b'<') || input.peek(1)? == unit(b'/') {
        return Ok(None);
    }
    // Do not take the opening angle from a generic arrow function.
    let mut offset = 1;
    while space(input.peek(offset)?) {
        offset += 1;
    }
    if identifier(input.peek(offset)?, true) {
        offset += 1;
        while identifier(input.peek(offset)?, false) {
            offset += 1;
        }
        while space(input.peek(offset)?) {
            offset += 1;
        }
        if input.peek(offset)? == unit(b',') {
            return Ok(None);
        }
        let mut extends = true;
        for ch in b"extends" {
            if input.peek(offset)? != unit(*ch) {
                extends = false;
                break;
            }
            offset += 1;
        }
        if extends && !identifier(input.peek(offset)?, true) {
            return Ok(None);
        }
    }
    input.advance(1)?;
    Ok(Some(input.token(JSX_START_TAG)))
}

// These local groups have one DFA group and no competing accepting terms.
// If nothing matches, collect the text up to the next recognizable token.
fn local(
    input: &mut TokenInput<'_>,
    data: &[u16],
    precedence: usize,
    otherwise: u16,
) -> Result<Option<ExternalToken>, ParseError> {
    debug_assert_eq!(data[precedence], u16::MAX);
    let mut skipped = 0;
    while let Some(first) = input.peek(skipped)? {
        let mut at = 0;
        let mut offset = skipped;
        let mut found = None;
        let mut ch = Some(first);
        loop {
            let accept_end = usize::from(data[at + 1]);
            if accept_end > at + 3 {
                found = Some((data[at + 3], offset));
            }
            let Some(unit) = ch else { break };
            let count = usize::from(data[at + 2]);
            let (edges, _) = data[accept_end..].as_chunks::<3>();
            let Some(&[_, _, target]) = edges[..count]
                .iter()
                .find(|&&[from, to, _]| unit >= from && (to == 0 || unit < to))
            else {
                break;
            };
            at = usize::from(target);
            offset += 1;
            ch = input.peek(offset)?;
        }
        if let Some((term, end)) = found {
            let (term, end) = if skipped == 0 {
                (term, end)
            } else {
                (otherwise, skipped)
            };
            input.advance(end as usize)?;
            return Ok(Some(input.token(term)));
        }
        // A local token cannot start at a low surrogate in valid UTF-8.
        skipped += if (0xd800..=0xdbff).contains(&first) {
            2
        } else {
            1
        };
    }
    if skipped == 0 {
        Ok(None)
    } else {
        input.advance(skipped as usize)?;
        Ok(Some(input.token(otherwise)))
    }
}
