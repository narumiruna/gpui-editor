use std::sync::LazyLock;

use regex::Regex;

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorSelection, EditorState, Line, RangeEdit, Text,
    Transaction, view::keymap::KeyBinding,
};

use super::{
    super::indent_service::{IndentStyle, style},
    MarkdownDocument, MarkdownNode as Node,
    MarkdownNodeKind::*,
};

/// A Markdown command's edit and its cursor position in the changed document.
#[derive(Debug, Eq, PartialEq)]
pub struct MarkdownEdit {
    pub changes: ChangeSet,
    pub cursor: usize,
}

/// The commands in the stock Markdown markup keymap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkdownMarkupCommand {
    Continue,
    DeleteBackward,
}

impl MarkdownMarkupCommand {
    pub fn run(
        self,
        document: &MarkdownDocument,
        state: &EditorState,
    ) -> Result<Option<Transaction>, EditError> {
        match self {
            Self::Continue => document.continue_markup_transaction(state, true),
            Self::DeleteBackward => document.delete_markup_backward_transaction(state),
        }
    }
}

/// Bind Enter and Backspace to the pinned Markdown markup commands.
pub fn markdown_keymap() -> [KeyBinding<MarkdownMarkupCommand>; 2] {
    [
        KeyBinding::new("Enter", MarkdownMarkupCommand::Continue),
        KeyBinding::new("Backspace", MarkdownMarkupCommand::DeleteBackward),
    ]
}

impl MarkdownDocument {
    /// Continue list or quote markup at all cursors in one transaction.
    /// If any range is not a caret, has no markup edit, or belongs to a
    /// different text snapshot, return `None` without a partial plan.
    pub fn continue_markup_transaction(
        &self,
        state: &EditorState,
        non_tight_lists: bool,
    ) -> Result<Option<Transaction>, EditError> {
        let tabs = uses_tab_indent(state);
        self.markup_transaction(
            state,
            |head| self.continue_markup_with_indent(head, non_tight_lists, tabs),
            "input",
        )
    }

    /// Delete markup at all cursors in one transaction. Use the same
    /// snapshot and all-or-nothing rules as [`Self::continue_markup_transaction`].
    pub fn delete_markup_backward_transaction(
        &self,
        state: &EditorState,
    ) -> Result<Option<Transaction>, EditError> {
        let tabs = uses_tab_indent(state);
        self.markup_transaction(
            state,
            |head| self.delete_markup_with_indent(head, tabs),
            "delete",
        )
    }

