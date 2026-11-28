use std::sync::Arc;

use super::super::lezer::{
    ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, StackView,
    TokenInput, Tokenizer,
};

const START_TAG: u16 = 1;
const START_CLOSE_TAG: u16 = 2;
const MISSING_CLOSE_TAG: u16 = 3;
const MISMATCHED_START_CLOSE_TAG: u16 = 4;
const INCOMPLETE_START_CLOSE_TAG: u16 = 5;
const ELEMENT: u16 = 11;

// The external tag matcher uses UTF-16 code units and its own name class.
// In particular, it stops at ASCII digits, unlike the TagName token.
fn tag_name(input: &TokenInput<'_>, mut offset: isize) -> Result<Vec<u16>, ParseError> {
    while matches!(input.peek(offset)?, Some(9 | 10 | 13 | 32)) {
        offset += 1;
    }
    let mut name = Vec::new();
    while let Some(ch @ (45 | 46 | 58 | 65..=90 | 95 | 97..=122 | 161..)) = input.peek(offset)? {
        name.push(ch);
        offset += 1;
    }
    Ok(name)
}

#[derive(Clone, Default)]
pub(super) struct Context(Option<Arc<Frame>>);

struct Frame {
    name: Vec<u16>,
    parent: Context,
}

impl PartialEq for Context {
    fn eq(&self, other: &Self) -> bool {
        let (mut a, mut b) = (&self.0, &other.0);
        while let (Some(left), Some(right)) = (a, b) {
            if Arc::ptr_eq(left, right) {
                return true;
            }
            if left.name != right.name {
                return false;
            }
            (a, b) = (&left.parent.0, &right.parent.0);
        }
        a.is_none() && b.is_none()
    }
}

impl Eq for Context {}

impl Drop for Context {
    fn drop(&mut self) {
        // A deep unfinished parse can release an entire unshared chain.
        while let Some(mut frame) = self.0.take().and_then(Arc::into_inner) {
            self.0 = frame.parent.0.take();
        }
    }
}

pub(super) fn tracker() -> ContextTracker<Context> {
    ContextTracker::new(Context::default())
        .with_shift(|context, term, _, input| {
            if term != START_TAG {
                return Ok(None);
            }
            Ok(Some(Context(Some(Arc::new(Frame {
                name: tag_name(input, 1)?,
                parent: context.clone(),
            })))))
        })
        .with_reduce_term(|context, term| {
            context
                .0
                .as_ref()
                .filter(|_| term == ELEMENT)
                .map(|frame| frame.parent.clone())
        })
}

pub(super) fn start_tag() -> Tokenizer {
    Tokenizer::External(ExternalTokenizer::new_deterministic(
        ExternalOptions {
            contextual: true,
            ..ExternalOptions::default()
        },
        scan_tag,
    ))
}

fn scan_tag(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
) -> Result<Option<ExternalToken>, ParseError> {
    if input.peek(0)? != Some(60) {
        return Ok(None);
    }
    let missing = input.token(MISSING_CLOSE_TAG);
    match input.advance(1)? {
        Some(33 | 63) => Ok(None), // ! or ?
        Some(47) => {
            // /
            input.advance(1)?;
            let name = tag_name(input, 0)?;
            if name.is_empty() {
                return Ok(Some(input.token(INCOMPLETE_START_CLOSE_TAG)));
            }
            let mut frame = stack
                .context::<Context>()
                .and_then(|context| context.0.as_deref());
            if let Some(top) = frame {
                if top.name == name {
                    return Ok(Some(input.token(START_CLOSE_TAG)));
                }
                frame = top.parent.0.as_deref();
            }
            while let Some(ancestor) = frame {
                // Charge each ancestor visit to the scan budget. This unit
                // was already read by tag_name.
                input.peek(0)?;
                if ancestor.name == name {
                    return Ok(Some(missing));
                }
                frame = ancestor.parent.0.as_deref();
            }
            Ok(Some(input.token(MISMATCHED_START_CLOSE_TAG)))
        }
        _ => Ok(Some(input.token(START_TAG))),
    }
}

pub(super) fn delimited_content(term: u16, end: &'static [u16]) -> Tokenizer {
    Tokenizer::External(ExternalTokenizer::new_deterministic(
        ExternalOptions::default(),
        move |input, _| {
            let start = input.position();
            let mut next = input.peek(0)?;
            'scan: while let Some(ch) = next {
                if ch == end[0] {
                    for (offset, &unit) in end.iter().enumerate().skip(1) {
                        if input.peek(offset as isize)? != Some(unit) {
                            next = input.advance(1)?;
                            continue 'scan;
                        }
                    }
                    break;
                }
                next = input.advance(1)?;
            }
            Ok((input.position() != start).then(|| input.token(term)))
        },
    ))
}
