//! Layer construction and chunk reuse, based on CodeMirror state 6.7.6.
//! See `docs/licenses/codemirror-state.txt` for the upstream MIT notice.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{Assoc, ChangeDesc, ChangeSet, MapMode, RangeTouch};

use super::{Range, RangeSet, RangeSetUpdate, RangeValue, StoredRange, store_ranges};

const CHUNK_SIZE: usize = 250;
pub(super) type Edge = (usize, i32);
pub(super) type SharedChunks = HashSet<*const ()>;

pub(super) trait Mapping {
    fn touch(&self, from: usize, to: usize) -> RangeTouch;
    fn position(&self, at: usize, assoc: Assoc, mode: MapMode) -> Option<usize>;

    fn simple_position(&self, at: usize, assoc: Assoc) -> usize {
        self.position(at, assoc, MapMode::Simple)
            .expect("invalid byte position or mapped position exceeds usize")
    }
}

impl Mapping for ChangeSet {
    fn touch(&self, from: usize, to: usize) -> RangeTouch {
        let first = self
            .changes()
            .take_while(|(change, _)| change.start <= to)
            .find(|(change, _)| change.end >= from);
        match first {
            Some((change, _)) if change.start < from && change.end > to => RangeTouch::Cover,
            Some(_) => RangeTouch::Touch,
            None => RangeTouch::None,
        }
    }

    fn position(&self, at: usize, assoc: Assoc, mode: MapMode) -> Option<usize> {
        self.map_pos_mode(at, assoc, mode)
    }
}

impl Mapping for ChangeDesc {
    fn touch(&self, from: usize, to: usize) -> RangeTouch {
        self.touches_range(from..to)
    }

