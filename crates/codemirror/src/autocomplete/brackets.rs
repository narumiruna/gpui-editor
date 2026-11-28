//! Automatic bracket closing from `@codemirror/autocomplete` 6.20.4.
//! See `docs/licenses/codemirror-autocomplete.txt` for the upstream MIT notice.
//!
//! Positions are UTF-8 byte offsets. The host must check read-only and other
//! edit policies before dispatch. For typed input, call this only for a single
//! character replacing the main selection, outside of IME composition.

use std::ops::Range;

use crate::{
    Assoc, Change, ChangeSet, CharCategorizer, ConfigurationError, EditError, EditorState,
    Extension, RangeEdit, SelectionRange,
    commands::CommandResult,
    language_data::{CloseBracketData, NativeSyntax, attached_language_snapshot},
};

mod quotes;
mod tracking;

pub use quotes::CloseBracketSyntax;

/// Language settings at the main selection. They apply to every selection range.
#[derive(Clone, Copy)]
pub struct CloseBracketConfig<'a> {
    /// Opening tokens. Use single characters, and list a triple quote as a
    /// separate token as well as its single-quote form to enable triple quotes.
    pub brackets: &'a [&'a str],
    /// Close an asymmetric bracket before these characters, as well as before
    /// whitespace or the end of the document.
    pub before: &'a str,
    /// Recognized prefixes immediately before a string's opening quote.
    pub string_prefixes: &'a [&'a str],
    /// Additional characters counted as part of words by the quote rules.
    pub word_chars: &'a str,
    /// Syntax of the current document, when available. Without it, quote
    /// handling uses character context and tracked closes only.
    pub syntax: Option<&'a dyn CloseBracketSyntax>,
}

impl Default for CloseBracketConfig<'_> {
    fn default() -> Self {
        Self {
            brackets: &["(", "[", "{", "'", "\""],
            before: ")]}:;>",
            string_prefixes: &[],
            word_chars: "",
            syntax: None,
        }
    }
}

/// Track automatically inserted closes in editor transactions. The host calls
/// [`insert_bracket`] for text input and [`delete_bracket_pair`] for Backspace.
pub fn close_brackets() -> Extension {
    tracking::extension()
}

/// Whether the state has the automatic closing-bracket tracker installed.
pub fn close_brackets_enabled(state: &EditorState) -> Result<bool, ConfigurationError> {
    Ok(tracking::get(state)?.is_some())
}

fn close_bracket_config(state: &EditorState) -> CloseBracketConfig<'static> {
    let mut config = CloseBracketConfig::default();
    if let Some(data) = state
        .language_data_at::<CloseBracketData>(
            "closeBrackets",
            state.selection().main().head(),
            Assoc::Before,
        )
        .first()
    {
        config.brackets = data.brackets.unwrap_or(config.brackets);
        config.before = data
            .before
            .filter(|before| !before.is_empty())
            .unwrap_or(config.before);
        config.string_prefixes = data.string_prefixes;
    }
    config
}

/// Build a custom typing transaction, or return `None` for ordinary input.
/// Use the main head's close-bracket data, word rules at each caret, and
/// current attached quote syntax.
/// Install [`close_brackets`] to enable skipping automatically inserted closes.
pub fn insert_bracket(state: &EditorState, bracket: &str) -> CommandResult {
    let mut config = close_bracket_config(state);
    let attached = attached_language_snapshot(state);
    let cpp;
    let go;
    let java;
    let lezer;
    let rust;
    let wast;
    config.syntax = match attached.as_deref().map(|doc| doc.syntax()) {
        Some(NativeSyntax::Cpp(syntax)) => {
            cpp = syntax.quote_syntax_in(0);
            Some(&cpp)
        }
        Some(NativeSyntax::Go(syntax)) => {
            go = syntax.quote_syntax_in(0);
            Some(&go)
        }
        Some(NativeSyntax::Java(syntax)) => {
            java = syntax.quote_syntax_in(0);
            Some(&java)
        }
        Some(NativeSyntax::Json(syntax)) => Some(syntax),
        Some(NativeSyntax::LezerGrammar(syntax)) => {
            lezer = syntax.quote_syntax_in(0);
            Some(&lezer)
        }
        Some(NativeSyntax::Rust(syntax)) => {
            rust = syntax.quote_syntax_in(0);
            Some(&rust)
        }
        Some(NativeSyntax::Wast(syntax)) => {
            wast = syntax.quote_syntax_in(0);
            Some(&wast)
        }
        _ => None,
    };
    insert_bracket_configured(state, bracket, config, |pos| {
        Some(state.char_categorizer(pos))
    })
}

/// Like [`insert_bracket`], using only the settings and syntax supplied by the host.
pub fn insert_bracket_with(
    state: &EditorState,
    bracket: &str,
    config: CloseBracketConfig<'_>,
) -> CommandResult {
    insert_bracket_configured(state, bracket, config, |_| None)
}

