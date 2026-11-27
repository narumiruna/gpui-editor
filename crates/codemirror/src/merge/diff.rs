use std::ops::Range;

#[path = "presentable.rs"]
mod presentable;

/// A matching pair of changed ranges, in UTF-8 bytes. An insertion has an empty
/// range in A, and a deletion has an empty range in B.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffChange {
    pub a: Range<usize>,
    pub b: Range<usize>,
}

/// Ordered, non-overlapping changes. The gaps have identical content on both sides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diff {
    pub changes: Vec<DiffChange>,
    /// False if the search limit or work budget required a coarser diff.
    /// True does not guarantee the smallest possible edit script.
    pub precise: bool,
}

/// Limits for the native diff. Search operates on Unicode scalar values, not bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffConfig {
    /// Number of changed characters to search per unmatched region before using
    /// a coarser search. The default is 500.
    pub scan_limit: usize,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self { scan_limit: 500 }
    }
}

/// Compute a diff with the default limits.
pub fn diff(a: &str, b: &str) -> Diff {
    DiffConfig::default().diff(a, b)
}

/// Compute a diff and align changes to words and lines for display.
pub fn presentable_diff(a: &str, b: &str) -> Diff {
    DiffConfig::default().presentable_diff(a, b)
}

impl DiffConfig {
    pub fn diff(self, a: &str, b: &str) -> Diff {
        self.compute(a, b, false)
    }

    pub fn presentable_diff(self, a: &str, b: &str) -> Diff {
        self.compute(a, b, true)
    }

    fn compute(self, a: &str, b: &str, presentable: bool) -> Diff {
        if a == b {
            return Diff {
                changes: Vec::new(),
                precise: true,
            };
        }
        let (a, b) = (Input::new(a), Input::new(b));
        let depth = self.scan_limit / 2;
        let mut budget = Budget(
            (a.len() + b.len())
                .saturating_mul(16)
                .saturating_add(depth.min(1024).pow(2) * 4),
        );
        let mut result = find_diff(&a, &b, depth, &mut budget);
        normalize(&mut result.changes, &a.chars, &b.chars);
        if presentable {
            presentable::make_presentable(&mut result.changes, &a.chars, &b.chars);
        }
        for change in &mut result.changes {
            change.a = a.bytes(change.a.clone());
            change.b = b.bytes(change.b.clone());
        }
        result
    }
}

struct Input<'a> {
    text: &'a str,
    chars: Vec<char>,
    offsets: Vec<usize>,
}

