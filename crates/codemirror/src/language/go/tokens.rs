use super::super::lezer::{
    ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, TokenInput,
    Tokenizer,
};

pub(super) const IDENTIFIER: u16 = 184;
const SPACE: u16 = 179;
const INSERTED_SEMI: u16 = 177;
const BLOCK_COMMENT_OPEN: u16 = 180;
const BLOCK_COMMENT_CONTENT: u16 = 181;
const BLOCK_COMMENT_NEWLINE: u16 = 182;
const BLOCK_COMMENT_END: u16 = 183;

// ContextTracker shift terms from the published Go package.
const TRACKED: &[u16] = &[
    IDENTIFIER, 12, 17, 20, // String, Number, Rune
    13, 25, 53, // ), }, ]
    95, // IncDecOp
    142, 144, 145, 148, // return, break, continue, fallthrough
];

pub(super) fn track_tokens() -> ContextTracker<bool> {
    ContextTracker::new(false).with_shift(|&after_term, term, _, _| {
        let next = if term == SPACE {
            after_term
        } else {
            TRACKED.contains(&term)
        };
        Ok((next != after_term).then_some(next))
    })
}

pub(super) fn tokenizers() -> [Tokenizer; 4] {
    [
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions {
                contextual: true,
                ..ExternalOptions::default()
            },
            |input, stack| semicolon(input, stack.context::<bool>().copied().unwrap_or(false)),
        )),
        1.into(),
        2.into(),
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions::default(),
            |input, _| block_comment(input),
        )),
    ]
}

fn semicolon(
    input: &TokenInput<'_>,
    after_term: bool,
) -> Result<Option<ExternalToken>, ParseError> {
    let mut offset = 0;
    loop {
        let next = input.peek(offset)?;
        if matches!(next, Some(41 | 125))
            || after_term
                && (matches!(next, None | Some(10 | 13))
                    || next == Some(47) && input.peek(offset + 1)? == Some(47))
        {
            return Ok(Some(input.token(INSERTED_SEMI)));
        }
        if !matches!(next, Some(32 | 9)) {
            return Ok(None);
        }
        offset += 1;
    }
}

// The local token group in the published parser has two matches, newline and
// `*/`, and an @else term that consumes everything up to either one or EOF.
fn block_comment(input: &mut TokenInput<'_>) -> Result<Option<ExternalToken>, ParseError> {
    let start = input.position();
    let mut next = input.peek(0)?;
    loop {
        let term = match next {
            None if input.position() == start => return Ok(None),
            None => BLOCK_COMMENT_CONTENT,
            Some(10) if input.position() == start => {
                input.advance(1)?;
                BLOCK_COMMENT_NEWLINE
            }
            Some(42) if input.position() == start && input.peek(1)? == Some(47) => {
                input.advance(2)?;
                BLOCK_COMMENT_END
            }
            Some(10) => BLOCK_COMMENT_CONTENT,
            Some(42) if input.peek(1)? == Some(47) => BLOCK_COMMENT_CONTENT,
            _ => {
                next = input.advance(1)?;
                continue;
            }
        };
        return Ok(Some(input.token(term)));
    }
}

/// Find delimiters that can complete a malformed input. Valid input does not
/// need this scan; shift context follows the actual parser branch.
pub(super) fn recovery_delimiters(data: &[u16], source: &str) -> (usize, String) {
    let mut closes = Vec::new();
    let mut comment = false;
    let mut last_end = 0;
    let mut end = 0;
    while end < source.len() {
        let start = end;
        let rest = &source[start..];
        let (term, len) =
            scan(data, rest).unwrap_or_else(|| (0, rest.chars().next().unwrap().len_utf8()));
        end = start + len;
        if term == SPACE {
            continue;
        }
        if term == BLOCK_COMMENT_OPEN {
            if let Some(close) = source[end..].find("*/") {
                end += close + 2;
            } else {
                end = source.len();
                comment = true;
            }
        }
        match &source.as_bytes()[start..end] {
            b"(" => closes.push(b')'),
            b"[" => closes.push(b']'),
            b"{" => closes.push(b'}'),
            [ch @ (b')' | b']' | b'}')] if closes.last() == Some(ch) => {
                closes.pop();
            }
            _ => {}
        }
        last_end = end;
    }
    let mut suffix = String::new();
    if comment {
        suffix.push_str("*/");
    }
    suffix.extend(closes.into_iter().rev().map(char::from));
    (last_end, suffix)
}

fn scan(data: &[u16], source: &str) -> Option<(u16, usize)> {
    const GROUPS: u16 = (1 << 1) | (1 << 2);
    let mut row = 0;
    let mut found = None;
    let mut chars = source.char_indices();
    let mut low = None;
    let mut end = 0;
    loop {
        if data[row] & GROUPS == 0 {
            break;
        }
        let accept_end = usize::from(data[row + 1]);
        let count = usize::from(data[row + 2]);
        if low.is_none()
            && let Some(&[term, _]) = data[row + 3..accept_end]
                .as_chunks::<2>()
                .0
                .iter()
                .find(|pair| pair[1] & GROUPS != 0)
        {
            found = Some((term, end));
        }
        let unit = if let Some(unit) = low.take() {
            unit
        } else if let Some((byte, ch)) = chars.next() {
            let mut units = [0; 2];
            let units = ch.encode_utf16(&mut units);
            low = units.get(1).copied();
            end = byte + ch.len_utf8();
            units[0]
        } else {
            break;
        };
        let edges = data[accept_end..accept_end + 3 * count].as_chunks::<3>().0;
        let index = edges.partition_point(|&[from, _, _]| from <= unit);
        let Some(&[from, to, next]) = index.checked_sub(1).map(|i| &edges[i]) else {
            break;
        };
        if to != 0 && unit >= to || from == u16::MAX && to == u16::MAX {
            break;
        }
        row = usize::from(next);
    }
    found.filter(|&(_, end)| end != 0)
}
