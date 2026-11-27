use std::{ops::Range, sync::Arc};

use crate::{Range as MarkerRange, RangeSet, RangeSetBuilder, RangeValue, Text};

use super::{Diagnostic, LocatedDiagnostic, Severity};

/// One native diagnostic marker. A point stays before insertions at its position.
#[derive(Clone, Debug)]
pub struct DiagnosticMarker {
    diagnostics: Arc<[Arc<Diagnostic>]>,
    point: bool,
    inclusive_end: bool,
}

impl DiagnosticMarker {
    pub fn diagnostics(&self) -> &[Arc<Diagnostic>] {
        &self.diagnostics
    }

    pub fn severity(&self) -> Severity {
        self.diagnostics
            .iter()
            .map(|d| d.severity)
            .max()
            .unwrap_or(Severity::Hint)
    }
}

impl RangeValue for DiagnosticMarker {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.diagnostics, &other.diagnostics)
            && self.point == other.point
            && self.inclusive_end == other.inclusive_end
    }

    fn start_side(&self) -> i32 {
        if self.point { -2 } else { 2 }
    }

    fn end_side(&self) -> i32 {
        if self.point {
            -2
        } else if self.inclusive_end {
            1
        } else {
            -3
        }
    }

    fn is_point(&self) -> bool {
        self.point
    }
}

/// Split overlapping diagnostics at every start and end. Insertions inside
/// a diagnostic at a split belong to the preceding segment.
pub(super) fn build<'a>(
    text: &Text,
    diagnostics: impl Iterator<Item = &'a Diagnostic>,
) -> RangeSet<DiagnosticMarker> {
    let mut sorted: Vec<_> = diagnostics.collect();
    sorted.sort_by_key(|d| (d.range.start, d.range.end));
    let mut pending = sorted.into_iter().peekable();
    let mut active: Vec<Arc<Diagnostic>> = Vec::new();
    let mut markers = RangeSetBuilder::new();
    let mut pos = 0;
    while !active.is_empty() || pending.peek().is_some() {
        let from = if active.is_empty() {
            pending.peek().expect("a pending diagnostic").range.start
        } else {
            pos
        };
        while pending.peek().is_some_and(|d| d.range.start == from) {
            active.push(Arc::new(
                pending.next().expect("a pending diagnostic").clone(),
            ));
        }
        let to = active
            .iter()
            .map(|d| d.range.end)
            .chain(pending.peek().map(|d| d.range.start))
            .min()
            .expect("an active diagnostic");
        let point = from == to
            || active.iter().any(|d| d.range == (from..to)) && short_line_breaks(text, from..to);
        markers
            .add(
                from,
                if point { from } else { to },
                DiagnosticMarker {
                    inclusive_end: active.iter().any(|d| d.range.end > to),
                    diagnostics: active.as_slice().into(),
                    point,
                },
            )
            .expect("ordered diagnostic marker");
        pos = to;
        if pos == text.len() {
            break;
        }
        active.retain(|d| d.range.end > pos);
    }
    markers.finish()
}

fn short_line_breaks(text: &Text, range: Range<usize>) -> bool {
    let bytes = text.as_str().as_bytes();
    let mut breaks = 0;
    for at in range.clone() {
        match bytes[at] {
            b'\n' => breaks += 1,
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                breaks += usize::from(at + 1 == range.end);
            }
            _ => return false,
        }
        if breaks >= 10 {
            return false;
        }
    }
    true
}

// A point at an inclusive end does not fit in the same upstream range layer.
// Keep those points last for editor navigation. Reconstruct layers from the
// current ranges rather than keeping the upstream chunk and layer history.
pub(super) fn in_layer_order(
    markers: &RangeSet<DiagnosticMarker>,
    after: usize,
) -> impl Iterator<Item = &MarkerRange<DiagnosticMarker>> {
    let layer = |spilled| {
        markers
            .iter(after)
            .scan(None, move |last_end, marker| {
                let spill = if marker.value.point {
                    *last_end == Some(marker.from)
                } else {
                    *last_end = marker.value.inclusive_end.then_some(marker.to);
                    false
                };
                Some((spill == spilled).then_some(marker))
            })
            .flatten()
    };
    layer(false).chain(layer(true))
}

pub(super) fn find(
    markers: &RangeSet<DiagnosticMarker>,
    diagnostic: Option<&Arc<Diagnostic>>,
    after: usize,
) -> Option<LocatedDiagnostic> {
    if let Some(wanted) = diagnostic {
        // Use positional order for actions and list selection, so a point
        // shared with the diagnostic cannot truncate its full mapped range.
        let mut candidates = markers
            .iter(after)
            .filter(|marker| contains(marker.value.diagnostics(), wanted));
        let first = candidates.next()?;
        let to = candidates.fold(first.to, |end, marker| end.max(marker.to));
        return Some(LocatedDiagnostic {
            range: first.from..to,
            diagnostic: wanted.clone(),
        });
    }
    let mut candidates = in_layer_order(markers, after);
    let first = candidates.next()?;
    let diagnostic = &first.value.diagnostics()[0];
    let to = candidates
        .take_while(|marker| contains(marker.value.diagnostics(), diagnostic))
        .fold(first.to, |end, marker| end.max(marker.to));
    Some(LocatedDiagnostic {
        range: first.from..to,
        diagnostic: diagnostic.clone(),
    })
}

pub(super) fn for_each(
    markers: &RangeSet<DiagnosticMarker>,
    mut visit: impl FnMut(LocatedDiagnostic),
) {
    let mut pending: Vec<LocatedDiagnostic> = Vec::new();
    for marker in markers.iter(0) {
        let diagnostics = marker.value.diagnostics();
        pending.retain_mut(|entry| {
            if contains(diagnostics, &entry.diagnostic) {
                entry.range.end = marker.to;
                true
            } else {
                visit(entry.clone());
                false
            }
        });
        for diagnostic in diagnostics {
            if !pending
                .iter()
                .any(|entry| Arc::ptr_eq(&entry.diagnostic, diagnostic))
            {
                pending.push(LocatedDiagnostic {
                    range: marker.from..marker.to,
                    diagnostic: diagnostic.clone(),
                });
            }
        }
    }
    pending.into_iter().for_each(visit);
}

fn contains(diagnostics: &[Arc<Diagnostic>], wanted: &Arc<Diagnostic>) -> bool {
    diagnostics.iter().any(|d| Arc::ptr_eq(d, wanted))
}
