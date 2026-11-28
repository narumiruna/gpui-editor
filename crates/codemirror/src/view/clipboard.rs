//! Plain-text clipboard planning from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt`.

use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, Change, ChangeSet, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Facet, Line, RangeEdit, Text, Transaction,
    language::line_projection::{is_editor_line_view, keeps_separate_breaks},
};

type Filter = Arc<dyn Fn(String, &EditorState) -> String + Send + Sync>;
type Filters = Facet<Filter, Vec<Filter>>;

static CLIPBOARD_INPUT_FILTER: LazyLock<Filters> = LazyLock::new(Facet::collect);
static CLIPBOARD_OUTPUT_FILTER: LazyLock<Filters> = LazyLock::new(Facet::collect);

/// Transform plain text before parsing it for paste. Filters run in extension
/// precedence order. Each filter receives the previous result and the same state.
pub fn clipboard_input_filter(
    filter: impl Fn(String, &EditorState) -> String + Send + Sync + 'static,
) -> Extension {
    CLIPBOARD_INPUT_FILTER.of(Arc::new(filter))
}

/// Transform plain text after the copy planner joins its parts. This uses the
/// same order as [`clipboard_input_filter`] and does not change the cut ranges.
pub fn clipboard_output_filter(
    filter: impl Fn(String, &EditorState) -> String + Send + Sync + 'static,
) -> Extension {
    CLIPBOARD_OUTPUT_FILTER.of(Arc::new(filter))
}

/// Run the input filters on host-supplied text without parsing it or planning an edit.
/// The caller decides whether to accept paste before calling this function.
pub fn filter_clipboard_input(
    state: &EditorState,
    text: String,
) -> Result<String, ConfigurationError> {
    filter_text(state, &CLIPBOARD_INPUT_FILTER, text)
}

/// Run the output filters on text that the caller has already collected.
/// This does not inspect selections or change the document.
pub fn filter_clipboard_output(
    state: &EditorState,
    text: String,
) -> Result<String, ConfigurationError> {
    filter_text(state, &CLIPBOARD_OUTPUT_FILTER, text)
}

fn filter_text(
    state: &EditorState,
    filters: &Filters,
    mut text: String,
) -> Result<String, ConfigurationError> {
    for filter in state.context().facet(filters)?.iter() {
        text = filter(text, state);
    }
    Ok(text)
}

/// Filtered text to copy or cut, with the original document ranges.
#[derive(Debug)]
pub struct ClipboardCopy {
    text: String,
    ranges: Vec<Range<usize>>,
    linewise: bool,
    projected: bool,
}

impl ClipboardCopy {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Ranges in the original state, in UTF-8 bytes. A linewise range includes
    /// its following line break, if any, but never its preceding break.
    pub fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }

    pub fn is_linewise(&self) -> bool {
        self.linewise
    }

    /// Build the deletion for a cut. Use the same state as for the copy plan.
    /// Read-only states still permit copying but do not produce a deletion.
    pub fn cut_transaction(&self, state: &EditorState) -> Result<Option<Transaction>, EditError> {
        if state.read_only() {
            return Ok(None);
        }
        let changes = ChangeSet::new(
            self.ranges
                .iter()
                .cloned()
                .map(|range| Change::new(range, ""))
                .collect(),
        )?;
        changes.check(state.text())?;
        if self.projected && !keeps_separate_breaks(state.text().as_str(), &changes) {
            return Err(EditError::InvalidRange);
        }
        Ok(Some(
            Transaction::new(changes)
                .with_user_event("delete.cut")
                .with_scroll_into_view(),
        ))
    }
}

/// Host-owned memory of the last handled copy or cut. Share this across views
/// in the same clipboard scope to match CodeMirror's module-wide memory.
/// It does not read the clipboard or expire when another app copies text.
#[derive(Debug, Default)]
pub struct ClipboardLinewiseMemory(Option<String>);

impl ClipboardLinewiseMemory {
    /// Call for each handled copy or cut, including read-only cuts. A declined
    /// copy or cut leaves this memory unchanged. An ordinary copy clears it.
    pub fn record_copy(&mut self, copy: &ClipboardCopy) {
        self.0 = copy.linewise.then(|| copy.text.clone());
    }

    /// Record a successful copy or cut that did not copy whole lines.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    fn copied_text_for_cursors(&self, state: &EditorState) -> Option<&str> {
        self.0.as_deref().filter(|_| {
            state
                .selection()
                .ranges()
                .iter()
                .all(|range| range.is_empty())
        })
    }
}

