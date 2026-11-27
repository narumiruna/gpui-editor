//! Comment commands adapted from `@codemirror/commands` 6.11.2.
//! See `docs/licenses/codemirror-commands.txt`.

use super::{lines::LineEditText, *};
use crate::{Assoc, MapMode, language::line_projection::command_line_projection};

/// Comment markers supplied by the language at a document position.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommentTokens<'a> {
    pub line: Option<&'a str>,
    pub block: Option<(&'a str, &'a str)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommentMode {
    Toggle,
    Comment,
    Uncomment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommentStyle {
    /// Use line comments if the main line supports them, or block comments by line.
    PreferLine,
    Line,
    Block,
    BlockByLine,
}

/// Build a comment transaction. The caller supplies the language's tokens at
/// each requested UTF-8 byte offset. Markers must be nonempty.
pub fn comment<'a>(
    state: &EditorState,
    mode: CommentMode,
    style: CommentStyle,
    tokens_at: impl Fn(usize) -> CommentTokens<'a>,
) -> CommandResult {
    let projection = command_line_projection(state);
    comment_in_lines(state, mode, style, projection.as_ref(), tokens_at)
}

fn comment_in_lines<'a>(
    state: &EditorState,
    mode: CommentMode,
    style: CommentStyle,
    projection: Option<&Text>,
    tokens_at: impl Fn(usize) -> CommentTokens<'a>,
) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let text = LineEditText::new(state.text(), projection)?;
    let style = if style == CommentStyle::PreferLine {
        let main_line = line_at(text.lines, state.selection().main().range().start);
        let tokens = tokens_at(main_line.range.start);
        if tokens.line.is_some_and(|marker| !marker.is_empty()) {
            CommentStyle::Line
        } else if tokens.block.is_some() {
            CommentStyle::BlockByLine
        } else {
            return Ok(None);
        }
    } else {
        style
    };
    let transaction = match style {
        CommentStyle::Line => line_comment(state, &text, mode, tokens_at),
        _ => block_comment(
            state,
            &text,
            mode,
            style == CommentStyle::BlockByLine,
            tokens_at,
        ),
    }?;
    Ok(transaction.filter(|tx| {
        projection.is_none()
            || (tx.changes().changes().all(|(range, _)| {
                !inside_crlf(text.raw, range.start) && !inside_crlf(text.raw, range.end)
            }) && text.keeps_breaks(tx.changes()))
    }))
}

/// Use the first `commentTokens` language datum at each queried position.
/// At a language boundary request the data for the following text.
pub fn comment_with_language_data(
    state: &EditorState,
    mode: CommentMode,
    style: CommentStyle,
) -> CommandResult {
    comment(state, mode, style, |position| {
        comment_tokens(state, position)
    })
}

/// Use a same-width editor line view that treats lone CR as a break. Query
/// metadata and build changes in the original state's UTF-8 coordinates.
/// Reject edits that split a CRLF or join two distinct breaks.
pub fn comment_with_language_data_in_lines(
    state: &EditorState,
    mode: CommentMode,
    style: CommentStyle,
    lines: &Text,
) -> CommandResult {
    comment_in_lines(state, mode, style, Some(lines), |position| {
        comment_tokens(state, position)
    })
}