    fn markup_transaction(
        &self,
        state: &EditorState,
        plan: impl Fn(usize) -> Option<MarkdownEdit>,
        event: &str,
    ) -> Result<Option<Transaction>, EditError> {
        if state.text() != &self.text {
            return Ok(None);
        }
        let Some(edits) = state
            .selection()
            .ranges()
            .iter()
            .map(|range| {
                if range.is_empty() {
                    plan(range.head())
                } else {
                    None
                }
            })
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(None);
        };
        let mut edits = edits.into_iter();
        let transaction = state.change_by_range(|_| {
            let edit = edits.next().expect("one edit for each selection");
            Ok(RangeEdit {
                changes: edit.changes,
                selection: EditorSelection::cursor(edit.cursor, None, None, None),
                effects: Vec::new(),
            })
        })?;
        Ok(Some(
            transaction.with_user_event(event).with_scroll_into_view(),
        ))
    }

    /// Continue the list item or quote at one cursor, or remove an empty level.
    /// When `non_tight_lists` is true, allow an empty second item to make a list loose.
    pub fn continue_markup(&self, position: usize, non_tight_lists: bool) -> Option<MarkdownEdit> {
        self.continue_markup_with_indent(position, non_tight_lists, false)
    }

    fn continue_markup_with_indent(
        &self,
        position: usize,
        non_tight_lists: bool,
        tabs: bool,
    ) -> Option<MarkdownEdit> {
        let lines = self.syntax_lines();
        let line = lines.line_at(position)?;
        let offset = position.checked_sub(line.range.start)?;
        if offset > line.text.len() {
            return None;
        }
        let path = self.path(position, true).collect::<Vec<_>>();
        if in_html(&path) && in_html(&self.path(position, false).collect::<Vec<_>>()) {
            return None;
        }
        let mut context = context(&path, lines);
        while context.last().is_some_and(|inner| inner.from > offset) {
            context.pop();
        }
        let inner = context.last()?;
        let marker_end = inner.to - inner.after.len();
        if marker_end > offset {
            return None;
        }
        let empty = whitespace(line.text.get(inner.to..).unwrap_or(""));
        let previous = line.number.checked_sub(1).and_then(|n| lines.line(n));
        let br = lines
            .line(line.number + 1)
            .and_then(|next| self.text.slice(line.range.end..next.range.start))
            .or_else(|| {
                previous
                    .as_ref()
                    .and_then(|prev| self.text.slice(prev.range.end..line.range.start))
            })
            .unwrap_or("\n");
        let mut changes = Vec::new();
        if let Some(item) = inner.item.filter(|_| empty) {
            if item.range.start < line.range.start
                && !blank_quote(line.text.get(..inner.to).unwrap_or(line.text))
            {
                return None;
            }
            let mut items = inner.node.children().filter(|node| node.kind == ListItem);
            let first = items.next()?;
            let second = items.next();
            if first.range.end >= position
                || second.is_some_and(|second| second.range.end < position)
                || previous.as_ref().is_some_and(|prev| blank_quote(prev.text))
                || !non_tight_lists
            {
                let outer = context.iter().rev().nth(1);
                let (start, insert) = outer.map_or_else(
                    || (0, String::new()),
                    |outer| {
                        if outer.item.is_some() {
                            (outer.from, outer.marker())
                        } else {
                            (outer.to, String::new())
                        }
                    },
                );
                let start = line.range.start + start.min(offset);
                let cursor = start + insert.len();
                changes.push(Change::new(start..position, insert));
                renumber(inner, &self.text, -1, &mut changes);
                if let Some(outer) = outer {
                    renumber(outer, &self.text, 1, &mut changes);
                }
                return edit(changes, cursor);
            }
            let insert = blank_line(&context, &line, tabs) + br;
            let cursor = position + insert.len();
            return edit(
                vec![Change::new(line.range.start..line.range.start, insert)],
                cursor,
            );
        }
        if inner.item.is_none()
            && empty
            && let Some(prev) = previous
            && let Some(at) = prev
                .text
                .trim_end_matches(js_space)
                .strip_suffix('>')
                .map(str::len)
            && at == inner.from
        {
            changes.push(Change::new(prev.range.start + at..prev.range.end, ""));
            changes.push(Change::new(
                line.range.start + inner.from..line.range.end,
                "",
            ));
            let changes = ChangeSet::new(changes).ok()?;
            return Some(MarkdownEdit {
                cursor: changes.map_pos(position, Assoc::Before),
                changes,
            });
        }
        renumber(inner, &self.text, 1, &mut changes);
        let continued = inner
            .item
            .is_some_and(|item| item.range.start < line.range.start);
        let mut insert = String::new();
        if !continued
            || line
                .text
                .bytes()
                .take_while(|&b| {
                    b.is_ascii_whitespace()
                        || b.is_ascii_digit()
                        || matches!(b, b'.' | b')' | b'-' | b'+' | b'*' | b'>')
                })
                .count()
                >= inner.to
        {
            for (i, ctx) in context.iter().enumerate() {
                if let Some(next) = context.get(i + 1) {
                    insert.push_str(&ctx.blank(
                        Some(columns(line.text, next.from).saturating_sub(insert.len())),
                        true,
                    ));
                } else if continued {
                    insert.push_str(&ctx.blank(None, true));
                } else {
                    insert.push_str(&ctx.marker());
                }
            }
        }
        normalize_indent(&mut insert, tabs);
        if loose(inner.node, lines) {
            insert = blank_line(&context, &line, tabs) + br + &insert;
        }
        let start = line.range.start + line.text[..offset].trim_end_matches(js_space).len();
        let insert = br.to_owned() + &insert;
        let cursor = start + insert.len();
        changes.push(Change::new(start..position, insert));
        edit(changes, cursor)
    }

    /// Delete one level of quote or list markup, when the cursor follows it.
    pub fn delete_markup_backward(&self, position: usize) -> Option<MarkdownEdit> {
        self.delete_markup_with_indent(position, false)
    }

    fn delete_markup_with_indent(&self, position: usize, tabs: bool) -> Option<MarkdownEdit> {
        let lines = self.syntax_lines();
        let line = lines.line_at(position)?;
        let offset = position.checked_sub(line.range.start)?;
        if offset > line.text.len() {
            return None;
        }
        let mut path = self.path(position, true).collect::<Vec<_>>();
        if in_html(&path) {
            return None;
        }
        let mut scan = position;
        if path.last().is_some_and(|(node, _)| is_mark(node)) {
            scan = path.pop()?.0.range.start;
        }
        while let Some((node, _)) = path.last().copied() {
            let Some(prev) = node.children().rev().find(|child| child.range.start < scan) else {
                break;
            };
            if is_mark(prev) {
                scan = prev.range.start;
            } else if matches!(prev.kind, OrderedList | BulletList) {
                let last = prev.children.last()?;
                path.push((prev, &[]));
                path.push((last, &[]));
                scan = last.range.end;
            } else {
                break;
            }
        }
        let context = context(&path, lines);
        let inner = context.last()?;
        let space_end = inner.to - inner.after.len() + usize::from(!inner.after.is_empty());
        if offset > space_end && line.text.get(space_end..offset).is_some_and(whitespace) {
            let start = line.range.start + space_end;
            return edit(vec![Change::new(start..position, "")], start);
        }
        let before = line.text.get(..inner.to).unwrap_or(line.text);
        if offset != space_end
            || !(inner
                .item
                .is_some_and(|item| line.range.start <= item.range.start)
                || blank_quote(before))
        {
            return None;
        }
        let start = line.range.start + inner.from;
        if inner
            .item
            .is_some_and(|item| inner.node.range.start < item.range.start)
            && line
                .text
                .get(inner.from..inner.to)
                .is_some_and(|part| !whitespace(part))
        {
            let mut insert = inner.blank(
                Some(columns(line.text, inner.to) - columns(line.text, inner.from)),
                true,
            );
            if start == line.range.start {
                normalize_indent(&mut insert, tabs);
            }
            let cursor = start + insert.len();
            return edit(
                vec![Change::new(start..line.range.start + inner.to, insert)],
                cursor,
            );
        }
        (start < position)
            .then(|| edit(vec![Change::new(start..position, "")], start))
            .flatten()
    }
}