impl<'a> Input<'a> {
    fn new(text: &'a str) -> Self {
        let (chars, mut offsets): (Vec<_>, Vec<_>) =
            text.char_indices().map(|(byte, ch)| (ch, byte)).unzip();
        offsets.push(text.len());
        Self {
            text,
            chars,
            offsets,
        }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn bytes(&self, range: Range<usize>) -> Range<usize> {
        self.offsets[range.start]..self.offsets[range.end]
    }

    fn slice(&self, range: Range<usize>) -> &str {
        &self.text[self.bytes(range)]
    }

    fn char_index(&self, offset: usize) -> usize {
        self.offsets.binary_search(&offset).unwrap()
    }
}

struct Budget(usize);

impl Budget {
    fn spend(&mut self, amount: usize) -> bool {
        let permitted = amount <= self.0;
        self.0 = self.0.saturating_sub(amount);
        permitted
    }
}

fn common_prefix(a: &[char], b: &[char]) -> usize {
    a.iter().zip(b).take_while(|(a, b)| a == b).count()
}

fn common_suffix(a: &[char], b: &[char]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

fn trim(change: &mut DiffChange, a: &[char], b: &[char]) {
    let prefix = common_prefix(&a[change.a.clone()], &b[change.b.clone()]);
    change.a.start += prefix;
    change.b.start += prefix;
    let suffix = common_suffix(&a[change.a.clone()], &b[change.b.clone()]);
    change.a.end -= suffix;
    change.b.end -= suffix;
}

fn find_diff(a: &Input<'_>, b: &Input<'_>, depth: usize, budget: &mut Budget) -> Diff {
    let mut result = Diff {
        changes: Vec::new(),
        precise: true,
    };
    let mut pending = vec![DiffChange {
        a: 0..a.len(),
        b: 0..b.len(),
    }];
    let mut frontiers = [Frontier::default(), Frontier::default()];
    while let Some(mut change) = pending.pop() {
        trim(&mut change, &a.chars, &b.chars);
        let (len_a, len_b) = (change.a.len(), change.b.len());
        if len_a == 0 || len_b == 0 {
            if len_a != len_b {
                result.changes.push(change);
            }
            continue;
        }

        let shared = if len_a > len_b {
            contained(a, &change.a, b, &change.b, budget)
                .map(|start| (start, change.b.start, len_b))
        } else if len_b > len_a {
            contained(b, &change.b, a, &change.a, budget)
                .map(|start| (change.a.start, start, len_a))
        } else {
            None
        };
        let split = shared.or_else(|| {
            let (long, short) = (len_a.max(len_b), len_a.min(len_b));
            if short > 128 && depth != 0 {
                // A short search handles nearby revisions of a large file
                // without picking an arbitrary occurrence of a repeated seed.
                let limit = (len_a + len_b).saturating_mul(4).min(budget.0 / 2);
                let mut probe = Budget(limit);
                let split = snake(
                    &a.chars[change.a.clone()],
                    &b.chars[change.b.clone()],
                    &mut frontiers,
                    depth.min(16),
                    &mut probe,
                );
                budget.spend(limit - probe.0);
                if let Search::Split(x, y) = split {
                    return Some((change.a.start + x, change.b.start + y, 0));
                }
            }
            (long >= 4 && short >= long.div_ceil(2))
                .then(|| find_match(a, b, &change, long / 4, budget))
                .flatten()
        });
        let split = split.or_else(|| {
            if len_a == 1 || len_b == 1 {
                return None;
            }
            match snake(
                &a.chars[change.a.clone()],
                &b.chars[change.b.clone()],
                &mut frontiers,
                depth,
                budget,
            ) {
                Search::Split(x, y) => Some((change.a.start + x, change.b.start + y, 0)),
                Search::NoMatch => None,
                Search::Limit => {
                    result.precise = false;
                    let mut size = len_a.min(len_b) / 6;
                    while size >= 50 && budget.0 > 0 {
                        if let Some(shared) = find_match(a, b, &change, size, budget) {
                            return Some(shared);
                        }
                        size /= 2;
                    }
                    None
                }
            }
        });
        if let Some((x, y, len)) = split
            && (x, y) != (change.a.end, change.b.end)
            && (x + len, y + len) != (change.a.start, change.b.start)
        {
            pending.push(DiffChange {
                a: x + len..change.a.end,
                b: y + len..change.b.end,
            });
            pending.push(DiffChange {
                a: change.a.start..x,
                b: change.b.start..y,
            });
        } else {
            result.changes.push(change);
        }
    }
    result
}

fn contained(
    long: &Input<'_>,
    outer: &Range<usize>,
    short: &Input<'_>,
    inner: &Range<usize>,
    budget: &mut Budget,
) -> Option<usize> {
    let text = long.slice(outer.clone());
    // The one-character search is terminal. Its follow-up ranges have an
    // empty side, so those bytes will not be searched again.
    if inner.len() != 1 && !budget.spend(1 + text.len() / 32) {
        return None;
    }
    let offset = text.find(short.slice(inner.clone()))?;
    Some(long.char_index(long.offsets[outer.start] + offset))
}

// Test evenly spaced seeds from the longer range. Extend hits in both directions.
fn find_match(
    a: &Input<'_>,
    b: &Input<'_>,
    change: &DiffChange,
    size: usize,
    budget: &mut Budget,
) -> Option<(usize, usize, usize)> {
    let (long, short, outer, inner, swap) = if change.a.len() >= change.b.len() {
        (a, b, &change.a, &change.b, false)
    } else {
        (b, a, &change.b, &change.a, true)
    };
    // Repeated seeds can match at many positions. Limit their work here so
    // the general search still has budget if the seeds do not help.
    let mut allowance = Budget(
        (outer.len() + inner.len())
            .saturating_mul(4)
            .min(budget.0 / 2),
    );
    let mut spend = |cost| allowance.spend(cost) && budget.spend(cost);
    let mut best = None;
    for start in (outer.start + size..outer.end - size).step_by(size) {
        let end = start + size;
        let seed = long.slice(start..end);
        let mut from = inner.start;
        while from + size <= inner.end {
            // Reserve the cost of this substring search before scanning. Charge
            // for seed bytes too, so overlapping hits cannot cause unbounded work.
            let text = short.slice(from..inner.end);
            if !spend(1 + (text.len() + seed.len()) / 32) {
                return best;
            }
            let Some(offset) = text.find(seed) else {
                break;
            };
            let hit = short.char_index(short.offsets[from] + offset);
            let suffix = common_suffix(
                &long.chars[outer.start..start],
                &short.chars[inner.start..hit],
            );
            let prefix = common_prefix(
                &long.chars[end..outer.end],
                &short.chars[hit + size..inner.end],
            );
            let (x, y, len) = (start - suffix, hit - suffix, size + prefix + suffix);
            if best.is_none_or(|(_, _, best_len)| best_len < len) {
                best = Some(if swap { (y, x, len) } else { (x, y, len) });
            }
            if !spend(1 + prefix + suffix) {
                return best;
            }
            from = hit + 1;
        }
    }
    best
}

#[derive(Default)]
struct Frontier {
    reach: Vec<isize>,
    start: isize,
    end: isize,
}

enum Search {
    Split(usize, usize),
    NoMatch,
    Limit,
}

// Myers' bidirectional search. Only keep diagonals within the allowed depth.
fn snake(
    a: &[char],
    b: &[char],
    frontiers: &mut [Frontier; 2],
    depth: usize,
    budget: &mut Budget,
) -> Search {
    let max = (a.len() + b.len()).div_ceil(2) - 1;
    let depth = depth.min(max).min(budget.0.isqrt());
    let [forward, backward] = frontiers;
    for frontier in [&mut *forward, &mut *backward] {
        frontier.reset(depth);
    }
    let odd = !a.len().abs_diff(b.len()).is_multiple_of(2);
    for d in 0..=depth as isize {
        let f = forward.advance(a, b, d, false, odd.then_some(&*backward), budget);
        let result =
            f.or_else(|| backward.advance(a, b, d, true, (!odd).then_some(&*forward), budget));
        if let Some((x, y)) = result {
            return Search::Split(x, y);
        }
        if budget.0 == 0 {
            return Search::Limit;
        }
    }
    if depth == max {
        Search::NoMatch
    } else {
        Search::Limit
    }
}

impl Frontier {
    fn reset(&mut self, depth: usize) {
        self.reach.clear();
        self.reach.resize(2 * depth + 3, -1);
        self.reach[depth + 2] = 0;
        self.start = 0;
        self.end = 0;
    }

    fn advance(
        &mut self,
        a: &[char],
        b: &[char],
        d: isize,
        reverse: bool,
        other: Option<&Self>,
        budget: &mut Budget,
    ) -> Option<(usize, usize)> {
        let center = (self.reach.len() / 2) as isize;
        let (len_x, len_y) = (a.len() as isize, b.len() as isize);
        let mut k = -d + self.start;
        while k <= d - self.end {
            if !budget.spend(1) {
                break;
            }
            let i = (center + k) as usize;
            let mut x = if k == -d || (k != d && self.reach[i - 1] < self.reach[i + 1]) {
                self.reach[i + 1]
            } else {
                self.reach[i - 1] + 1
            };
            let mut y = x - k;
            while (0..len_x).contains(&x) && (0..len_y).contains(&y) {
                let same = if reverse {
                    a[(len_x - x - 1) as usize] == b[(len_y - y - 1) as usize]
                } else {
                    a[x as usize] == b[y as usize]
                };
                if !same || !budget.spend(1) {
                    break;
                }
                x += 1;
                y += 1;
            }
            self.reach[i] = x;
            if x > len_x {
                self.end += 2;
            } else if y > len_y {
                self.start += 2;
            } else if let Some(other) = other {
                let k_other = len_x - len_y - k;
                if let Some(&other_x) = other.reach.get((center + k_other) as usize)
                    && other_x >= 0
                    && x + other_x >= len_x
                {
                    return Some(if reverse {
                        (other_x as usize, (other_x - k_other) as usize)
                    } else {
                        (x as usize, y as usize)
                    });
                }
            }
            k += 2;
        }
        None
    }
}

fn merge_adjacent(changes: &mut Vec<DiffChange>, join: impl Fn(&DiffChange, &DiffChange) -> bool) {
    let mut keep = 0;
    for read in 0..changes.len() {
        if keep > 0 && join(&changes[keep - 1], &changes[read]) {
            changes[keep - 1].a.end = changes[read].a.end;
            changes[keep - 1].b.end = changes[read].b.end;
        } else {
            changes.swap(keep, read);
            keep += 1;
        }
    }
    changes.truncate(keep);
}

fn normalize(changes: &mut Vec<DiffChange>, a: &[char], b: &[char]) {
    // Bound repeated shifts on large groups of adjacent changes.
    for _ in 0..8 {
        merge_adjacent(changes, touching);
        let mut moved = false;
        for i in 0..changes.len() {
            let prev = i.checked_sub(1).map_or(0, |i| changes[i].a.end);
            let next = changes.get(i + 1).map_or(a.len(), |ch| ch.a.start);
            let ch = &mut changes[i];
            trim(ch, a, b);
            if ch.a.is_empty() == ch.b.is_empty() {
                continue;
            }
            let (before, after) = (ch.a.start - prev, next - ch.a.end);
            if before == 0 || after == 0 {
                continue;
            }
            let changed = if ch.a.is_empty() {
                &b[ch.b.clone()]
            } else {
                &a[ch.a.clone()]
            };
            if changed.ends_with(&a[prev..ch.a.start]) {
                shift_left(ch, before);
                moved = true;
            } else if changed.starts_with(&a[ch.a.end..next]) {
                shift_right(ch, after);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    merge_adjacent(changes, touching);
    changes.retain(|ch| !(ch.a.is_empty() && ch.b.is_empty()));
}

fn touching(previous: &DiffChange, next: &DiffChange) -> bool {
    previous.a.end >= next.a.start && previous.b.end >= next.b.start
}

fn shift_left(ch: &mut DiffChange, by: usize) {
    ch.a = ch.a.start - by..ch.a.end - by;
    ch.b = ch.b.start - by..ch.b.end - by;
}

fn shift_right(ch: &mut DiffChange, by: usize) {
    ch.a = ch.a.start + by..ch.a.end + by;
    ch.b = ch.b.start + by..ch.b.end + by;
}
