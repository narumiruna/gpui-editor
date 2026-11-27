use std::ops::{Range, RangeInclusive};

use crate::{
    EditorState, RangeSet, StateEffect, Text, Transaction,
    view::{ANNOUNCE, keymap::KeyBinding},
};

use super::{Fold, code_folding, field, fold_effect, foldable, folded_ranges, unfold_effect};

type Provider<'a> = dyn FnMut(&EditorState, usize, usize) -> Option<Range<usize>> + 'a;

/// Command identities in the stock folding keymap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldCommand {
    FoldCode,
    UnfoldCode,
    FoldAll,
    UnfoldAll,
}

impl FoldCommand {
    pub fn run(
        self,
        state: &EditorState,
        provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
    ) -> Option<Transaction> {
        self.run_on_lines(state, state.text(), provider)
    }

    pub(crate) fn run_on_lines(
        self,
        state: &EditorState,
        lines: &Text,
        provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
    ) -> Option<Transaction> {
        match self {
            Self::FoldCode => fold_code_on_lines(state, lines, provider),
            Self::UnfoldCode => unfold_code_on_lines(state, lines),
            Self::FoldAll => fold_all_on_lines(state, lines, provider),
            Self::UnfoldAll => unfold_all(state),
        }
    }
}

/// The pinned fold and unfold bindings, in extension order.
pub fn fold_keymap() -> [KeyBinding<FoldCommand>; 4] {
    use FoldCommand::*;
    [
        KeyBinding {
            mac: Some("Cmd-Alt-[".into()),
            ..KeyBinding::new("Ctrl-Shift-[", FoldCode)
        },
        KeyBinding {
            mac: Some("Cmd-Alt-]".into()),
            ..KeyBinding::new("Ctrl-Shift-]", UnfoldCode)
        },
        KeyBinding::new("Ctrl-Alt-[", FoldAll),
        KeyBinding::new("Ctrl-Alt-]", UnfoldAll),
    ]
}

