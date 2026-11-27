//! Element contexts and tag delimiters from @lezer/html 1.3.13.

use std::sync::Arc;

use super::{
    super::lezer::{ContextTracker, ExternalToken, ParseError, StackView, TokenInput},
    lexer,
};

pub(super) type Token = Result<Option<ExternalToken>, ParseError>;

#[derive(Clone, Default)]
struct Elements(Option<Arc<Frame>>);

struct Frame {
    name: String,
    foreign: bool,
    parent: Option<Arc<Frame>>,
}

impl Drop for Frame {
    fn drop(&mut self) {
        // A rejected parse can drop a deep context in one operation.
        let mut parent = self.parent.take();
        while let Some(frame) = parent {
            let Some(mut frame) = Arc::into_inner(frame) else {
                break;
            };
            parent = frame.parent.take();
        }
    }
}

impl PartialEq for Elements {
    fn eq(&self, other: &Self) -> bool {
        let (mut left, mut right) = (self.0.as_ref(), other.0.as_ref());
        while let (Some(a), Some(b)) = (left, right) {
            if Arc::ptr_eq(a, b) {
                return true;
            }
            if a.name != b.name {
                return false;
            }
            left = a.parent.as_ref();
            right = b.parent.as_ref();
        }
        left.is_none() && right.is_none()
    }
}

impl Eq for Elements {}

impl Elements {
    fn push(&self, name: String) -> Self {
        let foreign =
            lexer::is_foreign(&name) || self.0.as_ref().is_some_and(|frame| frame.foreign);
        Self(Some(Arc::new(Frame {
            name,
            foreign,
            parent: self.0.clone(),
        })))
    }
}

pub(super) fn install(parser: super::super::lezer::Parser) -> super::super::lezer::Parser {
    // Shift and reduce create immutable frames from metered input and prior context.
    parser.with_deterministic_context_tracker(
        ContextTracker::new(Elements::default())
            .with_shift(|context, term, _, input| {
                if (6..=10).contains(&term) {
                    let (name, _) = tag_name(input, 1)?;
                    Ok(Some(context.push(name)))
                } else {
                    Ok(None)
                }
            })
            .with_reduce(|context, term, _, _| {
                Ok((term == 21)
                    .then(|| Elements(context.0.as_ref().and_then(|frame| frame.parent.clone()))))
            }),
    )
}

fn tag_name(
    input: &TokenInput<'_>,
    mut offset: isize,
) -> Result<(String, Option<u16>), ParseError> {
    let mut units = Vec::new();
    let mut next = input.peek(offset)?;
    while let Some(ch @ (45 | 46 | 58 | 65..=90 | 95 | 97..=122 | 161..=u16::MAX)) = next {
        units.push(ch);
        offset += 1;
        next = input.peek(offset)?;
    }
    Ok((String::from_utf16_lossy(&units).to_lowercase(), next))
}

pub(super) fn start(
    input: &mut TokenInput<'_>,
    stack: &StackView<'_>,
    match_closing: bool,
) -> Token {
    let current = stack
        .context::<Elements>()
        .and_then(|elements| elements.0.as_deref());
    let Some(unit) = input.peek(0)? else {
        return Ok(current.is_some().then(|| input.token(58)));
    };
    if unit != 60 {
        return Ok(None);
    }
    let offset = input.position();
    let closed = input.advance(1)? == Some(47);
    if closed {
        input.advance(1)?;
    }
    let (name, next) = tag_name(input, 0)?;
    let missing = || {
        Ok(Some(ExternalToken {
            term: 58,
            end: offset,
        }))
    };
    let term = if name.is_empty() {
        if matches!(next, Some(33 | 63)) {
            return Ok(None);
        }
        if closed { 15 } else { 14 }
    } else if closed {
        if current.is_some_and(|frame| frame.name == name) {
            11
        } else if current.is_some_and(|frame| lexer::implicitly_closed(&frame.name)) {
            return missing();
        } else if !match_closing {
            12
        } else if std::iter::successors(current, |frame| frame.parent.as_deref())
            .any(|frame| frame.name == name)
        {
            // The current element has not closed, but this tag closes an ancestor.
            return Ok(None);
        } else {
            13
        }
    } else {
        match name.as_str() {
            "script" => 7,
            "style" => 8,
            "textarea" => 9,
            name if lexer::is_void(name) => 10,
            name if current.is_some_and(|frame| lexer::close_on_open(&frame.name, name)) => {
                return missing();
            }
            _ => 6,
        }
    };
    Ok(Some(input.token(term)))
}

pub(super) fn end(input: &mut TokenInput<'_>, stack: &StackView<'_>, self_closing: bool) -> Token {
    let (term, len) = match input.peek(0)? {
        Some(62) => (4, 1),
        Some(47) if input.peek(1)? == Some(62) => {
            let foreign = stack
                .context::<Elements>()
                .and_then(|elements| elements.0.as_deref())
                .is_some_and(|frame| frame.foreign);
            (if self_closing || foreign { 5 } else { 4 }, 2)
        }
        _ => return Ok(None),
    };
    input.advance(len)?;
    Ok(Some(input.token(term)))
}
