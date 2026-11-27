use std::num::NonZeroUsize;

use crate::word::ClusterBreaks;

/// Count text columns up to a UTF-8 byte offset. Tabs advance to the next
/// tab stop. A non-tab cluster counts once, even if `to` is inside it.
/// Offsets past the end count the whole string. Reject an offset inside a scalar.
pub fn count_column(text: &str, tab_size: NonZeroUsize, to: usize) -> Option<usize> {
    let to = to.min(text.len());
    if !text.is_char_boundary(to) {
        return None;
    }
    columns(text, tab_size, false).find_map(|(byte, column)| (byte >= to).then_some(column))
}

/// Find the first text-column stop at or after the requested column. The
/// result is a UTF-8 byte offset. If text ends before that column, return
/// `None` when strict, or the end offset otherwise.
pub fn find_column(
    text: &str,
    column: usize,
    tab_size: NonZeroUsize,
    strict: bool,
) -> Option<usize> {
    columns(text, tab_size, true)
        .find_map(|(byte, at)| (at >= column).then_some(byte))
        .or_else(|| (!strict).then_some(text.len()))
}

fn columns(
    text: &str,
    tab_size: NonZeroUsize,
    tabs_are_clusters: bool,
) -> impl Iterator<Item = (usize, usize)> {
    let tab_size = tab_size.get();
    let mut clusters = ClusterBreaks::new(text);
    let (mut byte, mut column) = (0, 0usize);
    std::iter::once((byte, column)).chain(std::iter::from_fn(move || {
        let ch = *text.as_bytes().get(byte)?;
        let tab = ch == b'\t';
        column = column.saturating_add(if tab { tab_size - column % tab_size } else { 1 });
        // The pinned count skips one unit for a tab. The inverse lookup
        // advances through any extending characters after it as well.
        byte = if tab && !tabs_are_clusters {
            byte + 1
        } else {
            clusters.next(byte, true)
        };
        Some((byte, column))
    }))
}