/// Plan a copy or cut for the current selection. The host decides whether to
/// handle the event. A copy with any nonempty selection ignores empty ranges.
/// With only cursors, copy each distinct line once. Join parts with the state
/// separator, then run the output filters once on the whole string.
/// Return `None` when an ordinary copy has empty output after filtering.
pub fn plan_clipboard_copy(
    state: &EditorState,
) -> Result<Option<ClipboardCopy>, ConfigurationError> {
    let mut ranges = Vec::new();
    let mut parts = Vec::new();
    for selection in state.selection().ranges() {
        if !selection.is_empty() {
            let range = selection.range();
            parts.push(state.slice_doc(range.clone()).expect("valid selection"));
            ranges.push(range);
        }
    }
    if ranges.is_empty() {
        return plan_linewise_copy(state, 0..state.text().len(), &state.line_break());
    }
    let text = filter_clipboard_output(state, parts.join(&state.line_break()))?;
    Ok((!text.is_empty()).then_some(ClipboardCopy {
        text,
        ranges,
        linewise: false,
        projected: false,
    }))
}

#[derive(Clone, Copy)]
struct ClipboardLines<'a> {
    text: &'a Text,
    origin: usize,
    projected: bool,
}

impl<'a> ClipboardLines<'a> {
    fn raw(state: &'a EditorState) -> Self {
        Self {
            text: state.text(),
            origin: 0,
            projected: false,
        }
    }

    fn projected(state: &EditorState, scope: &Range<usize>, lines: &'a Text) -> Option<Self> {
        let original = state.text().slice(scope.clone())?;
        let valid = is_editor_line_view(original, lines);
        // A projection must not introduce a boundary inside an original CRLF.
        let split_crlf = |at: usize| {
            at > 0 && state.text().as_str().as_bytes().get(at - 1..=at) == Some(b"\r\n")
        };
        (valid && !split_crlf(scope.start) && !split_crlf(scope.end)).then_some(Self {
            text: lines,
            origin: scope.start,
            projected: true,
        })
    }

    fn line_at(self, head: usize) -> Option<Line<'a>> {
        let mut line = self.text.line_at(head.checked_sub(self.origin)?)?;
        line.range = self.origin + line.range.start..self.origin + line.range.end;
        Some(line)
    }

    fn next_line_start(self, number: usize) -> usize {
        self.origin
            + self
                .text
                .line(number + 1)
                .map_or(self.text.len(), |line| line.range.start)
    }
}

/// Copy the distinct lines under the cursors when each line fits in `scope`.
/// The cut range never consumes a line break outside `scope`. Use a scope
/// ending at a line's content boundary to keep its following separator.
pub fn plan_linewise_copy(
    state: &EditorState,
    scope: Range<usize>,
    separator: &str,
) -> Result<Option<ClipboardCopy>, ConfigurationError> {
    if !state.text().is_valid_range(scope.clone()) {
        return Ok(None);
    }
    copy_lines(state, scope, separator, ClipboardLines::raw(state))
}

/// Plan a scoped copy or cut with displayed line boundaries. `lines` must
/// match `scope` byte for byte except that each lone CR is LF. Return `None`
/// for a different projection, a split CRLF, or a cursor outside the scope.
/// Output filters still receive the original editor state.
pub fn plan_linewise_copy_projected(
    state: &EditorState,
    scope: Range<usize>,
    separator: &str,
    lines: &Text,
) -> Result<Option<ClipboardCopy>, ConfigurationError> {
    let Some(lines) = ClipboardLines::projected(state, &scope, lines) else {
        return Ok(None);
    };
    copy_lines(state, scope, separator, lines)
}

fn copy_lines(
    state: &EditorState,
    scope: Range<usize>,
    separator: &str,
    lines: ClipboardLines<'_>,
) -> Result<Option<ClipboardCopy>, ConfigurationError> {
    let mut ranges = Vec::new();
    let mut parts = Vec::new();
    let mut last_line = 0;
    for selection in state.selection().ranges() {
        let head = selection.head();
        if !selection.is_empty() || !(scope.start..=scope.end).contains(&head) {
            return Ok(None);
        }
        let Some(line) = lines.line_at(head) else {
            return Ok(None);
        };
        if line.range.start < scope.start || line.range.end > scope.end {
            return Ok(None);
        }
        if line.number != last_line {
            parts.push(line.text);
            let end = lines.next_line_start(line.number);
            ranges.push(line.range.start..end.min(scope.end));
            last_line = line.number;
        }
    }
    let text = filter_clipboard_output(state, parts.join(separator))?;
    Ok(Some(ClipboardCopy {
        text,
        ranges,
        linewise: true,
        projected: lines.projected,
    }))
}

