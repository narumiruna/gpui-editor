use std::{
    fmt,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

#[path = "presentable.rs"]
mod presentable;
#[cfg(test)]
mod timeout_tests;

/// A matching pair of changed ranges in UTF-8 bytes. An insertion has an empty
/// range in A, and a deletion has an empty range in B. A trimmed split can be
/// empty on both sides and still affect how display ranges join.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffChange {
    pub a: Range<usize>,
    pub b: Range<usize>,
}

/// Ordered, non-overlapping changes. The gaps have identical content on both sides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diff {
    pub changes: Vec<DiffChange>,
    /// False if the search limit, work budget, or timeout required a coarser diff.
    /// True does not guarantee the smallest possible edit script.
    pub precise: bool,
}

/// Limits for the native diff. Large regions are searched in Unicode scalars.
/// Short regions can use UTF-16 units to match the upstream alignment.
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
        self.diff_with_timeout(a, b, Duration::ZERO)
    }

    pub fn presentable_diff(self, a: &str, b: &str) -> Diff {
        self.presentable_diff_with_timeout(a, b, Duration::ZERO)
    }

    /// Compute a diff with an optional elapsed time limit. Zero disables it.
    /// The main search checks the limit periodically and can use a coarser diff
    /// after it expires. This does not set a maximum run time.
    pub fn diff_with_timeout(self, a: &str, b: &str, timeout: Duration) -> Diff {
        self.compute(a, b, false, timeout, &Instant::now)
    }

    /// Compute a display diff with the same limit as [`Self::diff_with_timeout`].
    pub fn presentable_diff_with_timeout(self, a: &str, b: &str, timeout: Duration) -> Diff {
        self.compute(a, b, true, timeout, &Instant::now)
    }

    fn compute(
        self,
        a: &str,
        b: &str,
        presentable: bool,
        timeout: Duration,
        now: &dyn Fn() -> Instant,
    ) -> Diff {
        let deadline = Deadline::new(timeout, now);
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
        let mut result = find_diff(&a, &b, depth, &mut budget, deadline.as_ref());
        normalize(&mut result.changes, &a, &b);
        if presentable {
            presentable::make_presentable(&mut result.changes, &a.chars, &b.chars);
        }
        to_byte_ranges(&mut result.changes, &a, &b);
        result
    }
}

/// A custom diff that bypasses the native search and its limits.
///
/// The callback receives A then B, including when the inputs are equal. It
/// returns ordered ranges in UTF-8 bytes, relative to those inputs. The text
/// before, between, and after the ranges must match exactly on both sides.
#[derive(Clone)]
pub struct DiffOverride(Arc<OverrideFn>);

type OverrideFn = dyn Fn(&str, &str) -> Vec<DiffChange> + Send + Sync;

impl fmt::Debug for DiffOverride {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiffOverride").finish_non_exhaustive()
    }
}

/// The callback returned invalid UTF-8 ranges, overlapping or unordered
/// changes, or unequal text outside the changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidDiffOverride;

impl fmt::Display for InvalidDiffOverride {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid diff override: expected ordered UTF-8 ranges separated by equal text")
    }
}

impl std::error::Error for InvalidDiffOverride {}

