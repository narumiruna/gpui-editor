// Port of @lezer/sass 1.1.0 src/tokens.js. See docs/licenses/codemirror-lang-sass.txt.
use std::{cmp::Ordering, sync::Arc};

use super::{
    super::lezer::{
        ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, StackView,
        TokenInput, Tokenizer,
    },
    tables::*,
};

type Scan = Result<Option<ExternalToken>, ParseError>;

// Checkpoints and GLR branches share the immutable indentation chain.
#[derive(Clone, Default, Eq, PartialEq)]
pub(super) struct Context(Option<Arc<Indent>>);

#[derive(Eq, PartialEq)]
struct Indent {
    parent: Context,
    width: usize,
    depth: u16,
}

pub(super) fn tracker() -> ContextTracker<Context> {
    ContextTracker::new(Context::default()).with_shift(|cx, term, stack, input| {
        Ok(match term {
            INDENT => {
                let depth = cx.0.as_ref().map_or(1, |indent| indent.depth + 1);
                if depth > 256 {
                    return Err(ParseError::LimitExceeded {
                        offset: stack.byte_offset(),
                    });
                }
                Some(Context(Some(Arc::new(Indent {
                    parent: cx.clone(),
                    width: count_indent(input)?,
                    depth,
                }))))
            }
            DEDENT => cx.0.as_ref().map(|indent| indent.parent.clone()),
            _ => None,
        })
    })
}

pub(super) fn tokenizers(indented: bool) -> [Tokenizer; 13] {
    let plain = |scan: fn(&mut TokenInput<'_>) -> Scan| {
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, _| scan(input),
        ))
    };
    // The identifier and whitespace hooks probe the LR stack. The indentation
    // hook only reads the context value, which is already part of the cache key.
    let contextual = ExternalOptions {
        contextual: true,
        ..ExternalOptions::default()
    };
    [
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, stack| {
                if indented {
                    indentation(input, stack)
                } else {
                    Ok(None)
                }
            },
        )),
        plain(descendant),
        plain(interpolation_end),
        plain(unit),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            contextual,
            identifiers,
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            contextual,
            move |input, stack| spaces(input, stack, indented),
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, _| comments(input, indented),
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            move |input, _| {
                if indented {
                    let term = match input.peek(0)? {
                        Some(43) => INDENTED_INCLUDE, // +
                        Some(61) => INDENTED_MIXIN,   // =
                        _ => return Ok(None),
                    };
                    input.advance(1)?;
                    return Ok(Some(input.token(term)));
                }
                Ok(None)
            },
        )),
        0.into(),
        1.into(),
        2.into(),
        3.into(),
        4.into(),
    ]
}

const LF: u16 = b'\n' as u16;
const DASH: u16 = b'-' as u16;
const UNDERSCORE: u16 = b'_' as u16;
const HASH: u16 = b'#' as u16;
const SLASH: u16 = b'/' as u16;
const STAR: u16 = b'*' as u16;
const COLON: u16 = b':' as u16;
const OPEN_PAREN: u16 = b'(' as u16;
const OPEN_BRACE: u16 = b'{' as u16;
const CLOSE_BRACE: u16 = b'}' as u16;
const OPEN_BRACKET: u16 = b'[' as u16;
const PERIOD: u16 = b'.' as u16;
const AMPERSAND: u16 = b'&' as u16;
const PERCENT: u16 = b'%' as u16;