/// Fold the first selected line with a valid fold range.
///
/// The provider runs after configured fold services. All receive the state
/// and the byte bounds of a line block. An
/// existing fold joins the logical lines at its endpoints into one block.
/// The selection is not changed. A successful command enables folding if needed.
pub fn fold_code(
    state: &EditorState,
    provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<Transaction> {
    fold_code_on_lines(state, state.text(), provider)
}

fn fold_code_on_lines(
    state: &EditorState,
    text: &Text,
    mut provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<Transaction> {
    let folded = folded_ranges(state);
    let range = Lines::new(text, &folded)
        .selected(state)
        .find_map(|line| candidate(state, &line, &mut provider))?;
    transaction(state, fold_effects(state, text, range, true).into())
}

/// Unfold the earliest fold touching each selected line block. When starts
/// tie, use the range with the earlier end, in native range order.
pub fn unfold_code(state: &EditorState) -> Option<Transaction> {
    unfold_code_on_lines(state, state.text())
}

fn unfold_code_on_lines(state: &EditorState, text: &Text) -> Option<Transaction> {
    let folded = folded_ranges(state);
    if folded.is_empty() {
        return None;
    }
    let effects = Lines::new(text, &folded)
        .selected(state)
        .filter_map(|line| find_fold(&folded, &line))
        .flat_map(|range| fold_effects(state, text, range, false))
        .collect();
    transaction(state, effects)
}

/// Fold top-level ranges. After a match, continue after the line block that
/// contains the range's end. Do not query an empty final line at EOF.
pub fn fold_all(
    state: &EditorState,
    provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<Transaction> {
    fold_all_on_lines(state, state.text(), provider)
}

fn fold_all_on_lines(
    state: &EditorState,
    text: &Text,
    mut provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<Transaction> {
    let folded = folded_ranges(state);
    let lines = Lines::new(text, &folded);
    let mut number = 1;
    let mut effects = Vec::new();
    while number <= text.line_count() {
        let line = lines.block(number);
        if line.bytes.start == text.len() {
            break;
        }
        number = line.last + 1;
        if let Some(range) = candidate(state, &line, &mut provider) {
            number = number.max(lines.block(lines.at(range.end)).last + 1);
            effects.push(fold_effect().of(range));
        }
    }
    transaction(state, effects)
}

/// Unfold all stored ranges, including nested ranges.
pub fn unfold_all(state: &EditorState) -> Option<Transaction> {
    let effects = folded_ranges(state)
        .iter(0)
        .map(|fold| unfold_effect().of(fold.from..fold.to))
        .collect();
    transaction(state, effects)
}

/// For each selected line block, unfold the earliest touching fold or search
/// backwards for a provider range that extends past the selected block's start.
pub fn toggle_fold(
    state: &EditorState,
    mut provider: impl FnMut(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<Transaction> {
    let folded = folded_ranges(state);
    let lines = Lines::new(state.text(), &folded);
    let effects = lines
        .selected(state)
        .filter_map(|mut line| {
            if let Some(range) = find_fold(&folded, &line) {
                return Some(fold_effects(state, lines.text, range, false));
            }
            let start = line.bytes.start;
            loop {
                if let Some(range) = candidate(state, &line, &mut provider)
                    && range.end > start
                {
                    return Some(fold_effects(state, lines.text, range, true));
                }
                if line.first == 1 {
                    return None;
                }
                line = lines.block(line.first - 1);
            }
        })
        .flatten()
        .collect();
    transaction(state, effects)
}

fn fold_effects(
    state: &EditorState,
    text: &Text,
    range: Range<usize>,
    folding: bool,
) -> [StateEffect; 2] {
    let from = text.line_at(range.start).expect("valid fold start").number;
    let to = text.line_at(range.end).expect("valid fold end").number;
    let effect = if folding {
        fold_effect()
    } else {
        unfold_effect()
    };
    [effect.of(range), announce_fold(state, from..=to, folding)]
}

/// Announce an explicit fold or unfold using the host's displayed line numbers.
pub fn announce_fold(
    state: &EditorState,
    lines: RangeInclusive<usize>,
    folding: bool,
) -> StateEffect {
    let phrase = state.phrase(
        if folding {
            "Folded lines"
        } else {
            "Unfolded lines"
        },
        &[],
    );
    let to_phrase = state.phrase("to", &[]);
    ANNOUNCE.of(format!(
        "{phrase} {} {to_phrase} {}.",
        lines.start(),
        lines.end()
    ))
}

pub(super) fn transaction(
    state: &EditorState,
    mut effects: Vec<StateEffect>,
) -> Option<Transaction> {
    if effects.is_empty() {
        return None;
    }
    if field(state).is_none() {
        effects.push(code_folding().append());
    }
    Some(Transaction::default().with_effects(effects))
}

fn find_fold(folded: &RangeSet<Fold>, line: &LineBlock) -> Option<Range<usize>> {
    folded
        .iter(line.bytes.start)
        .next()
        .filter(|fold| fold.from <= line.bytes.end)
        .map(|fold| fold.from..fold.to)
}

fn candidate(
    state: &EditorState,
    line: &LineBlock,
    provider: &mut Provider<'_>,
) -> Option<Range<usize>> {
    foldable(state, line.bytes.start, line.bytes.end, provider)
}

struct LineBlock {
    first: usize,
    last: usize,
    bytes: Range<usize>,
}

// Project just the line joins caused by folds. Wrapping and unrelated view
// decorations do not affect the bounds passed to the provider.
struct Lines<'a> {
    text: &'a Text,
    joined: Vec<RangeInclusive<usize>>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a Text, folded: &RangeSet<Fold>) -> Self {
        let mut lines = Self {
            text,
            joined: Vec::new(),
        };
        for fold in folded.iter(0) {
            let first = lines.at(fold.from);
            let last = lines.at(fold.to);
            if first == last {
                continue;
            }
            if let Some(previous) = lines.joined.last_mut()
                && *previous.end() >= first
            {
                *previous = *previous.start()..=last.max(*previous.end());
            } else {
                lines.joined.push(first..=last);
            }
        }
        lines
    }

    fn at(&self, position: usize) -> usize {
        self.text
            .line_at(position)
            .expect("valid byte offset")
            .number
    }

    fn block(&self, number: usize) -> LineBlock {
        let next = self.joined.partition_point(|group| *group.end() < number);
        let (first, last) = self
            .joined
            .get(next)
            .filter(|group| group.contains(&number))
            .map_or((number, number), |group| (*group.start(), *group.end()));
        let start = self
            .text
            .line(first)
            .expect("valid line number")
            .range
            .start;
        let end = self.text.line(last).expect("valid line number").range.end;
        LineBlock {
            first,
            last,
            bytes: start..end,
        }
    }

    fn selected(&self, state: &EditorState) -> impl Iterator<Item = LineBlock> {
        let mut previous = 0;
        state.selection().ranges().iter().filter_map(move |range| {
            let number = self.at(range.head());
            if number <= previous {
                return None;
            }
            let line = self.block(number);
            previous = line.last;
            Some(line)
        })
    }
}