fn insert_bracket_configured(
    state: &EditorState,
    bracket: &str,
    config: CloseBracketConfig<'_>,
    words_at: impl Fn(usize) -> Option<CharCategorizer>,
) -> CommandResult {
    let source = state.text().as_str();
    let tracked = tracking::get(state)?;
    let tracked_at = |pos| tracked.as_deref().is_some_and(|set| set.at(pos));
    for &open in config.brackets {
        let Some(close) = open.chars().next().and_then(closing) else {
            continue;
        };
        let mut encoded = [0; 4];
        let close: &str = close.encode_utf8(&mut encoded);
        if bracket == open {
            return edit_all(state, "input.type", |selection| {
                let range = selection.range();
                if !range.is_empty() {
                    return Some(surround(selection, open, close));
                }
                let pos = range.start;
                if open == close {
                    return quotes::insert(source, pos, open, tracked_at(pos), config, &words_at);
                }
                let allowed = next_char(source, pos)
                    .is_none_or(|next| space(next) || config.before.contains(next));
                allowed.then(|| paired(pos, open, close))
            });
        }
        if bracket == close && tracked_at(state.selection().main().range().start) {
            // The main range must be tracked. As in CodeMirror, secondary
            // carets only need to be in front of the same closing character.
            return edit_all(state, "input.type", |selection| {
                let range = selection.range();
                (range.is_empty() && source[range.start..].starts_with(close))
                    .then(|| skip(range.start, close))
            });
        }
    }
    Ok(None)
}

/// Delete a matching pair at every caret, or return `None` for ordinary
/// Backspace. Use the main head's language data. The pair does not have to
/// be automatically inserted.
pub fn delete_bracket_pair(state: &EditorState) -> CommandResult {
    delete_bracket_pair_with(state, close_bracket_config(state))
}

/// Like [`delete_bracket_pair`], with settings selected by the host.
pub fn delete_bracket_pair_with(
    state: &EditorState,
    config: CloseBracketConfig<'_>,
) -> CommandResult {
    let source = state.text().as_str();
    edit_all(state, "delete.backward", |selection| {
        let range = selection.range();
        let pos = range.start;
        let open = prev_char(source, pos).filter(|_| range.is_empty())?;
        let close = next_char(source, pos).filter(|next| Some(*next) == closing(open))?;
        if !config
            .brackets
            .contains(&&source[pos - open.len_utf8()..pos])
        {
            return None;
        }
        let start = pos - open.len_utf8();
        Some(Plan {
            changes: vec![Change::new(start..pos + close.len_utf8(), "")],
            selection: SelectionRange::new(start, start),
            track: None,
        })
    })
}

fn closing(open: char) -> Option<char> {
    Some(match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        '«' => '»',
        '»' => '«',
        '［' => '］',
        '｛' => '｝',
        ascii if ascii.is_ascii() => ascii,
        other => char::from_u32(u32::from(other) + 1)?,
    })
}

fn prev_char(source: &str, pos: usize) -> Option<char> {
    source[..pos].chars().next_back()
}

fn next_char(source: &str, pos: usize) -> Option<char> {
    source[pos..].chars().next()
}

fn space(ch: char) -> bool {
    // ECMAScript whitespace differs from Unicode White_Space at these two characters.
    ch == '\u{feff}' || ch != '\u{0085}' && ch.is_whitespace()
}

struct Plan {
    changes: Vec<Change>,
    selection: SelectionRange,
    track: Option<usize>,
}

impl Plan {
    fn into_edit(self) -> Result<RangeEdit, EditError> {
        Ok(RangeEdit {
            changes: ChangeSet::new(self.changes)?,
            selection: self.selection,
            effects: self.track.into_iter().map(tracking::mark).collect(),
        })
    }
}

fn surround(selection: SelectionRange, open: &str, close: &str) -> Plan {
    let Range { start, end } = selection.range();
    Plan {
        changes: vec![
            Change::new(start..start, open),
            Change::new(end..end, close),
        ],
        selection: SelectionRange::new(
            selection.anchor() + open.len(),
            selection.head() + open.len(),
        ),
        track: Some(end + open.len()),
    }
}

fn paired(pos: usize, open: &str, close: &str) -> Plan {
    let head = pos + open.len();
    Plan {
        changes: vec![Change::new(pos..pos, [open, close].concat())],
        selection: SelectionRange::new(head, head),
        track: Some(head),
    }
}

fn skip(pos: usize, close: &str) -> Plan {
    let end = pos + close.len();
    Plan {
        // Keep CodeMirror's same-text replacement in the proposal. The native
        // editor removes it when applying the transaction to text and history.
        changes: vec![Change::new(pos..end, close)],
        selection: SelectionRange::new(end, end),
        track: None,
    }
}

fn edit_all(
    state: &EditorState,
    event: &str,
    mut plan: impl FnMut(SelectionRange) -> Option<Plan>,
) -> CommandResult {
    let Some(plans) = state
        .selection()
        .ranges()
        .iter()
        .copied()
        .map(&mut plan)
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    let mut plans = plans.into_iter();
    let transaction = state.change_by_range(|_| {
        plans
            .next()
            .expect("one plan per selection range")
            .into_edit()
    })?;
    Ok(Some(
        transaction.with_scroll_into_view().with_user_event(event),
    ))
}