impl DiffOverride {
    pub fn new(callback: impl Fn(&str, &str) -> Vec<DiffChange> + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    /// Return the checked ranges without trimming, shifting, or joining them.
    /// A valid override is marked precise; it need not return a minimal diff.
    pub fn diff(&self, a: &str, b: &str) -> Result<Diff, InvalidDiffOverride> {
        let changes = (self.0)(a, b);
        let (mut end_a, mut end_b) = (0, 0);
        for change in &changes {
            if change.a.start < end_a
                || change.b.start < end_b
                || a.get(change.a.clone()).is_none()
                || b.get(change.b.clone()).is_none()
                || a[end_a..change.a.start] != b[end_b..change.b.start]
            {
                return Err(InvalidDiffOverride);
            }
            (end_a, end_b) = (change.a.end, change.b.end);
        }
        if a[end_a..] != b[end_b..] {
            return Err(InvalidDiffOverride);
        }
        Ok(Diff {
            changes,
            precise: true,
        })
    }

    /// Check the ranges, then apply display normalization only. This skips
    /// the raw normalization that follows the native search.
    pub fn presentable_diff(&self, a: &str, b: &str) -> Result<Diff, InvalidDiffOverride> {
        let mut result = self.diff(a, b)?;
        if !result.changes.is_empty() {
            let (a, b) = (Input::new(a), Input::new(b));
            for change in &mut result.changes {
                change.a = a.char_index(change.a.start)..a.char_index(change.a.end);
                change.b = b.char_index(change.b.start)..b.char_index(change.b.end);
            }
            presentable::make_presentable(&mut result.changes, &a.chars, &b.chars);
            to_byte_ranges(&mut result.changes, &a, &b);
        }
        Ok(result)
    }
}

fn to_byte_ranges(changes: &mut [DiffChange], a: &Input<'_>, b: &Input<'_>) {
    for change in changes {
        change.a = a.bytes(change.a.clone());
        change.b = b.bytes(change.b.clone());
    }
}

struct Input<'a> {
    text: &'a str,
    chars: Vec<char>,
    offsets: Vec<usize>,
    supplementary: Vec<usize>,
}

impl<'a> Input<'a> {
    fn new(text: &'a str) -> Self {
        let mut supplementary = Vec::new();
        let (chars, mut offsets): (Vec<_>, Vec<_>) = text
            .char_indices()
            .enumerate()
            .map(|(index, (byte, ch))| {
                if ch.len_utf16() == 2 {
                    supplementary.push(index);
                }
                (ch, byte)
            })
            .unzip();
        offsets.push(text.len());
        Self {
            text,
            chars,
            offsets,
            supplementary,
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

    fn utf16_at(&self, index: usize) -> usize {
        index + self.supplementary.partition_point(|&i| i < index)
    }

    fn char_index(&self, offset: usize) -> usize {
        self.offsets.binary_search(&offset).unwrap()
    }
}

struct Deadline<'a> {
    start: Instant,
    timeout: Duration,
    now: &'a dyn Fn() -> Instant,
}

impl<'a> Deadline<'a> {
    fn new(timeout: Duration, now: &'a dyn Fn() -> Instant) -> Option<Self> {
        (!timeout.is_zero()).then(|| Self {
            start: now(),
            timeout,
            now,
        })
    }

