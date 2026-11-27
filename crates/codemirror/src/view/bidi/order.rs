//! Directional classification and visual span ordering from CodeMirror view 6.44.0.

use std::{iter, ops::Range};

use super::{BidiIsolate, BidiSpan, Direction};
use crate::EditError;

use Kind::{Al, An, Cs, En, Et, L, Ni, Nsm, R};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    L,
    R,
    Al,
    En,
    An,
    Et,
    Cs,
    Ni,
    Nsm,
}

impl Kind {
    fn strong(self) -> bool {
        matches!(self, L | R | Al)
    }
}

const LOW_TYPES: &[u8; 0xf8] = b"88888888888888888888888888888888888666888888787833333333337888888000000000000000000000000008888880000000000000000000000000088888888888888888888888888888888888887866668888088888663380888308888800000000000000000000000800000000000000000000000000000008";
const ARABIC_TYPES: &[u8; 0xfa] = b"4444448826627288999999999992222222222222222222222222222222222222222222222229999999999999999999994444444444644222822222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222999999949999999229989999223333333333";

fn char_type(ch: char) -> Kind {
    const TYPES: [Kind; 10] = [L, R, Al, En, An, Ni, Et, Cs, Ni, Nsm];
    match ch as usize {
        c @ 0..=0xf7 => TYPES[(LOW_TYPES[c] - b'0') as usize],
        0x590..=0x5f4 => R,
        c @ 0x600..=0x6f9 => TYPES[(ARABIC_TYPES[c - 0x600] - b'0') as usize],
        0x6fa..=0x8ac | 0xfb50..=0xfdff => Al,
        0x2000..=0x200c => Ni,
        _ => L,
    }
}

fn has_bidi(line: &str) -> bool {
    line.chars()
        .any(|ch| matches!(ch as u32, 0x590..=0x5f4 | 0x600..=0x8ac | 0xfb50..=0xfdff))
}

struct Isolate {
    from: usize,
    to: usize,
    direction: Direction,
    inner: Vec<Self>,
}

pub(super) fn compute(
    line: &str,
    direction: Direction,
    isolates: &[BidiIsolate],
) -> Result<Vec<BidiSpan>, EditError> {
    if isolates.is_empty() && (line.is_empty() || direction == Direction::Ltr && !has_bidi(line)) {
        return Ok(vec![BidiSpan {
            from: 0,
            to: line.len(),
            level: usize::from(direction == Direction::Rtl),
        }]);
    }

    // The pinned classifier treats both UTF-16 surrogates as L. One L per
    // supplementary character produces the same run on valid UTF-8 boundaries.
    let (mut bytes, chars): (Vec<_>, Vec<_>) = line.char_indices().unzip();
    bytes.push(line.len());
    let len = chars.len();
    let isolates = resolve_isolates(&bytes, &chars, 0..len, isolates, super::MAX_ISOLATE_DEPTH)?;
    let base = usize::from(direction == Direction::Rtl);
    let mut ordering = Ordering {
        line: &chars,
        kinds: vec![Ni; len],
        brackets: Vec::new(),
        base,
        spans: Vec::new(),
    };
    ordering.section(0..len, base, &isolates.iter().collect::<Vec<_>>());
    for span in &mut ordering.spans {
        span.from = bytes[span.from];
        span.to = bytes[span.to];
    }
    Ok(ordering.spans)
}

fn resolve_isolates(
    bytes: &[usize],
    line: &[char],
    region: Range<usize>,
    isolates: &[BidiIsolate],
    remaining_depth: usize,
) -> Result<Vec<Isolate>, EditError> {
    if remaining_depth == 0 && !isolates.is_empty() {
        return Err(EditError::InvalidRange);
    }
    let mut end = region.start;
    isolates
        .iter()
        .map(|isolate| {
            let from = bytes
                .binary_search(&isolate.from)
                .map_err(|_| EditError::InvalidRange)?;
            let to = bytes
                .binary_search(&isolate.to)
                .map_err(|_| EditError::InvalidRange)?;
            if from < end || from >= to || to > region.end {
                return Err(EditError::InvalidRange);
            }
            end = to;
            let inner =
                resolve_isolates(bytes, line, from..to, &isolate.inner, remaining_depth - 1)?;
            let direction = isolate.direction.unwrap_or_else(|| {
                gaps(from..to, inner.iter())
                    .flatten()
                    .find_map(|i| match char_type(line[i]) {
                        L => Some(Direction::Ltr),
                        R | Al => Some(Direction::Rtl),
                        _ => None,
                    })
                    .unwrap_or(Direction::Ltr)
            });
            Ok(Isolate {
                from,
                to,
                direction,
                inner,
            })
        })
        .collect()
}