/// Plan a paste of host-supplied plain text. Read-only states return `None`
/// before running input filters. Installed stock Markdown can handle a URL
/// before the clipboard filters. Dispatch the transaction against this state
/// to run its normal transaction filters and history rules.
///
/// Distribute lines when their count matches the selection count. For a
/// linewise paste, all ranges must be cursors and the parsed text joined with
/// LF must match the exact last linewise copy. Insert above each distinct
/// target line. Extra cursors on that line do not consume clipboard lines.
/// Parsing follows native [`EditorState::to_text`] line rules.
pub fn plan_clipboard_paste(
    state: &EditorState,
    input: &str,
    memory: &ClipboardLinewiseMemory,
) -> Result<Option<Transaction>, EditError> {
    if state.read_only() {
        return Ok(None);
    }
    if let Some(paste) = crate::language_data::plan_catalog_markdown_paste(state, input) {
        return Ok(Some(paste));
    }
    let input = filter_clipboard_input(state, input.to_owned())?;
    let text = state.to_text(&input);
    let matches_copy = memory
        .copied_text_for_cursors(state)
        .is_some_and(|copied| text.iter_lines().eq(copied.split('\n')));
    let linewise = matches_copy.then(|| {
        let line_break = state.line_break();
        move |part: &str| {
            state
                .to_text(&format!("{part}{line_break}"))
                .as_str()
                .to_owned()
        }
    });
    plan_paste(state, &input, &text, linewise, ClipboardLines::raw(state)).map(Some)
}

/// Plan a paste above each distinct cursor line when the already-filtered
/// input matches the last handled linewise copy. Parse both strings with the
/// current state. The host supplies the line break to append and validates
/// the proposed edit against its scope.
pub fn plan_linewise_paste(
    state: &EditorState,
    input: &str,
    memory: &ClipboardLinewiseMemory,
    line_break: &str,
) -> Result<Option<Transaction>, EditError> {
    paste_lines(state, input, memory, line_break, ClipboardLines::raw(state))
}

/// Plan a matching linewise paste using the scoped line projection accepted
/// by [`plan_linewise_copy_projected`]. Reject selections outside the scope.
/// `line_break` supplies the separator for new lines; original bytes remain.
pub fn plan_linewise_paste_projected(
    state: &EditorState,
    input: &str,
    memory: &ClipboardLinewiseMemory,
    line_break: &str,
    scope: Range<usize>,
    lines: &Text,
) -> Result<Option<Transaction>, EditError> {
    let Some(lines) = ClipboardLines::projected(state, &scope, lines) else {
        return Ok(None);
    };
    if state
        .selection()
        .ranges()
        .iter()
        .any(|range| !(scope.start..=scope.end).contains(&range.head()))
    {
        return Ok(None);
    }
    let paste = paste_lines(state, input, memory, line_break, lines)?;
    if paste.as_ref().is_some_and(|transaction| {
        !keeps_separate_breaks(state.text().as_str(), transaction.changes())
    }) {
        return Err(EditError::InvalidRange);
    }
    Ok(paste)
}

fn paste_lines(
    state: &EditorState,
    input: &str,
    memory: &ClipboardLinewiseMemory,
    line_break: &str,
    lines: ClipboardLines<'_>,
) -> Result<Option<Transaction>, EditError> {
    let text = state.to_text(input);
    if state.read_only()
        || !memory
            .copied_text_for_cursors(state)
            .is_some_and(|copied| text.iter_lines().eq(state.to_text(copied).iter_lines()))
    {
        return Ok(None);
    }
    plan_paste(
        state,
        input,
        &text,
        Some(|part: &str| format!("{part}{line_break}")),
        lines,
    )
    .map(Some)
}

fn plan_paste(
    state: &EditorState,
    input: &str,
    text: &Text,
    linewise_insert: Option<impl Fn(&str) -> String>,
    target_lines: ClipboardLines<'_>,
) -> Result<Transaction, EditError> {
    let by_line = text.line_count() == state.selection().ranges().len();
    let mut lines = text.iter_lines();
    let mut last_line = 0;
    let transaction = state.change_by_range(|selection| {
        let range = selection.range();
        let from = range.start;
        let (range, insert) = if let Some(insert) = &linewise_insert {
            let line = target_lines
                .line_at(from)
                .ok_or(EditError::InvalidSelection)?;
            if line.number == last_line {
                return Ok(RangeEdit {
                    changes: ChangeSet::default(),
                    selection,
                    effects: Vec::new(),
                });
            }
            last_line = line.number;
            let input = if by_line {
                lines.next().expect("one line per range")
            } else {
                input
            };
            (line.range.start..line.range.start, insert(input))
        } else if by_line {
            (range, lines.next().expect("one line per range").to_owned())
        } else {
            (range, text.as_str().to_owned())
        };
        let end = from
            .checked_add(insert.len())
            .ok_or(EditError::InvalidRange)?;
        Ok(RangeEdit {
            changes: ChangeSet::single(Change::new(range, insert))?,
            selection: EditorSelection::cursor(end, Some(Assoc::Before), None, None),
            effects: Vec::new(),
        })
    })?;
    Ok(transaction
        .with_user_event("input.paste")
        .with_scroll_into_view())
}
