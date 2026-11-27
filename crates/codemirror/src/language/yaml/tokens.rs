// Based on @lezer/yaml 1.0.4 src/tokens.js. See docs/licenses/codemirror-lang-yaml.txt.

use std::sync::Arc;

use super::super::lezer::{
    ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, StackView,
    TokenInput, Tokenizer,
};

mod scalar;

use scalar::{read_anchor, read_plain, read_quoted, read_tag};

// Terms from src/parser.terms.js.
const DIRECTIVE_END: u16 = 1;
const DOC_END: u16 = 2;
const SEQUENCE_START: u16 = 3;
const SEQUENCE_CONTINUE: u16 = 4;
const EXPLICIT_MAP_START: u16 = 5;
const EXPLICIT_MAP_CONTINUE: u16 = 6;
const FLOW_MAP_MARK: u16 = 7;
const LITERAL: u16 = 8;
const QUOTED_LITERAL: u16 = 9;
const ANCHOR: u16 = 10;
const ALIAS: u16 = 11;
const TAG: u16 = 12;
const BLOCK_LITERAL_CONTENT: u16 = 13;
const BRACKET_L: u16 = 19;
const FLOW_SEQUENCE: u16 = 20;
const COLON: u16 = 29;
const BRACE_L: u16 = 33;
const FLOW_MAPPING: u16 = 34;
const BLOCK_LITERAL_HEADER: u16 = 47;
const BLOCK_END: u16 = 63;
const EOF: u16 = 64;
const MAP_START: u16 = 65;
const MAP_CONTINUE: u16 = 66;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Top,
    Sequence,
    Mapping,
    Flow,
    Literal,
}

#[derive(Eq, PartialEq)]
pub(super) struct Context {
    parent: Option<Arc<Self>>,
    column: isize,
    kind: Kind,
    depth: usize,
}

impl Context {
    fn push(
        self: &Arc<Self>,
        column: isize,
        kind: Kind,
        offset: usize,
    ) -> Result<Arc<Self>, ParseError> {
        if self.depth == 256 {
            return Err(ParseError::LimitExceeded { offset });
        }
        Ok(Arc::new(Self {
            parent: Some(Arc::clone(self)),
            column,
            kind,
            depth: self.depth + 1,
        }))
    }
}

pub(super) fn tracker() -> ContextTracker<Arc<Context>> {
    ContextTracker::new(Arc::new(Context {
        parent: None,
        column: -1,
        kind: Kind::Top,
        depth: 0,
    }))
    .with_shift(|cx, term, stack, input| {
        let offset = stack.byte_offset();
        let (column, kind) = match term {
            SEQUENCE_START => (column_at(input, input.position())?, Kind::Sequence),
            MAP_START | EXPLICIT_MAP_START => (column_at(input, input.position())?, Kind::Mapping),
            BRACKET_L | BRACE_L => (0, Kind::Flow),
            BLOCK_END => return Ok(cx.parent.clone()),
            BLOCK_LITERAL_CONTENT if cx.kind == Kind::Literal => return Ok(cx.parent.clone()),
            BLOCK_LITERAL_HEADER => {
                let mut digit = peek(input, 1)?;
                if matches!(digit, Some('+' | '-')) {
                    digit = peek(input, 2)?;
                }
                let Some(indent @ '1'..='9') = digit else {
                    return Ok(None);
                };
                (cx.column + (indent as isize - '0' as isize), Kind::Literal)
            }
            _ => return Ok(None),
        };
        Ok(Some(cx.push(column, kind, offset)?))
    })
    .with_reduce(|cx, term, _, _| {
        Ok(
            if cx.kind == Kind::Flow && matches!(term, FLOW_SEQUENCE | FLOW_MAPPING) {
                cx.parent.clone()
            } else {
                None
            },
        )
    })
}

