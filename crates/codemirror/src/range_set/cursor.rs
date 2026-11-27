//! Layer and heap cursors based on CodeMirror state 6.7.6.
//! See `docs/licenses/codemirror-state.txt` for the upstream MIT notice.

use std::cmp::Ordering;

use super::chunks::{Edge, Layer, SharedChunks};
use super::{RangeSet, StoredRange};

pub(super) type RankedRange<'a, T> = (usize, &'a StoredRange<T>);

struct LayerCursor<'a, 's, T> {
    set: &'a RangeSet<T>,
    layer: &'a Layer,
    shared: Option<&'s SharedChunks>,
    min_point: Option<usize>,
    rank: usize,
    chunk: usize,
    index: usize,
    current: Option<&'a StoredRange<T>>,
}

impl<'a, T> LayerCursor<'a, '_, T> {
    fn goto(&mut self, pos: usize) {
        self.chunk = 0;
        self.index = 0;
        self.seek(pos, None, false);
    }

    fn forward(&mut self, (pos, side): Edge) {
        if self.current.is_some_and(|entry| entry.end() < (pos, side)) {
            self.seek(pos, Some(side), true);
        }
    }

    fn seek(&mut self, pos: usize, side: Option<i32>, mut forward: bool) {
        while let Some(chunk) = self.layer.chunks.get(self.chunk) {
            if !self.is_shared(self.chunk)
                && self.set.ranges[chunk.last()].range.to >= pos
                && chunk.max_point >= self.min_point
            {
                break;
            }
            self.chunk += 1;
            forward = false;
        }
        if let Some(chunk) = self.layer.chunks.get(self.chunk) {
            let index = chunk.indices.partition_point(|&i| {
                let (end, end_side) = self.set.ranges[i].end();
                end < pos || end == pos && side.is_some_and(|side| end_side < side)
            });
            if !forward || self.index < index {
                self.set_index(index);
            }
        }
        self.next();
    }

    fn next(&mut self) {
        while let Some(chunk) = self.layer.chunks.get(self.chunk) {
            let entry = &self.set.ranges[chunk.indices[self.index]];
            self.set_index(self.index + 1);
            if entry.point_size >= self.min_point {
                self.current = Some(entry);
                return;
            }
        }
        self.current = None;
    }

    fn set_index(&mut self, index: usize) {
        if index == self.layer.chunks[self.chunk].indices.len() {
            self.chunk += 1;
            while self.is_shared(self.chunk) {
                self.chunk += 1;
            }
            self.index = 0;
        } else {
            self.index = index;
        }
    }

    fn is_shared(&self, index: usize) -> bool {
        self.shared.is_some_and(|shared| {
            self.layer
                .chunks
                .get(index)
                .is_some_and(|chunk| shared.contains(&chunk.key()))
        })
    }

    fn key(&self) -> Option<(Edge, usize, Edge)> {
        self.current
            .map(|entry| (entry.start(), self.rank, entry.end()))
    }

    fn compare(&self, other: &Self) -> Ordering {
        match (self.key(), other.key()) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => self.rank.cmp(&other.rank),
        }
    }
}

pub(super) struct Cursor<'a, 's, T> {
    heap: Vec<LayerCursor<'a, 's, T>>,
    current: Option<RankedRange<'a, T>>,
}

impl<'a, 's, T: 'a> Cursor<'a, 's, T> {
    pub(super) fn new(
        sets: &[&'a RangeSet<T>],
        shared: Option<&'s SharedChunks>,
        min_point: Option<usize>,
    ) -> Self {
        let mut heap = Vec::new();
        for (rank, &set) in sets.iter().enumerate() {
            for layer in &set.layers {
                if layer.max_point >= min_point {
                    heap.push(LayerCursor {
                        set,
                        layer,
                        shared,
                        min_point,
                        rank,
                        chunk: 0,
                        index: 0,
                        current: None,
                    });
                }
            }
        }
        Self {
            heap,
            current: None,
        }
    }

    pub(super) fn goto(&mut self, pos: usize) {
        for layer in &mut self.heap {
            layer.goto(pos);
        }
        if self.heap.len() != 1 {
            self.heapify();
            self.next();
        }
    }

    pub(super) fn forward(&mut self, end: Edge) {
        for layer in &mut self.heap {
            layer.forward(end);
        }
        if self.heap.len() != 1 {
            self.heapify();
            if self.current.is_some_and(|(_, entry)| entry.end() < end) {
                self.next();
            }
        }
    }

    pub(super) fn current(&self) -> Option<RankedRange<'a, T>> {
        match self.heap.as_slice() {
            [layer] => layer.current.map(|entry| (layer.rank, entry)),
            _ => self.current,
        }
    }

    pub(super) fn next(&mut self) {
        match self.heap.as_mut_slice() {
            [] => {}
            [layer] => layer.next(),
            [layer, ..] => {
                self.current = layer.current.map(|entry| (layer.rank, entry));
                if self.current.is_some() {
                    layer.next();
                }
                self.bubble(0);
            }
        }
    }

    pub(super) fn iter(mut self, from: usize) -> impl Iterator<Item = RankedRange<'a, T>> {
        self.goto(from);
        std::iter::from_fn(move || {
            let entry = self.current()?;
            self.next();
            Some(entry)
        })
    }

    fn heapify(&mut self) {
        for index in (0..=self.heap.len() / 2).rev() {
            self.bubble(index);
        }
    }

    // Keep exhausted layers in the heap. On ties, use the right child and
    // move the parent down. These choices affect the order of tied points.
    fn bubble(&mut self, mut parent: usize) {
        while let Some(left) = parent
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .filter(|&n| n < self.heap.len())
        {
            let right = left + 1;
            let child =
                if right < self.heap.len() && !self.heap[left].compare(&self.heap[right]).is_lt() {
                    right
                } else {
                    left
                };
            if self.heap[parent].compare(&self.heap[child]).is_lt() {
                break;
            }
            self.heap.swap(parent, child);
            parent = child;
        }
    }
}