fn in_html(path: &[(&Node, &[Node])]) -> bool {
    path.iter()
        .any(|(node, _)| matches!(node.kind, HTMLBlock | HTMLTag | CommentBlock))
}

fn is_mark(node: &Node) -> bool {
    matches!(node.kind, QuoteMark | ListMark)
}

struct Context<'a> {
    node: &'a Node,
    item: Option<&'a Node>,
    from: usize,
    to: usize,
    before: &'a str,
    after: &'a str,
    marker: String,
    number: Option<u32>,
}

impl Context<'_> {
    fn blank(&self, width: Option<usize>, trailing: bool) -> String {
        let mut s = self.before.to_owned();
        if self.item.is_none() {
            s.push('>');
        }
        let n = width
            .unwrap_or(self.to - self.from - self.after.len())
            .saturating_sub(s.len());
        s.extend(std::iter::repeat_n(' ', n));
        if width.is_none() && trailing {
            s.push_str(self.after);
        }
        s
    }

    fn marker(&self) -> String {
        let mut s = self.before.to_owned();
        if let Some(n) = self.number {
            s.push_str(&(n + 1).to_string());
        }
        s + &self.marker + self.after
    }
}

fn context<'a>(path: &[(&'a Node, &'a [Node])], text: &'a Text) -> Vec<Context<'a>> {
    static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^>( ?)").unwrap());
    static ORDERED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^( *)([0-9]+)([.)])( *)").unwrap());
    static BULLET: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^( *)([-+*])( {1,4}\[[ xX]\])?( +)").unwrap());
    let mut result = Vec::new();
    for (i, &(node, _)) in path.iter().enumerate() {
        if node.kind == FencedCode {
            return Vec::new();
        }
        if !matches!(node.kind, Blockquote | ListItem) {
            continue;
        }
        let Some(line) = text.line_at(node.range.start) else {
            continue;
        };
        let from = node.range.start - line.range.start;
        let rest = &line.text[from..];
        if node.kind == Blockquote {
            if let Some(m) = QUOTE.captures(rest) {
                result.push(Context {
                    node,
                    item: None,
                    from,
                    to: from + m[0].len(),
                    before: "",
                    after: m.get(1).unwrap().as_str(),
                    marker: ">".into(),
                    number: None,
                });
            }
        } else if let Some(&(list, _)) = path.get(i.wrapping_sub(1)) {
            let ordered = list.kind == OrderedList;
            if let Some(m) = if ordered {
                ORDERED.captures(rest)
            } else {
                BULLET.captures(rest)
            } {
                let mut after = m.get(4).unwrap().as_str();
                let mut to = from + m[0].len();
                if (ordered && after.len() >= 4) || (!ordered && after.len() > 4) {
                    after = &after[..after.len() - 4];
                    to -= 4;
                }
                let (marker, number) = if ordered {
                    (m[3].into(), m[2].parse().ok())
                } else {
                    let task = m
                        .get(3)
                        .map_or_else(String::new, |task| task.as_str().replace(['x', 'X'], " "));
                    (m[2].to_owned() + &task, None)
                };
                result.push(Context {
                    node: list,
                    item: Some(node),
                    from,
                    to,
                    before: m.get(1).unwrap().as_str(),
                    after,
                    marker,
                    number,
                });
            }
        }
    }
    result
}