fn space(ch: Option<u16>) -> bool {
    matches!(
        ch,
        Some(9..=13 | 32 | 133 | 160 | 5760 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
    )
}

fn alpha(ch: Option<u16>) -> bool {
    matches!(ch, Some(65..=90 | 97..=122 | 161..))
}

fn digit(ch: Option<u16>) -> bool {
    matches!(ch, Some(48..=57))
}

fn starts_comment(input: &TokenInput<'_>) -> Result<bool, ParseError> {
    Ok(input.peek(0)? == Some(SLASH) && matches!(input.peek(1)?, Some(SLASH | STAR)))
}

fn count_indent(input: &mut TokenInput<'_>) -> Result<usize, ParseError> {
    let start = input.position();
    let mut next = input.peek(0)?;
    while next != Some(LF) && space(next) {
        next = input.advance(1)?;
    }
    Ok(input.position() - start)
}

fn spaces(input: &mut TokenInput<'_>, stack: &StackView<'_>, indented: bool) -> Scan {
    let next = input.peek(0)?;
    if indented {
        if next.is_none() && stack.can_shift(EOF)? {
            return Ok(Some(input.token(EOF)));
        }
        if matches!(input.peek(-1)?, None | Some(LF)) && stack.can_shift(BLANK_LINE_START)? {
            let blank = input.token(BLANK_LINE_START);
            let width = count_indent(input)?;
            if input.peek(0)? == Some(LF) || starts_comment(input)? {
                return Ok(Some(blank));
            }
            return Ok((width > 0).then(|| input.token(WHITESPACE)));
        }
        if next == Some(LF) {
            input.advance(1)?;
            return Ok(Some(input.token(NEWLINE)));
        }
        return Ok((count_indent(input)? > 0).then(|| input.token(WHITESPACE)));
    }
    let start = input.position();
    let mut next = next;
    while space(next) {
        next = input.advance(1)?;
    }
    Ok((input.position() > start).then(|| input.token(WHITESPACE)))
}

fn comments(input: &mut TokenInput<'_>, indented: bool) -> Scan {
    if !starts_comment(input)? {
        return Ok(None);
    }
    let block = input.advance(1)? == Some(STAR);
    let term = if block { COMMENT } else { LINE_COMMENT };
    let nested = if indented {
        let mut off = -2;
        loop {
            let prev = input.peek(off)?;
            if matches!(prev, None | Some(LF)) {
                break Some(off.unsigned_abs());
            }
            if !space(prev) {
                break None;
            }
            off -= 1;
        }
    } else {
        None
    };
    input.advance(1)?;
    while let Some(next) = input.peek(0)? {
        if next == LF {
            let end = input.token(term);
            if let Some(min_width) = nested {
                input.advance(1)?;
                if count_indent(input)? < min_width {
                    return Ok(Some(end));
                }
                continue;
            }
            if !block {
                break;
            }
        }
        if block && next == STAR && input.peek(1)? == Some(SLASH) {
            input.advance(2)?;
            break;
        }
        input.advance(1)?;
    }
    Ok(Some(input.token(term)))
}

fn indentation(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> Scan {
    let cx = stack.context::<Context>().expect("indented Sass context");
    let width = cx.0.as_ref().map_or(0, |indent| indent.width);
    let dedent = input.token(DEDENT);
    if input.peek(0)?.is_none() && width != 0 {
        return Ok(Some(dedent));
    }
    if input.peek(-1)? != Some(LF) {
        return Ok(None);
    }
    let next_width = count_indent(input)?;
    if next_width == width || input.peek(0)? == Some(LF) || starts_comment(input)? {
        return Ok(None);
    }
    Ok(match next_width.cmp(&width) {
        Ordering::Less => Some(dedent),
        Ordering::Greater => Some(input.token(INDENT)),
        Ordering::Equal => None,
    })
}

fn identifiers(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> Scan {
    let start = input.position();
    let mut inside = false;
    let mut dashes = 0;
    loop {
        let next = input.peek(0)?;
        let seen = input.position() - start;
        if alpha(next) || matches!(next, Some(DASH | UNDERSCORE)) || inside && digit(next) {
            if next == Some(DASH) && dashes == seen {
                dashes += 1;
            }
            inside |= next != Some(DASH) || seen > 0;
            input.advance(1)?;
        } else if next == Some(HASH) && input.peek(1)? == Some(OPEN_BRACE) {
            input.advance(2)?;
            return Ok(Some(input.token(INTERPOLATION_START)));
        } else if inside {
            let term = if dashes == 2 && stack.can_shift(VARIABLE_NAME)? {
                VARIABLE_NAME
            } else if stack.can_shift(QUERY_IDENTIFIER)? {
                QUERY_IDENTIFIER
            } else if next == Some(OPEN_PAREN) {
                CALLEE
            } else {
                IDENTIFIER
            };
            return Ok(Some(input.token(term)));
        } else {
            return Ok(None);
        }
    }
}

fn interpolation_end(input: &mut TokenInput<'_>) -> Scan {
    if input.peek(0)? != Some(CLOSE_BRACE) {
        return Ok(None);
    }
    let mut next = input.advance(1)?;
    while alpha(next) || digit(next) || matches!(next, Some(DASH | UNDERSCORE)) {
        next = input.advance(1)?;
    }
    let term = if next == Some(HASH) && input.peek(1)? == Some(OPEN_BRACE) {
        input.advance(2)?;
        INTERPOLATION_CONTINUE
    } else {
        INTERPOLATION_END
    };
    Ok(Some(input.token(term)))
}

fn descendant(input: &mut TokenInput<'_>) -> Scan {
    if !space(input.peek(-1)?) {
        return Ok(None);
    }
    let next = input.peek(0)?;
    let found = alpha(next)
        || matches!(
            next,
            Some(UNDERSCORE | HASH | PERIOD | OPEN_BRACKET | DASH | AMPERSAND | STAR)
        )
        || next == Some(COLON) && alpha(input.peek(1)?);
    Ok(found.then(|| input.token(DESCENDANT_OP)))
}

fn unit(input: &mut TokenInput<'_>) -> Scan {
    if space(input.peek(-1)?) {
        return Ok(None);
    }
    let next = input.peek(0)?;
    if next == Some(PERCENT) {
        input.advance(1)?;
    } else if alpha(next) {
        let mut next = input.advance(1)?;
        while alpha(next) || digit(next) {
            next = input.advance(1)?;
        }
    } else {
        return Ok(None);
    }
    Ok(Some(input.token(UNIT)))
}
