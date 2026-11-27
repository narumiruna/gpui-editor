use std::ops::Range;

/// Walk from the innermost node at the line end. A valid outer fold on the
/// same line replaces the inner one. Keep the inner fold when an ancestor
/// starts on an earlier line.
pub(in crate::language) fn fold_on_line<N: Copy>(
    nodes: impl Iterator<Item = N>,
    line: Range<usize>,
    range: impl Fn(N) -> Range<usize>,
    mut proposal: impl FnMut(N) -> Option<Range<usize>>,
) -> Option<Range<usize>> {
    let mut found = None;
    for node in nodes {
        let span = range(node);
        if span.end <= line.end || span.start > line.end {
            continue;
        }
        if found.is_some() && span.start < line.start {
            break;
        }
        if let Some(fold) = proposal(node)
            && (line.start..=line.end).contains(&fold.start)
            && line.end < fold.end
        {
            found = Some(fold);
        }
    }
    found
}