pub(super) fn tokenizers() -> [Tokenizer; 6] {
    let contextual = ExternalOptions {
        contextual: true,
        ..Default::default()
    };
    [
        Tokenizer::External(ExternalTokenizer::new_deterministic(contextual, newlines)),
        Tokenizer::External(ExternalTokenizer::new_deterministic(contextual, block_mark)),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            Default::default(),
            literals,
        )),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            Default::default(),
            block_literal,
        )),
        Tokenizer::Internal(0),
        Tokenizer::Internal(1),
    ]
}

type ScanResult = Result<Option<ExternalToken>, ParseError>;

fn context<'a>(stack: &'a StackView<'_>) -> &'a Context {
    stack
        .context::<Arc<Context>>()
        .expect("YAML indentation tracker")
}

fn peek(input: &TokenInput<'_>, offset: isize) -> Result<Option<char>, ParseError> {
    // Surrogate halves have the same classification as any non-ASCII unit.
    Ok(input
        .peek(offset)?
        .map(|unit| char::from_u32(unit.into()).unwrap_or('\u{fffd}')))
}

fn horizontal(ch: Option<char>) -> bool {
    matches!(ch, Some(' ' | '\t'))
}

fn line_break(ch: Option<char>) -> bool {
    matches!(ch, Some('\n' | '\r'))
}

fn space(ch: Option<char>) -> bool {
    horizontal(ch) || line_break(ch)
}

fn separator(ch: Option<char>) -> bool {
    ch.is_none() || space(ch)
}

fn column_at(input: &TokenInput<'_>, start: usize) -> Result<isize, ParseError> {
    let offset = start as isize - input.position() as isize;
    let mut column = 0;
    loop {
        let ch = peek(input, offset - column - 1)?;
        if ch.is_none() || line_break(ch) {
            return Ok(column);
        }
        column += 1;
    }
}

fn three(input: &TokenInput<'_>, ch: char, at: isize) -> Result<bool, ParseError> {
    Ok(peek(input, at)? == Some(ch)
        && peek(input, at + 1)? == Some(ch)
        && peek(input, at + 2)? == Some(ch)
        && separator(peek(input, at + 3)?))
}