    fn expired(&self) -> bool {
        (self.now)().saturating_duration_since(self.start) > self.timeout
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

fn common_prefix(a: &Input<'_>, ra: Range<usize>, b: &Input<'_>, rb: Range<usize>) -> usize {
    // The pinned diff skips this prefix when the absolute UTF-16 offsets
    // on the two sides compare equal. Range alignment depends on this guard.
    if a.utf16_at(ra.start) == b.utf16_at(rb.end) {
        return 0;
    }
    a.chars[ra]
        .iter()
        .zip(&b.chars[rb])
        .take_while(|(a, b)| a == b)
        .count()
}

fn common_suffix(a: &[char], b: &[char]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

fn trim(change: &mut DiffChange, a: &Input<'_>, b: &Input<'_>) {
    let prefix = common_prefix(a, change.a.clone(), b, change.b.clone());
    change.a.start += prefix;
    change.b.start += prefix;
    let suffix = common_suffix(&a.chars[change.a.clone()], &b.chars[change.b.clone()]);
    change.a.end -= suffix;
    change.b.end -= suffix;
}

fn find_diff(
    a: &Input<'_>,
    b: &Input<'_>,
    depth: usize,
    budget: &mut Budget,
    deadline: Option<&Deadline<'_>>,
) -> Diff {
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
        trim(&mut change, a, b);
        let (len_a, len_b) = (change.a.len(), change.b.len());
        if len_a == 0 || len_b == 0 {
            result.changes.push(change);
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
            if len_a == 1 || len_b == 1 {
                return None;
            }
            let smaller_units = (a.utf16_at(change.a.end) - a.utf16_at(change.a.start))
                .min(b.utf16_at(change.b.end) - b.utf16_at(change.b.start));
            // Apply the UTF-16 size limit to both Myers searches, not to anchors.
            let over_size_limit = smaller_units > depth.saturating_mul(16);
            if !over_size_limit && len_a.min(len_b) > 128 && depth != 0 {
                // A short search handles nearby revisions of a large file
                // without picking an arbitrary occurrence of a repeated seed.
                // This capped probe uses only the work budget.
                let limit = (len_a + len_b).saturating_mul(4).min(budget.0 / 2);
                let mut probe = Budget(limit);
                let split = snake(
                    &a.chars[change.a.clone()],
                    &b.chars[change.b.clone()],
                    &mut frontiers,
                    depth.min(16),
                    &mut probe,
                    None,
                );
                budget.spend(limit - probe.0);
                if let Search::Split(x, y) = split {
                    return Some((change.a.start + x, change.b.start + y, 0));
                }
            }
            let chars = [&a.chars[change.a.clone()], &b.chars[change.b.clone()]];
            let utf16 = short_utf16(chars, budget);
            if let Some(anchor) = find_match(a, b, &change, MatchMode::Half(utf16.as_ref()), budget)
            {
                return Some(anchor);
            }
            let search = if over_size_limit {
                Search::Limit
            } else if let Some([a, b]) = &utf16 {
                match snake(&a.units, &b.units, &mut frontiers, depth, budget, deadline) {
                    Search::Split(x, y) => match (a.split(x), b.split(y)) {
                        (Some(x), Some(y)) => Search::Split(x, y),
                        _ => Search::NoMatch,
                    },
                    result => result,
                }
            } else {
                snake(
                    &a.chars[change.a.clone()],
                    &b.chars[change.b.clone()],
                    &mut frontiers,
                    depth,
                    budget,
                    deadline,
                )
            };
            match search {
                Search::Split(x, y) => Some((change.a.start + x, change.b.start + y, 0)),
                Search::NoMatch => None,
                Search::Limit => {
                    result.precise = false;
                    if over_size_limit && smaller_units > depth.saturating_mul(64) {
                        return None;
                    }
                    let mut size = len_a.min(len_b) / 6;
                    while size >= 50 && budget.0 > 0 {
                        if let Some(shared) =
                            find_match(a, b, &change, MatchMode::Coarse(size), budget)
                        {
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

enum MatchMode<'a> {
    Half(Option<&'a [Utf16Region; 2]>),
    Coarse(usize),
}

// Test evenly spaced seeds from the longer range. Extend hits in both directions.
fn find_match(
    a: &Input<'_>,
    b: &Input<'_>,
    change: &DiffChange,
    mode: MatchMode<'_>,
    budget: &mut Budget,
) -> Option<(usize, usize, usize)> {
    let [a_utf16, b_utf16] = match mode {
        MatchMode::Half(Some(regions)) => regions.each_ref().map(Some),
        _ => [None, None],
    };
    let len_a = a_utf16.map_or(change.a.len(), |region| region.units.len());
    let len_b = b_utf16.map_or(change.b.len(), |region| region.units.len());
    let (long, short, outer, inner, utf16, swap) = if len_a >= len_b {
        (a, b, &change.a, &change.b, a_utf16, false)
    } else {
        (b, a, &change.b, &change.a, b_utf16, true)
    };
    let long_len = len_a.max(len_b);
    let size = match mode {
        MatchMode::Half(_) if long_len >= 4 && len_a.min(len_b) >= long_len.div_ceil(2) => {
            long_len / 4
        }
        MatchMode::Half(_) => return None,
        MatchMode::Coarse(size) => size,
    };
    let units = |scalar| utf16.map_or(scalar, |region| region.offsets[scalar]);
    // Repeated seeds can match at many positions. Limit their work here so
    // the general search still has budget if the seeds do not help.
    let mut allowance = Budget(
        (outer.len() + inner.len())
            .saturating_mul(4)
            .min(budget.0 / 2),
    );
    let mut spend = |cost| allowance.spend(cost) && budget.spend(cost);
    let mut best = None;
    let mut best_len = 0;
    let mut cursor = size;
    loop {
        let seed_range = match utf16 {
            Some(region) => region.seed(cursor, size),
            None => (cursor + size < long_len).then_some(cursor..cursor + size),
        };
        let Some(range) = seed_range else { break };
        cursor = units(range.end);
        let (start, end) = (outer.start + range.start, outer.start + range.end);
        let seed = long.slice(start..end);
        let mut from = inner.start;
        while from + range.len() <= inner.end {
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
            let prefix = common_prefix(long, end..outer.end, short, hit + range.len()..inner.end);
            let (x, y, len) = (start - suffix, hit - suffix, range.len() + prefix + suffix);
            let match_len = units(x + len - outer.start) - units(x - outer.start);
            if match_len > best_len {
                best_len = match_len;
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

struct Utf16Region {
    units: Vec<u16>,
    // Map each scalar boundary to its offset in `units`.
    offsets: Vec<usize>,
}

fn short_utf16(chars: [&[char]; 2], budget: &mut Budget) -> Option<[Utf16Region; 2]> {
    // Bound the encoding cost independently of the Myers search depth.
    const MAX_SCALARS: usize = 64;
    let scalars = chars[0].len() + chars[1].len();
    if scalars > MAX_SCALARS {
        return None;
    }
    let units = chars
        .into_iter()
        .flatten()
        .map(|ch| ch.len_utf16())
        .sum::<usize>();
    if units == scalars || units > budget.0 {
        return None;
    }
    budget.spend(units);
    Some(chars.map(Utf16Region::new))
}

impl Utf16Region {
    fn new(chars: &[char]) -> Self {
        let mut units = Vec::with_capacity(chars.len() * 2);
        let mut offsets = Vec::with_capacity(chars.len() + 1);
        for &ch in chars {
            offsets.push(units.len());
            units.extend_from_slice(ch.encode_utf16(&mut [0; 2]));
        }
        offsets.push(units.len());
        Self { units, offsets }
    }

    fn seed(&self, from: usize, size: usize) -> Option<Range<usize>> {
        let start = self.offsets.partition_point(|&unit| unit < from);
        let to = self.offsets[start] + size;
        if to >= self.units.len() {
            return None;
        }
        // Start at the next boundary. A one-unit seed must not disappear
        // when its end falls inside a surrogate pair.
        let end = match self.offsets.binary_search(&to) {
            Ok(end) => end,
            Err(next) if size == 1 => next,
            Err(next) => next - 1,
        };
        (end < self.offsets.len() - 1).then_some(start..end)
    }

    fn split(&self, unit: usize) -> Option<usize> {
        match self.offsets.binary_search(&unit) {
            Ok(index) => Some(index),
            // Match the upstream bisection rule: advance over the scalar, but
            // keep the region unsplit if that would land at its end.
            Err(next) => (next < self.offsets.len() - 1).then_some(next),
        }
    }
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
fn snake<T: Eq>(
    a: &[T],
    b: &[T],
    frontiers: &mut [Frontier; 2],
    depth: usize,
    budget: &mut Budget,
    deadline: Option<&Deadline<'_>>,
) -> Search {
    if deadline.is_some_and(Deadline::expired) {
        return Search::Limit;
    }
    let max = (a.len() + b.len()).div_ceil(2) - 1;
    let depth = depth.min(max).min(budget.0.isqrt());
    let [forward, backward] = frontiers;
    for frontier in [&mut *forward, &mut *backward] {
        frontier.reset(depth);
    }
    let odd = !a.len().abs_diff(b.len()).is_multiple_of(2);
    for d in 0..=depth as isize {
        if d % 64 == 0 && deadline.is_some_and(Deadline::expired) {
            return Search::Limit;
        }
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

    fn advance<T: Eq>(
        &mut self,
        a: &[T],
        b: &[T],
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

fn normalize(changes: &mut Vec<DiffChange>, a: &Input<'_>, b: &Input<'_>) {
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
                &b.chars[ch.b.clone()]
            } else {
                &a.chars[ch.a.clone()]
            };
            if changed.ends_with(&a.chars[prev..ch.a.start]) {
                shift_left(ch, before);
                moved = true;
            } else if changed.starts_with(&a.chars[ch.a.end..next]) {
                shift_right(ch, after);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    merge_adjacent(changes, touching);
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
