// Port of @lezer/python 1.1.20 src/tokens.js. See docs/licenses/codemirror-lang-python.txt.
use std::sync::Arc;

use super::{
    super::lezer::{
        ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, StackView,
        TokenInput, Tokenizer,
    },
    tables::*,
};

const BRACKETED: u8 = 1;
const STRING_FLAG: u8 = 2;
const DOUBLE_QUOTE: u8 = 4;
const LONG: u8 = 8;
const RAW: u8 = 16;
const FORMAT: u8 = 32;
const MAX_CONTEXT_DEPTH: u16 = 256;

#[derive(Clone, Default, Eq, PartialEq)]
pub(super) struct Context(Option<Arc<Frame>>);

#[derive(Eq, PartialEq)]
struct Frame {
    parent: Context,
    indent: usize,
    flags: u8,
    depth: u16,
}

impl Context {
    fn push(&self, indent: usize, flags: u8, offset: usize) -> Result<Option<Self>, ParseError> {
        let depth = self.0.as_ref().map_or(1, |frame| frame.depth + 1);
        if depth > MAX_CONTEXT_DEPTH {
            return Err(ParseError::LimitExceeded { offset });
        }
        Ok(Some(Self(Some(Arc::new(Frame {
            parent: self.clone(),
            indent,
            flags,
            depth,
        })))))
    }

    fn pop(&self) -> Option<Self> {
        self.0.as_ref().map(|frame| frame.parent.clone())
    }

    fn indent(&self) -> usize {
        self.0.as_ref().map_or(0, |frame| frame.indent)
    }

    fn flags(&self) -> u8 {
        self.0.as_ref().map_or(0, |frame| frame.flags)
    }
}

// Frames are immutable. Checkpoints restore the full context, and the
// indentation read during a shift is part of the recorded input extent.
pub(super) fn tracker() -> ContextTracker<Context> {
    ContextTracker::new(Context::default())
        .with_shift(|cx, term, stack, input| match term {
            INDENT => cx.push(count_indent(input)?, 0, stack.byte_offset()),
            DEDENT => Ok(cx.pop()),
            PAREN_L | BRACKET_L | BRACE_L | REPLACEMENT_START => {
                cx.push(0, BRACKETED, stack.byte_offset())
            }
            STRING_START..=STRING_START_FRLD => {
                // The serialized terms vary quote, length, raw, then format.
                let flags = ((term - STRING_START) as u8) << 2;
                cx.push(
                    0,
                    flags | STRING_FLAG | (cx.flags() & BRACKETED),
                    stack.byte_offset(),
                )
            }
            _ => Ok(None),
        })
        .with_reduce(|cx, term, _, _| {
            let flags = cx.flags();
            let bracketed = matches!(
                term,
                PARENTHESIZED_EXPRESSION
                    | TUPLE_EXPRESSION
                    | COMPREHENSION_EXPRESSION
                    | IMPORT_LIST
                    | ARG_LIST
                    | PARAM_LIST
                    | ARRAY_EXPRESSION
                    | ARRAY_COMPREHENSION_EXPRESSION
                    | SUBSCRIPT
                    | SET_EXPRESSION
                    | SET_COMPREHENSION_EXPRESSION
                    | FORMAT_STRING
                    | FORMAT_REPLACEMENT
                    | NESTED_FORMAT_REPLACEMENT
                    | DICTIONARY_EXPRESSION
                    | DICTIONARY_COMPREHENSION_EXPRESSION
                    | SEQUENCE_PATTERN
                    | MAPPING_PATTERN
                    | PATTERN_ARG_LIST
                    | TYPE_PARAM_LIST
            );
            Ok(
                if (flags & BRACKETED != 0 && bracketed)
                    || (flags & STRING_FLAG != 0 && matches!(term, STRING | FORMAT_STRING))
                {
                    cx.pop()
                } else {
                    None
                },
            )
        })
}

fn context<'a>(stack: &'a StackView<'_>) -> &'a Context {
    stack.context().expect("Python grammar context")
}

pub(super) fn tokenizers() -> [Tokenizer; 9] {
    [
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| legacy_print(input),
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            indentation,
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions {
                // The blank-line probe depends on the LR stack, not just the context.
                contextual: true,
                ..ExternalOptions::default()
            },
            newlines,
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            strings,
        )),
        0.into(),
        1.into(),
        2.into(),
        3.into(),
        4.into(),
    ]
}

const LF: u16 = b'\n' as u16;
const CR: u16 = b'\r' as u16;
const HASH: u16 = b'#' as u16;
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;
const OPEN_PAREN: u16 = b'(' as u16;
const DOT: u16 = b'.' as u16;
const OPEN_BRACE: u16 = b'{' as u16;
const CLOSE_BRACE: u16 = b'}' as u16;
const SINGLE_QUOTE: u16 = b'\'' as u16;
const DOUBLE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