fn gaps<'a>(
    region: Range<usize>,
    isolates: impl Iterator<Item = &'a Isolate> + Clone,
) -> impl Iterator<Item = Range<usize>> + Clone {
    let mut cursor = region.start;
    isolates.map(Some).chain(iter::once(None)).map(move |iso| {
        let gap = cursor..iso.map_or(region.end, |iso| iso.from);
        cursor = iso.map_or(region.end, |iso| iso.to);
        gap
    })
}

const OPPOSITE_BEFORE: u8 = 1;
const EMBED_INSIDE: u8 = 2;
const OPPOSITE_INSIDE: u8 = 4;
const MAX_BRACKETS: usize = 63;

#[derive(Clone, Copy)]
struct Bracket {
    index: usize,
    close: char,
    flags: u8,
}

struct Ordering<'a> {
    line: &'a [char],
    kinds: Vec<Kind>,
    brackets: Vec<Bracket>,
    base: usize,
    spans: Vec<BidiSpan>,
}

impl Ordering<'_> {
    fn section(&mut self, region: Range<usize>, level: usize, isolates: &[&Isolate]) {
        let outer = level_type(level);
        self.compute_types(region.clone(), isolates, outer);
        self.resolve_brackets(region.clone(), isolates, outer);
        self.resolve_neutrals(region.clone(), isolates, outer);
        self.emit(region, level, isolates);
    }

    fn compute_types(&mut self, region: Range<usize>, isolates: &[&Isolate], outer: Kind) {
        let limit = region.end;
        for (gap_index, gap) in gaps(region, isolates.iter().copied()).enumerate() {
            let start_type = if gap_index == 0 { outer } else { Ni };
            // W1–W3: inherit the type for marks, normalize numbers after Arabic
            // letters, and treat Arabic letters as R for later rules.
            let (mut prev, mut strong) = (start_type, start_type);
            for i in gap.clone() {
                let mut kind = char_type(self.line[i]);
                if kind == Nsm {
                    kind = prev;
                } else if kind == En && strong == Al {
                    kind = An;
                }
                self.kinds[i] = if kind == Al { R } else { kind };
                if kind.strong() {
                    strong = kind;
                }
                prev = kind;
            }

            // Normalize separators and terminators, and treat European digits
            // after L as L. Keep the pre-normalized type in `prev`.
            (prev, strong) = (start_type, start_type);
            let mut i = gap.start;
            while i < gap.end {
                let mut kind = self.kinds[i];
                let mut next = i + 1;
                if kind == Cs {
                    if next < gap.end && prev == self.kinds[next] && matches!(prev, En | An) {
                        kind = prev;
                        self.kinds[i] = prev;
                    } else {
                        self.kinds[i] = Ni;
                    }
                } else if kind == Et {
                    while next < gap.end && self.kinds[next] == Et {
                        next += 1;
                    }
                    let replace =
                        if (i != 0 && prev == En) || (next < limit && self.kinds[next] == En) {
                            if strong == L { L } else { En }
                        } else {
                            Ni
                        };
                    self.kinds[i..next].fill(replace);
                } else if kind == En && strong == L {
                    self.kinds[i] = L;
                }
                if kind.strong() {
                    strong = kind;
                }
                prev = kind;
                i = next;
            }
        }
    }

    fn resolve_brackets(&mut self, region: Range<usize>, isolates: &[&Isolate], outer: Kind) {
        let opposite = if outer == L { R } else { L };
        let (mut open, mut context) = (0, 0);
        // N0: match (), [] and {} across the outer text, skipping isolates.
        for gap in gaps(region, isolates.iter().copied()) {
            for i in gap {
                let ch = self.line[i];
                let close = match ch {
                    '(' => Some(')'),
                    '[' => Some(']'),
                    '{' => Some('}'),
                    _ => None,
                };
                if let Some(close) = close {
                    if self.brackets.len() == MAX_BRACKETS {
                        break;
                    }
                    let bracket = Bracket {
                        index: i,
                        close,
                        flags: context,
                    };
                    if open == self.brackets.len() {
                        self.brackets.push(bracket);
                    } else {
                        self.brackets[open] = bracket;
                    }
                    open += 1;
                } else if matches!(ch, ')' | ']' | '}') {
                    if let Some(j) = self.brackets[..open].iter().rposition(|b| b.close == ch) {
                        let Bracket { index, flags, .. } = self.brackets[j];
                        let replace = if flags & EMBED_INSIDE != 0 {
                            Some(outer)
                        } else if flags & OPPOSITE_INSIDE != 0 {
                            Some(if flags & OPPOSITE_BEFORE != 0 {
                                opposite
                            } else {
                                outer
                            })
                        } else {
                            None
                        };
                        if let Some(kind) = replace {
                            self.kinds[index] = kind;
                            self.kinds[i] = kind;
                        }
                        open = j;
                    }
                } else if matches!(self.kinds[i], L | R) {
                    let embed = self.kinds[i] == outer;
                    context = if embed { 0 } else { OPPOSITE_BEFORE };
                    for bracket in self.brackets[..open].iter_mut().rev() {
                        if bracket.flags & EMBED_INSIDE != 0 {
                            break;
                        }
                        if embed {
                            bracket.flags |= EMBED_INSIDE;
                        } else if bracket.flags & OPPOSITE_INSIDE != 0 {
                            break;
                        } else {
                            bracket.flags |= OPPOSITE_INSIDE;
                        }
                    }
                }
            }
        }
    }

    fn resolve_neutrals(&mut self, region: Range<usize>, isolates: &[&Isolate], outer: Kind) {
        let mut indices = gaps(region, isolates.iter().copied()).flatten().peekable();
        let mut prev = outer;
        // N1–N2: resolve neutrals from surrounding directions, treating digits
        // as R. If those directions differ, use the embedding direction.
        while let Some(i) = indices.next() {
            if self.kinds[i] == Ni {
                let mut rest = indices.clone();
                while rest.next_if(|&j| self.kinds[j] == Ni).is_some() {}
                let after = rest.peek().map_or(outer, |&j| self.kinds[j]);
                let replace = match (prev == L, after == L) {
                    (true, true) => L,
                    (false, false) => R,
                    _ => outer,
                };
                self.kinds[i] = replace;
                while let Some(j) = indices.next_if(|&j| self.kinds[j] == Ni) {
                    self.kinds[j] = replace;
                }
            } else {
                prev = self.kinds[i];
            }
        }
    }

    fn emit(&mut self, region: Range<usize>, level: usize, isolates: &[&Isolate]) {
        let ours = level_type(level);
        let reverse = level % 2 != self.base % 2;
        let (mut at, end) = if reverse {
            (region.end, region.start)
        } else {
            (region.start, region.end)
        };
        let mut iso_index = 0;
        let get_iso = |i: usize| {
            let i = if reverse {
                isolates.len().checked_sub(i + 1)?
            } else {
                i
            };
            isolates.get(i).copied()
        };
        let iso_start = |iso: &Isolate| if reverse { iso.to } else { iso.from };
        let iso_end = |iso: &Isolate| if reverse { iso.from } else { iso.to };
        let advance = |i: usize| if reverse { i - 1 } else { i + 1 };
        let char_index = |i: usize| i.min(advance(i));

        while at != end {
            let first = if get_iso(iso_index).is_some_and(|iso| iso_start(iso) == at) {
                ours
            } else {
                self.kinds[char_index(at)]
            };
            let same = first == ours;
            let local_level = level + usize::from(!same);
            // Distinguish numbers from RTL text in an opposite run of LTR text.
            let mut recurse = (!same && ours == L).then(Vec::new);
            let mut scan = at;
            loop {
                if let Some(iso) = get_iso(iso_index).filter(|iso| iso_start(iso) == scan) {
                    if first == An {
                        break;
                    }
                    if reverse {
                        // The pinned reverse scan takes this isolate before look-ahead.
                        iso_index += 1;
                    }
                    if !same {
                        let mut next = iso_end(iso);
                        let mut check = iso_index + usize::from(!reverse);
                        while let Some(adjacent) =
                            get_iso(check).filter(|iso| iso_start(iso) == next)
                        {
                            next = iso_end(adjacent);
                            check += 1;
                        }
                        if next == end || self.kinds[char_index(next)] == ours {
                            break;
                        }
                    }
                    if !reverse {
                        iso_index += 1;
                    }
                    if let Some(inner) = &mut recurse {
                        inner.push(iso);
                    } else {
                        self.push(at, scan, local_level);
                        let swap = iso.direction != Direction::from_level(local_level);
                        self.section(
                            iso.from..iso.to,
                            level + usize::from(swap),
                            &iso.inner.iter().collect::<Vec<_>>(),
                        );
                        at = iso_end(iso);
                    }
                    scan = iso_end(iso);
                } else if scan == end || (self.kinds[char_index(scan)] == ours) != same {
                    break;
                } else {
                    scan = advance(scan);
                }
            }
            if let Some(inner) = recurse {
                self.emit(at.min(scan)..at.max(scan), level + 1, &inner);
            } else {
                self.push(at, scan, local_level);
            }
            at = scan;
        }
    }

    fn push(&mut self, from: usize, to: usize, level: usize) {
        if from != to {
            self.spans.push(BidiSpan {
                from: from.min(to),
                to: from.max(to),
                level,
            });
        }
    }
}

fn level_type(level: usize) -> Kind {
    if level.is_multiple_of(2) { L } else { R }
}
