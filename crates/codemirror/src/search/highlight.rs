use crate::{
    EditError, EditorState, RangeSet,
    view::{MarkDecoration, Viewport},
};

use super::{Pattern, bytes_for_units, query_state::stored_query, search_panel_open};

const REGEXP_MARGIN: usize = 250;

/// Calculate marks for the stored query while the search panel is open.
/// The scan extends beyond visible ranges to include matches that cross an
/// edge. The host clips the marks when drawing. Empty matches have no mark.
/// Return an error if a visible range is not valid in this state.
pub fn highlight_search_matches(
    state: &EditorState,
    viewport: &Viewport,
) -> Result<RangeSet<MarkDecoration>, EditError> {
    let text = state.text();
    let parts = viewport.visible_ranges();
    if parts
        .iter()
        .any(|range| !text.is_valid_range(range.clone()))
    {
        return Err(EditError::InvalidRange);
    }
    if !search_panel_open(state) {
        return Ok(RangeSet::empty());
    }
    let Some(query) = stored_query(state) else {
        return Ok(RangeSet::empty());
    };
    let Ok(pattern) = query.pattern() else {
        return Ok(RangeSet::empty());
    };
    let margin = match pattern {
        Pattern::Literal(literal) => literal.units,
        Pattern::RegExp(_) => REGEXP_MARGIN,
    };
    let source = text.as_str();
    let mut marks = Vec::new();
    let mut parts = parts.iter().peekable();
    while let Some(part) = parts.next() {
        let mut end = part.end;
        while let Some(next) = parts.next_if(|next| {
            source[end..next.start]
                .encode_utf16()
                .take(2 * REGEXP_MARGIN)
                .count()
                < 2 * REGEXP_MARGIN
        }) {
            end = next.end;
        }
        let start = part.start - bytes_for_units(source[..part.start].chars().rev(), margin);
        let end = end + bytes_for_units(source[end..].chars(), margin);
        for found in query.cursor_in(text, start..end, pattern, Some(state)) {
            if found.range.is_empty() {
                continue;
            }
            let selected = state
                .selection()
                .ranges()
                .iter()
                .any(|range| range.range() == found.range);
            let class = if selected {
                "cm-searchMatch cm-searchMatch-selected"
            } else {
                "cm-searchMatch"
            };
            marks.push(MarkDecoration::new(class).range(found.range)?);
        }
    }
    // Wide literal margins can overlap even after merging nearby visible ranges.
    Ok(RangeSet::of(marks, true).expect("all search marks have nonempty ranges"))
}