    fn position(&self, at: usize, assoc: Assoc, mode: MapMode) -> Option<usize> {
        self.map_pos(at, assoc, mode)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Layer {
    pub(super) chunks: Vec<Chunk>,
    pub(super) max_point: Option<usize>,
    id: Arc<()>,
}

impl Layer {
    fn new(chunks: Vec<Chunk>) -> Self {
        let max_point = chunks.iter().filter_map(|chunk| chunk.max_point).max();
        Self {
            chunks,
            max_point,
            id: Arc::new(()),
        }
    }

    fn offset_indices(&mut self, offset: usize) {
        for index in self.chunks.iter_mut().flat_map(|chunk| &mut chunk.indices) {
            *index += offset;
        }
    }
}

/// Indices into the range list. The base can precede the first range
/// if that range spilled into another layer during mapping.
#[derive(Clone, Debug)]
pub(super) struct Chunk {
    pub(super) start: usize,
    pub(super) indices: Vec<usize>,
    pub(super) max_point: Option<usize>,
    // An untouched chunk keeps its identity when its base or indices change.
    id: Arc<()>,
}

impl Chunk {
    fn new(start: usize, indices: Vec<usize>, max_point: Option<usize>) -> Self {
        Self {
            start,
            indices,
            max_point,
            id: Arc::new(()),
        }
    }

    pub(super) fn last(&self) -> usize {
        *self.indices.last().expect("nonempty chunk")
    }

    pub(super) fn key(&self) -> *const () {
        Arc::as_ptr(&self.id)
    }
}

#[derive(Default)]
struct LayerBuilder {
    chunks: Vec<Chunk>,
    pending: Option<Chunk>,
    end: Option<Edge>,
}

impl LayerBuilder {
    fn can_add(&self, start: Edge) -> bool {
        self.end.is_none_or(|end| end <= start)
    }

    fn add<T>(&mut self, index: usize, entry: &StoredRange<T>, spill: &mut Vec<usize>) {
        if !self.can_add(entry.start()) {
            spill.push(index);
            return;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|chunk| chunk.indices.len() == CHUNK_SIZE)
        {
            self.flush();
        }
        let chunk = self
            .pending
            .get_or_insert_with(|| Chunk::new(entry.range.from, Vec::new(), None));
        chunk.indices.push(index);
        chunk.max_point = chunk.max_point.max(entry.point_size);
        self.end = Some(entry.end());
    }

    fn add_chunk(&mut self, chunk: Chunk, end: Edge) {
        self.flush();
        self.chunks.push(chunk);
        self.end = Some(end);
    }

    fn flush(&mut self) {
        self.chunks.extend(self.pending.take());
    }

    fn finish(mut self) -> Layer {
        self.flush();
        Layer::new(self.chunks)
    }
}

fn build_layers<'a, T: 'a>(
    indices: impl IntoIterator<Item = usize>,
    get: impl Fn(usize) -> &'a StoredRange<T>,
    layers: &mut Vec<Layer>,
) {
    let mut layer = LayerBuilder::default();
    let mut spill = Vec::new();
    for i in indices {
        layer.add(i, get(i), &mut spill);
    }
    while layer.end.is_some() {
        layers.push(layer.finish());
        layer = LayerBuilder::default();
        let mut next = Vec::new();
        for i in spill {
            layer.add(i, get(i), &mut next);
        }
        spill = next;
    }
}

impl<T: RangeValue> RangeSet<T> {
    pub(super) fn ordered(ranges: Vec<Range<T>>) -> Self {
        let ranges = store_ranges(ranges);
        let mut layers = Vec::new();
        build_layers(0..ranges.len(), |i| &ranges[i], &mut layers);
        Self { ranges, layers }
    }

    /// Compact removed slots and resolve the indices in each layer.
    fn with_layout(
        ranges: impl Iterator<Item = Option<StoredRange<T>>>,
        mut layers: Vec<Layer>,
    ) -> Self {
        let mut positions = Vec::with_capacity(ranges.size_hint().0);
        let mut kept = Vec::new();
        for range in ranges {
            positions.push(kept.len());
            kept.extend(range);
        }
        for index in layers
            .iter_mut()
            .flat_map(|layer| &mut layer.chunks)
            .flat_map(|chunk| &mut chunk.indices)
        {
            *index = positions[*index];
        }
        Self {
            ranges: kept,
            layers,
        }
    }

    pub(super) fn join_layers(sets: &[&Self]) -> Self
    where
        T: Clone,
    {
        let mut ranges = Vec::new();
        let mut layers = Vec::new();
        for (n, set) in sets.iter().enumerate() {
            let start = layers.len();
            layers.extend(set.layers.iter().cloned().map(|mut layer| {
                layer.offset_indices(ranges.len());
                layer
            }));
            // Earlier sets prepend their layers one by one.
            if n + 1 < sets.len() {
                layers[start..].reverse();
            }
            ranges.extend_from_slice(&set.ranges);
        }
        let wrapped = layers.len() - sets.last().map_or(0, |set| set.layers.len());
        let mut max_point = layers.get(wrapped).and_then(|layer| layer.max_point);
        for layer in layers[..wrapped].iter_mut().rev() {
            max_point = max_point.max(layer.max_point);
            layer.max_point = max_point;
            layer.id = Arc::new(());
        }
        Self { ranges, layers }
    }

    pub(super) fn update_layers(&self, update: RangeSetUpdate<'_, T>) -> Self
    where
        T: Clone,
    {
        let RangeSetUpdate {
            add,
            mut filter,
            filter_range,
            ..
        } = update;
        if self.is_empty() {
            return Self::ordered(add);
        }
        let touches_filter = |from, to| {
            filter_range.as_ref().is_none_or(|bounds| {
                !bounds.is_empty() && from <= *bounds.end() && to >= *bounds.start()
            })
        };
        let added = add.len();
        let has_filter = filter.is_some();
        let additions = store_ranges(add);
        let get = |i: usize| {
            if i < added {
                &additions[i]
            } else {
                &self.ranges[i - added]
            }
        };
        let mut kept = vec![true; self.len()];
        let mut layers = Vec::new();
        let mut incoming: Vec<_> = (0..added).collect();
        for old_layer in &self.layers {
            if incoming.is_empty() && !has_filter {
                let mut layer = old_layer.clone();
                layer.offset_indices(added);
                layers.push(layer);
                continue;
            }
            let mut layer = LayerBuilder::default();
            let mut spill = Vec::new();
            let mut add = incoming.into_iter().peekable();
            for chunk in &old_layer.chunks {
                for (j, &i) in chunk.indices.iter().enumerate() {
                    let entry = &self.ranges[i];
                    while add.peek().is_some_and(|&i| get(i).start() <= entry.start()) {
                        let i = add.next().unwrap();
                        layer.add(i, get(i), &mut spill);
                    }
                    let end = self.ranges[chunk.last()].end();
                    // The upstream cursor can reuse a chunk only while it
                    // is at its first range and still points into the chunk.
                    if j == 0
                        && chunk.indices.len() > 1
                        && add.peek().is_none_or(|&i| get(i).range.from > end.0)
                        && (!has_filter || !touches_filter(chunk.start, end.0))
                        && layer.can_add((chunk.start, entry.start_side))
                    {
                        let mut chunk = chunk.clone();
                        chunk.indices.iter_mut().for_each(|i| *i += added);
                        layer.add_chunk(chunk, end);
                        break;
                    }
                    if !touches_filter(entry.range.from, entry.range.to)
                        || filter.as_mut().is_none_or(|keep| keep(&entry.range))
                    {
                        layer.add(i + added, entry, &mut spill);
                    } else {
                        kept[i] = false;
                    }
                }
            }
            for i in add {
                layer.add(i, get(i), &mut spill);
            }
            if layer.end.is_some() {
                layers.push(layer.finish());
            }
            incoming = spill;
        }
        build_layers(incoming, get, &mut layers);
        let old = self
            .ranges
            .iter()
            .zip(kept)
            .map(|(entry, keep)| keep.then(|| entry.clone()));
        Self::with_layout(additions.into_iter().map(Some).chain(old), layers)
    }

    pub(super) fn map_chunks(&self, changes: &impl Mapping) -> Self
    where
        T: Clone,
    {
        let mut ranges: Vec<_> = (0..self.len()).map(|_| None).collect();
        let mut layers = Vec::new();
        for old_layer in &self.layers {
            let mut end = None;
            let mut layer = Vec::new();
            let mut spill = Vec::new();
            for chunk in &old_layer.chunks {
                let chunk_end = self.ranges[chunk.last()].range.to;
                match changes.touch(chunk.start, chunk_end) {
                    RangeTouch::Cover => continue,
                    RangeTouch::Touch => {}
                    RangeTouch::None => {
                        let start = changes.position(chunk.start, Assoc::Before, MapMode::Simple);
                        let mut indices = Vec::new();
                        for &i in &chunk.indices {
                            let entry = &self.ranges[i];
                            let range = &entry.range;
                            let from =
                                start.and_then(|base| base.checked_add(range.from - chunk.start));
                            let to =
                                start.and_then(|base| base.checked_add(range.to - chunk.start));
                            if from.is_none() && range.from == range.to {
                                continue;
                            }
                            let from = from.expect("mapped byte position exceeds usize");
                            let to = to.expect("mapped byte position exceeds usize");
                            end = Some((to, entry.end_side));
                            ranges[i] = Some(entry.at(from, to));
                            indices.push(i);
                        }
                        if !indices.is_empty() {
                            let (id, max_point) = if indices.len() == chunk.indices.len() {
                                (Arc::clone(&chunk.id), chunk.max_point)
                            } else {
                                let max_point = indices
                                    .iter()
                                    .filter_map(|&i| ranges[i].as_ref().unwrap().point_size)
                                    .max();
                                (Arc::new(()), max_point)
                            };
                            layer.push(Chunk {
                                start: start.unwrap(),
                                indices,
                                max_point,
                                id,
                            });
                        }
                        continue;
                    }
                }

                let mut start = None;
                let mut indices = Vec::new();
                let mut max_point = None;
                for &i in &chunk.indices {
                    let old_entry = &self.ranges[i];
                    let Some((from, to)) = map_bounds(&old_entry.range, changes) else {
                        continue;
                    };
                    start.get_or_insert(from);
                    let entry = old_entry.at(from, to);
                    let (from, to) = (entry.start(), entry.end());
                    max_point = max_point.max(entry.point_size);
                    ranges[i] = Some(entry);
                    if end.is_none_or(|end| end <= from) {
                        indices.push(i);
                        end = Some(to);
                    } else if from.0 == to.0
                        && let Some(at) = insertion_index(&indices, &ranges, from, to)
                    {
                        indices.insert(at, i);
                    } else {
                        spill.push(i);
                    }
                }
                if !indices.is_empty() {
                    layer.push(Chunk::new(start.unwrap(), indices, max_point));
                }
            }
            if !layer.is_empty() {
                layers.push(Layer::new(layer));
            }
            build_layers(spill, |i| ranges[i].as_ref().unwrap(), &mut layers);
        }
        Self::with_layout(ranges.into_iter(), layers)
    }
}

pub(super) fn distinct_sets<'a, T>(
    sets: &[&'a RangeSet<T>],
    other: &[&RangeSet<T>],
) -> Vec<&'a RangeSet<T>> {
    sets.iter()
        .copied()
        .filter(|set| {
            set.layers.first().is_some_and(|layer| {
                !other.iter().any(|other| {
                    other
                        .layers
                        .first()
                        .is_some_and(|other| Arc::ptr_eq(&layer.id, &other.id))
                })
            })
        })
        .collect()
}

pub(super) fn shared_chunks<T>(
    old: &[&RangeSet<T>],
    new: &[&RangeSet<T>],
    changes: Option<&ChangeDesc>,
) -> SharedChunks {
    // Upstream only looks at top layers, and never skips chunks with nonempty points.
    let chunks_a = old
        .iter()
        .filter_map(|set| set.layers.first())
        .flat_map(|layer| &layer.chunks);
    let known: HashMap<_, _> = chunks_a
        .filter(|chunk| chunk.max_point.unwrap_or(0) == 0)
        .map(|chunk| (chunk.key(), chunk.start))
        .collect();
    let mut shared = HashSet::new();
    for set in new {
        if let Some(layer) = set.layers.first() {
            for chunk in &layer.chunks {
                let key = chunk.key();
                let Some(&start) = known.get(&key) else {
                    continue;
                };
                let aligned = match changes {
                    None => start == chunk.start,
                    Some(changes) => {
                        let length = set.ranges[chunk.last()].range.to - chunk.start;
                        start.checked_add(length).is_some_and(|end| {
                            changes.map_pos(start, Assoc::Before, MapMode::Simple)
                                == Some(chunk.start)
                                && changes.touches_range(start..end) == RangeTouch::None
                        })
                    }
                };
                if aligned {
                    shared.insert(key);
                }
            }
        }
    }
    shared
}

/// Reorder a mapped empty range if it fits after an earlier range in this
/// chunk. Otherwise the range must spill into a lower layer.
fn insertion_index<T>(
    indices: &[usize],
    ranges: &[Option<StoredRange<T>>],
    from: Edge,
    to: Edge,
) -> Option<usize> {
    for (i, &index) in indices.iter().enumerate().rev() {
        let entry = ranges[index].as_ref().unwrap();
        if entry.end() <= from {
            return Some(i + 1);
        }
        if entry.start() < to {
            break;
        }
    }
    None
}

fn map_bounds<T: RangeValue>(range: &Range<T>, changes: &impl Mapping) -> Option<(usize, usize)> {
    fn assoc(side: i32) -> Assoc {
        if side < 0 {
            Assoc::Before
        } else {
            Assoc::After
        }
    }
    let Range { from, to, value } = range;
    let start_side = value.start_side();
    let end_side = value.end_side();
    let (new_from, new_to) = if from == to {
        let start = changes.position(*from, assoc(start_side), value.map_mode())?;
        let end = if start_side == end_side {
            start
        } else {
            changes.simple_position(*to, assoc(end_side))
        };
        (start, end)
    } else {
        (
            changes.simple_position(*from, assoc(start_side)),
            changes.simple_position(*to, assoc(end_side)),
        )
    };
    if (new_from, start_side) > (new_to, end_side) {
        return None;
    }
    Some((new_from, new_to))
}