fn comment_tokens(state: &EditorState, position: usize) -> CommentTokens<'static> {
    state
        .language_data_at::<CommentTokens<'static>>("commentTokens", position, Assoc::After)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn touched_last_line<'a>(text: &LineEditText<'a>, selection: &Range<usize>) -> Line<'a> {
    let end = line_at(text.lines, selection.end);
    if end.range.start == selection.end && !selection.is_empty() {
        let previous_end = selection.end - text.break_before(selection.end).len();
        line_at(text.lines, previous_end)
    } else {
        end
    }
}

struct LineComment<'doc, 'token> {
    line: Line<'doc>,
    token: &'token str,
    comment: Option<usize>,
    indent: usize,
    empty: bool,
    single: bool,
}

fn line_comment<'a>(
    state: &EditorState,
    text: &LineEditText<'_>,
    mode: CommentMode,
    tokens_at: impl Fn(usize) -> CommentTokens<'a>,
) -> CommandResult {
    let mut lines: Vec<LineComment<'_, '_>> = Vec::new();
    for range in state.selection().ranges().iter().map(|r| r.range()) {
        let first = line_at(text.lines, range.start);
        let Some(token) = tokens_at(first.range.start)
            .line
            .filter(|token| !token.is_empty())
        else {
            continue;
        };
        let last = touched_last_line(text, &range).range.start;
        let start = lines.len();
        let mut line = first;
        loop {
            let offset = line.range.start;
            let next = line.range.end + text.break_after(line.range.end).len();
            if lines
                .last()
                .is_none_or(|prior| prior.line.range.start < offset)
            {
                let indent = leading_space(line.text).len();
                lines.push(LineComment {
                    comment: line.text[indent..].starts_with(token).then_some(indent),
                    empty: indent == line.text.len(),
                    line,
                    token,
                    indent,
                    single: false,
                });
            }
            if offset >= last {
                break;
            }
            line = line_at(text.lines, next);
        }
        let new = &mut lines[start..];
        if let Some(min) = new
            .iter()
            .filter(|line| !line.empty)
            .map(|line| line.line.text[..line.indent].chars().count())
            .min()
        {
            for line in new.iter_mut().filter(|line| !line.empty) {
                line.indent = line
                    .line
                    .text
                    .char_indices()
                    .nth(min)
                    .expect("nonempty line")
                    .0;
            }
        }
        if let [line] = new {
            line.single = true;
        }
    }
    let add = |line: &LineComment<'_, '_>| !line.empty || line.single;
    if mode != CommentMode::Uncomment
        && lines.iter().any(|line| line.comment.is_none() && add(line))
    {
        let changes = ChangeSet::new(
            lines
                .iter()
                .filter(|line| add(line))
                .map(|line| {
                    let at = line.line.range.start + line.indent;
                    Change::new(at..at, format!("{} ", line.token))
                })
                .collect(),
        )?;
        let desc = changes.desc(text.raw.len())?;
        let selection = state
            .selection()
            .ranges()
            .iter()
            .copied()
            .map(|range| {
                if range.range().is_empty() {
                    let at = desc
                        .map_pos(range.head(), Assoc::After, MapMode::Simple)
                        .ok_or(EditError::InvalidSelection)?;
                    Ok(caret(at))
                } else {
                    range.map(&desc)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Some(Transaction::new(changes).with_selection(
            EditorSelection::new(selection, state.selection().main_index())?,
        )));
    }
    if mode == CommentMode::Comment {
        return Ok(None);
    }
    transaction(
        lines
            .iter()
            .filter_map(|line| {
                let start = line.comment?;
                let tail = start + line.token.len();
                let end = tail + usize::from(line.line.text[tail..].starts_with(' '));
                Some(Change::new(
                    line.line.range.start + start..line.line.range.start + end,
                    "",
                ))
            })
            .collect(),
    )
}

fn block_comment<'a>(
    state: &EditorState,
    text: &LineEditText<'_>,
    mode: CommentMode,
    by_line: bool,
    tokens_at: impl Fn(usize) -> CommentTokens<'a>,
) -> CommandResult {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for range in state.selection().ranges().iter().map(|r| r.range()) {
        if !by_line {
            ranges.push(range);
            continue;
        }
        let first = line_at(text.lines, range.start);
        let last = touched_last_line(text, &range);
        if let Some(previous) = ranges.last_mut()
            && previous.end > first.range.start
        {
            previous.end = last.range.end;
        } else {
            ranges.push(first.range.start + leading_space(first.text).len()..last.range.end);
        }
    }
    let tokens = ranges
        .iter()
        .map(|range| {
            tokens_at(range.start)
                .block
                .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        })
        .collect::<Option<Vec<_>>>();
    let Some(tokens) = tokens else {
        return Ok(None);
    };
    let source = text.raw;
    let comments: Vec<_> = ranges
        .iter()
        .zip(&tokens)
        .map(|(range, &(open, close))| comment_parts(source, range, open, close))
        .collect();
    if mode != CommentMode::Uncomment && comments.iter().any(Option::is_none) {
        return transaction(
            ranges
                .iter()
                .zip(tokens)
                .zip(comments)
                .filter(|item| item.1.is_none())
                .flat_map(|((range, (open, close)), _)| {
                    [
                        Change::new(range.start..range.start, format!("{open} ")),
                        Change::new(range.end..range.end, format!(" {close}")),
                    ]
                })
                .collect(),
        );
    }
    if mode == CommentMode::Comment {
        return Ok(None);
    }
    let mut remove: Vec<Range<usize>> = comments.into_iter().flatten().flatten().collect();
    remove.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in remove {
        if let Some(last) = merged.last_mut()
            && last.end >= range.start
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    transaction(
        merged
            .into_iter()
            .map(|range| Change::new(range, ""))
            .collect(),
    )
}

const SEARCH_MARGIN: usize = 50;

fn edge(text: &str, direction: Direction, limit: usize) -> usize {
    let mut iter = text.char_indices();
    let mut units = 0;
    let mut edge = if matches!(direction, Forward) {
        0
    } else {
        text.len()
    };
    while let Some((start, ch)) = match direction {
        Forward => iter.next(),
        Backward => iter.next_back(),
    } {
        let crlf = ch == '\r' && text.as_bytes().get(start + 1) == Some(&b'\n');
        units += if crlf { 0 } else { ch.len_utf16() };
        if units > limit {
            break;
        }
        edge = match direction {
            Forward => start + ch.len_utf8(),
            Backward => start,
        };
    }
    edge
}

fn comment_parts(
    source: &str,
    range: &Range<usize>,
    open: &str,
    close: &str,
) -> Option<[Range<usize>; 2]> {
    let before = &source[..range.start];
    let before = &before[edge(before, Backward, SEARCH_MARGIN)..];
    let after = &source[range.end..];
    let after = &after[..edge(after, Forward, SEARCH_MARGIN)];
    let open_end = range.start - (before.len() - before.trim_end_matches(space).len());
    let close_start = range.end + leading_space(after).len();
    if before.trim_end_matches(space).ends_with(open)
        && after.trim_start_matches(space).starts_with(close)
    {
        return Some([
            open_end - open.len()..open_end + first_space(&source[open_end..range.start]),
            close_start - last_space(&source[range.end..close_start])..close_start + close.len(),
        ]);
    }
    let selected = &source[range.clone()];
    let (start, end) = if edge(selected, Forward, 2 * SEARCH_MARGIN) < selected.len() {
        (
            &selected[..edge(selected, Forward, SEARCH_MARGIN)],
            &selected[edge(selected, Backward, SEARCH_MARGIN)..],
        )
    } else {
        (selected, selected)
    };
    let start_trimmed = start.trim_start_matches(space);
    let end_trimmed = end.trim_end_matches(space);
    let after_open = start_trimmed.strip_prefix(open)?;
    let before_close = end_trimmed.strip_suffix(close)?;
    let a = range.start + start.len() - start_trimmed.len();
    let b = range.end - (end.len() - end_trimmed.len());
    if a + open.len() > b - close.len() {
        return None;
    }
    Some([
        a..a + open.len() + first_space(after_open),
        b - close.len() - last_space(before_close)..b,
    ])
}

fn first_space(text: &str) -> usize {
    if text.starts_with("\r\n") {
        2
    } else {
        text.chars()
            .next()
            .filter(|&ch| space(ch))
            .map_or(0, char::len_utf8)
    }
}

fn last_space(text: &str) -> usize {
    if text.ends_with("\r\n") {
        2
    } else {
        text.chars()
            .next_back()
            .filter(|&ch| space(ch))
            .map_or(0, char::len_utf8)
    }
}

fn transaction(changes: Vec<Change>) -> CommandResult {
    if changes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Transaction::new(ChangeSet::new(changes)?)))
    }
}