fn renumber(context: &Context<'_>, text: &Text, delta: i32, changes: &mut Vec<Change>) {
    let (Some(item), Some(mut previous)) = (context.item, context.number) else {
        return;
    };
    for item in context
        .node
        .children()
        .filter(|n| n.kind == ListItem && n.range.start > item.range.start)
    {
        let Some(mark) = item.children().find(|node| node.kind == ListMark) else {
            break;
        };
        let Some(number) = text
            .slice(mark.range.start..mark.range.end - 1)
            .and_then(|s| s.parse::<u32>().ok())
        else {
            break;
        };
        if number != previous + 1 {
            break;
        }
        changes.push(Change::new(
            mark.range.start..mark.range.end - 1,
            (i64::from(number) + i64::from(delta)).to_string(),
        ));
        previous = number;
    }
}

fn loose(node: &Node, text: &Text) -> bool {
    let mut items = node.children().filter(|n| n.kind == ListItem);
    let Some((first, second)) = items.next().zip(items.next()) else {
        return false;
    };
    let Some((line1, line2)) = text
        .line_at(first.range.end)
        .zip(text.line_at(second.range.start))
    else {
        return false;
    };
    line1.number + usize::from(!blank_quote(line1.text)) < line2.number
}

fn blank_line(context: &[Context<'_>], line: &Line<'_>, tabs: bool) -> String {
    let mut s = String::new();
    let count = context.len().saturating_sub(1);
    for (i, ctx) in context.iter().take(count).enumerate() {
        let width = (i + 1 < count)
            .then(|| columns(line.text, context[i + 1].from).saturating_sub(s.len()));
        s.push_str(&ctx.blank(width, width.is_some()));
    }
    normalize_indent(&mut s, tabs);
    s
}

fn uses_tab_indent(state: &EditorState) -> bool {
    style(state) == IndentStyle::parse("\t").expect("single tab indent style")
}

fn normalize_indent(content: &mut String, tabs: bool) {
    if tabs {
        let first = content
            .bytes()
            .take_while(|&b| matches!(b, b' ' | b'\t'))
            .count();
        let width = columns(content, first);
        if width != 0 {
            let indent = "\t".repeat(width / 4) + &" ".repeat(width % 4);
            content.replace_range(..first, &indent);
        }
    }
}

fn columns(line: &str, end: usize) -> usize {
    line.bytes().take(end).fold(0, |col, b| {
        if b == b'\t' {
            col + 4 - col % 4
        } else {
            col + 1
        }
    })
}

fn whitespace(text: &str) -> bool {
    text.chars().all(js_space)
}
fn blank_quote(text: &str) -> bool {
    text.chars().all(|ch| ch == '>' || js_space(ch))
}
fn js_space(ch: char) -> bool {
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}'
}

fn edit(changes: Vec<Change>, cursor: usize) -> Option<MarkdownEdit> {
    Some(MarkdownEdit {
        changes: ChangeSet::new(changes).ok()?,
        cursor,
    })
}