fn newlines(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
) -> Result<Option<ExternalToken>, ParseError> {
    let Some(next) = input.peek(0)? else {
        return Ok(Some(input.token(EOF)));
    };
    if context(stack).flags() & BRACKETED != 0 {
        if matches!(next, LF | CR) {
            input.advance(1)?;
            return Ok(Some(input.token(NEWLINE_BRACKETED)));
        }
    } else if matches!(input.peek(-1)?, None | Some(LF | CR))
        && stack.can_shift(BLANK_LINE_START)?
    {
        let blank = input.token(BLANK_LINE_START);
        count_indent(input)?;
        if matches!(input.peek(0)?, Some(LF | CR | HASH)) {
            return Ok(Some(blank));
        }
    } else if matches!(next, LF | CR) {
        input.advance(1)?;
        return Ok(Some(input.token(NEWLINE)));
    }
    Ok(None)
}

fn count_indent(input: &mut TokenInput<'_>) -> Result<usize, ParseError> {
    let mut depth = 0;
    let mut next = input.peek(0)?;
    while let Some(SPACE | TAB) = next {
        depth += if next == Some(TAB) { 8 - depth % 8 } else { 1 };
        next = input.advance(1)?;
    }
    Ok(depth)
}

fn indentation(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
) -> Result<Option<ExternalToken>, ParseError> {
    let cx = context(stack);
    if cx.flags() != 0 || !matches!(input.peek(-1)?, Some(LF | CR)) {
        return Ok(None);
    }
    let dedent = input.token(DEDENT);
    let depth = count_indent(input)?;
    if matches!(input.peek(0)?, Some(LF | CR | HASH)) {
        return Ok(None);
    }
    Ok(match depth.cmp(&cx.indent()) {
        std::cmp::Ordering::Less => Some(dedent),
        std::cmp::Ordering::Greater => Some(input.token(INDENT)),
        std::cmp::Ordering::Equal => None,
    })
}

fn legacy_print(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    for ch in b"print" {
        if input.peek(0)? != Some(u16::from(*ch)) {
            return Ok(None);
        }
        input.advance(1)?;
    }
    let print = input.token(PRINT_KEYWORD);
    let mut next = input.peek(0)?;
    if next
        .and_then(|ch| u8::try_from(ch).ok())
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
    {
        return Ok(None);
    }
    while matches!(next, Some(SPACE | TAB)) {
        next = input.advance(1)?;
    }
    Ok((!matches!(next, Some(OPEN_PAREN | DOT | LF | CR | HASH))).then_some(print))
}

fn strings(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
) -> Result<Option<ExternalToken>, ParseError> {
    let flags = context(stack).flags();
    let quote = if flags & DOUBLE_QUOTE != 0 {
        DOUBLE
    } else {
        SINGLE_QUOTE
    };
    let long = flags & LONG != 0;
    let escapes = flags & RAW == 0;
    let format = flags & FORMAT != 0;
    let start = input.position();
    let mut next = input.peek(0)?;
    while let Some(ch) = next {
        let at_start = input.position() == start;
        if format && ch == OPEN_BRACE {
            if input.peek(1)? == Some(OPEN_BRACE) {
                next = input.advance(2)?;
            } else if at_start {
                input.advance(1)?;
                return Ok(Some(input.token(REPLACEMENT_START)));
            } else {
                break;
            }
        } else if escapes && ch == BACKSLASH {
            if !at_start {
                break;
            }
            if let Some(escaped) = input.advance(1)? {
                // Do not end a native token between UTF-16 surrogate halves.
                let width = if (0xD800..=0xDBFF).contains(&escaped) {
                    2
                } else {
                    1
                };
                input.advance(width)?;
                skip_escape(input, escaped)?;
            }
            return Ok(Some(input.token(ESCAPE)));
        } else if ch == BACKSLASH && !escapes && input.peek(1)?.is_some() {
            next = input.advance(2)?;
        } else if ch == quote
            && (!long || input.peek(1)? == Some(quote) && input.peek(2)? == Some(quote))
        {
            if at_start {
                input.advance(if long { 3 } else { 1 })?;
                return Ok(Some(input.token(STRING_END)));
            }
            break;
        } else if ch == LF && !long {
            if at_start {
                return Ok(Some(input.token(STRING_END)));
            }
            break;
        } else {
            next = input.advance(1)?;
        }
    }
    Ok((input.position() > start).then(|| input.token(STRING_CONTENT)))
}

fn skip_escape(input: &mut TokenInput<'_>, escaped: u16) -> Result<(), ParseError> {
    let len = match u8::try_from(escaped).ok() {
        Some(b'o' | b'x') => 2,
        Some(b'u') => 4,
        Some(b'U') => 8,
        Some(b'N') if input.peek(0)? == Some(OPEN_BRACE) => {
            let mut ch = input.advance(1)?;
            while !matches!(ch, None | Some(CLOSE_BRACE | SINGLE_QUOTE | DOUBLE | LF)) {
                ch = input.advance(1)?;
            }
            if ch == Some(CLOSE_BRACE) {
                input.advance(1)?;
            }
            return Ok(());
        }
        _ => return Ok(()),
    };
    for _ in 0..len {
        let Some(ch) = input.peek(0)?.and_then(|ch| u8::try_from(ch).ok()) else {
            break;
        };
        if if escaped == u16::from(b'o') {
            !(b'0'..=b'7').contains(&ch)
        } else {
            !ch.is_ascii_hexdigit()
        } {
            break;
        }
        input.advance(1)?;
    }
    Ok(())
}
