//! Logical subword motion from `@codemirror/commands` 6.11.2.
//!
//! Motion follows document order, not visual BiDi order, and does not consult
//! wrapping, cursor affinity, or atomic view ranges. The native commands do
//! not use the optional `Intl.Segmenter` pass or its platform dictionaries.

use super::{
    Backward, CommandResult, Direction, EditorState, Forward, GraphemeCursor, SelectionRange,
    caret, select, selection_by,
};
use crate::{CharCategorizer, CharCategory};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Space,
    Word,
    WordPunctuation,
    Other,
}

impl Category {
    fn of(categorizer: &CharCategorizer, cluster: &str) -> Self {
        match categorizer.category(cluster) {
            CharCategory::Space => Self::Space,
            CharCategory::Other => Self::Other,
            // The upstream ASCII test checks the whole cluster for punctuation,
            // including underscores and extending marks on an ASCII base.
            CharCategory::Word
                if cluster.as_bytes()[0].is_ascii()
                    && !cluster.bytes().all(|byte| byte.is_ascii_alphanumeric()) =>
            {
                Self::WordPunctuation
            }
            CharCategory::Word => Self::Word,
        }
    }
}

fn lowercase_form(text: &str) -> bool {
    text.chars()
        .all(|ch| ch.to_lowercase().eq(std::iter::once(ch)))
}

fn logical_code_units(source: &str) -> impl Iterator<Item = u16> + '_ {
    // CodeMirror counts a line break as one UTF-16 code unit even in CRLF text.
    let mut units = source.encode_utf16().peekable();
    std::iter::from_fn(move || {
        let unit = units.next()?;
        if unit == u16::from(b'\r') && units.peek() == Some(&u16::from(b'\n')) {
            return units.next();
        }
        Some(unit)
    })
}

fn is_lower_word(categorizer: &CharCategorizer, unit: u16) -> bool {
    categorizer.is_word_unit(unit)
        && char::from_u32(unit.into()).is_none_or(|ch| ch.to_lowercase().eq(std::iter::once(ch)))
}

fn boundary(state: &EditorState, range: SelectionRange, direction: Direction) -> usize {
    let start = range.range().start;
    let categorizer = state.char_categorizer(start);
    let source = state.text().as_str();
    let mut pos = range.head();
    let mut cursor = GraphemeCursor::new(pos, source.len(), true);
    let edge = direction.edge(&(0..source.len()));
    let mut category = Category::Space;
    let mut previous_lower = None;
    // Motion starts at the head, but the upstream acronym lookahead advances
    // from the lower selection endpoint by the traversed UTF-16 length.
    let mut ahead = logical_code_units(&source[start..]).peekable();
    while pos != edge {
        let next = direction.step(source, &mut cursor);
        let cluster = &source[pos.min(next)..pos.max(next)];
        let current = Category::of(&categorizer, cluster);
        if category == Category::Space {
            category = current;
        }
        if current != category {
            break;
        }
        if matches!(direction, Forward) {
            let width = logical_code_units(cluster).count();
            ahead.nth(width - 1);
        }
        if current == Category::Word {
            let lower = lowercase_form(cluster);
            match (direction, previous_lower, lower) {
                (Forward, Some(true), false) | (Backward, Some(false), true) => break,
                (Forward, Some(false), false)
                    if ahead
                        .peek()
                        .is_some_and(|&unit| is_lower_word(&categorizer, unit)) =>
                {
                    break;
                }
                (Backward, Some(true), false) => return next,
                _ => previous_lower = Some(lower),
            }
        }
        pos = next;
    }
    pos
}

fn subword(state: &EditorState, direction: Direction, extend: bool) -> CommandResult {
    let selection = selection_by(state, |range| {
        if extend {
            SelectionRange::new(range.anchor(), boundary(state, range, direction))
        } else if range.range().is_empty() {
            caret(boundary(state, range, direction))
        } else {
            caret(direction.edge(&range.range()))
        }
    })?;
    Ok((&selection != state.selection()).then(|| select(selection, true)))
}

/// Move forward by a logical subword. Collapse nonempty ranges to their end.
///
/// Skip leading whitespace, then move over one camel-case subword, a run of
/// word punctuation such as underscores, or a run of other punctuation.
/// Use document order without visual BiDi, wrapping, or dictionary boundaries.
pub fn cursor_subword_forward(state: &EditorState) -> CommandResult {
    subword(state, Forward, false)
}

/// Move backward by a logical subword. Collapse nonempty ranges to their start.
pub fn cursor_subword_backward(state: &EditorState) -> CommandResult {
    subword(state, Backward, false)
}

/// Move each selection head forward by a logical subword, keeping its anchor.
pub fn select_subword_forward(state: &EditorState) -> CommandResult {
    subword(state, Forward, true)
}

/// Move each selection head backward by a logical subword, keeping its anchor.
pub fn select_subword_backward(state: &EditorState) -> CommandResult {
    subword(state, Backward, true)
}