fn newlines(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> ScanResult {
    if peek(input, 0)?.is_none() && stack.can_shift(EOF)? {
        return Ok(Some(input.token(EOF)));
    }
    let cx = context(stack);
    let prev = peek(input, -1)?;
    if cx.kind == Kind::Flow || (prev.is_some() && !line_break(prev)) {
        return Ok(None);
    }
    let start = input.position();
    for (ch, term) in [('-', DIRECTIVE_END), ('.', DOC_END)] {
        if three(input, ch, 0)? {
            return Ok(Some(if stack.can_shift(BLOCK_END)? {
                input.token(BLOCK_END)
            } else {
                ExternalToken {
                    term,
                    end: start + 3,
                }
            }));
        }
    }
    let mut column = 0;
    while peek(input, 0)? == Some(' ') {
        column += 1;
        input.advance(1)?;
    }
    let next = peek(input, 0)?;
    if (column < cx.column
        || column == cx.column
            && cx.kind == Kind::Sequence
            && (next != Some('-') || !separator(peek(input, 1)?)))
        && next.is_some()
        && !line_break(next)
        && next != Some('#')
    {
        return Ok(Some(ExternalToken {
            term: BLOCK_END,
            end: start,
        }));
    }
    Ok(None)
}

fn block_mark(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> ScanResult {
    let cx = context(stack);
    let next = peek(input, 0)?;
    if cx.kind == Kind::Flow {
        if next == Some('?') && separator(peek(input, 1)?) {
            input.advance(1)?;
            return Ok(Some(input.token(FLOW_MAP_MARK)));
        }
        return Ok(None);
    }
    if let Some(ch @ ('-' | '?')) = next {
        if !separator(peek(input, 1)?) {
            return Ok(None);
        }
        let (kind, start, cont) = if ch == '-' {
            (Kind::Sequence, SEQUENCE_START, SEQUENCE_CONTINUE)
        } else {
            (Kind::Mapping, EXPLICIT_MAP_START, EXPLICIT_MAP_CONTINUE)
        };
        let term = if cx.kind == kind && cx.column == column_at(input, input.position())? {
            cont
        } else {
            start
        };
        input.advance(1)?;
        return Ok(Some(input.token(term)));
    }
    let start = input.position();
    loop {
        match peek(input, 0)? {
            Some(' ' | '\t') if input.position() == start => return Ok(None),
            Some(' ' | '\t') => {
                input.advance(1)?;
            }
            Some('!') => read_tag(input)?,
            Some('&') => read_anchor(input)?,
            Some('*') => {
                read_anchor(input)?;
                break;
            }
            Some('\'' | '"') => {
                if !read_quoted(input, true)? {
                    return Ok(None);
                }
                break;
            }
            Some('[' | '{') => {
                if !scan_brackets(input)? {
                    return Ok(None);
                }
                break;
            }
            _ => {
                read_plain(input, true, false, 0)?;
                break;
            }
        }
    }
    while horizontal(peek(input, 0)?) {
        input.advance(1)?;
    }
    if peek(input, 0)? == Some(':')
        && (input.position() != start || !stack.can_shift(COLON)?)
        && separator(peek(input, 1)?)
    {
        let term = if cx.kind == Kind::Mapping && column_at(input, start)? == cx.column {
            MAP_CONTINUE
        } else {
            MAP_START
        };
        return Ok(Some(ExternalToken { term, end: start }));
    }
    Ok(None)
}

fn scan_brackets(input: &mut TokenInput<'_>) -> Result<bool, ParseError> {
    let mut closers = Vec::new();
    let end = input.position() + 1024;
    loop {
        match peek(input, 0)? {
            Some('[') => closers.push(']'),
            Some('{') => closers.push('}'),
            Some('\'' | '"') => {
                if !read_quoted(input, true)? {
                    return Ok(false);
                }
                continue;
            }
            Some(ch @ (']' | '}')) => {
                if closers.pop() != Some(ch) {
                    return Ok(false);
                }
                if closers.is_empty() {
                    input.advance(1)?;
                    return Ok(true);
                }
            }
            None | Some('\n' | '\r') => return Ok(false),
            _ if input.position() > end => return Ok(false),
            _ => {}
        }
        input.advance(1)?;
    }
}

fn literals(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> ScanResult {
    let cx = context(stack);
    let term = match peek(input, 0)? {
        Some('!') => {
            read_tag(input)?;
            TAG
        }
        Some(ch @ ('&' | '*')) => {
            read_anchor(input)?;
            if ch == '&' { ANCHOR } else { ALIAS }
        }
        Some('\'' | '"') => {
            read_quoted(input, false)?;
            QUOTED_LITERAL
        }
        _ if read_plain(input, false, cx.kind == Kind::Flow, cx.column)? => LITERAL,
        _ => return Ok(None),
    };
    Ok(Some(input.token(term)))
}

fn block_literal(input: &mut TokenInput<'_>, stack: &StackView<'_>) -> ScanResult {
    let cx = context(stack);
    let mut indent = if cx.kind == Kind::Literal {
        cx.column
    } else {
        -1
    };
    let mut end = input.position();
    'scan: loop {
        let mut depth = 0;
        let mut next = peek(input, 0)?;
        while next == Some(' ') {
            depth += 1;
            next = peek(input, depth)?;
        }
        if depth == 0 && (three(input, '-', 0)? || three(input, '.', 0)?) {
            break;
        }
        if !line_break(next) {
            if indent < 0 {
                indent = (cx.column + 1).max(depth);
            }
            if depth < indent {
                break;
            }
        }
        loop {
            let Some(ch) = peek(input, 0)? else {
                break 'scan;
            };
            input.advance(1)?;
            if line_break(Some(ch)) {
                continue 'scan;
            }
            end = input.position();
        }
    }
    Ok(Some(ExternalToken {
        term: BLOCK_LITERAL_CONTENT,
        end,
    }))
}
