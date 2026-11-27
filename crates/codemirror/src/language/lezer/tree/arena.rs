use std::{mem, ops::Index, sync::Arc};

// Node and link IDs are stable across a checkpoint. A fresh parse uses only
// the tail; a checkpoint freezes it as an immutable block. Keeping the blocks
// flat avoids recursive drops and retains no records from the replaced suffix.
#[derive(Debug)]
pub(super) struct Arena<T> {
    blocks: Vec<Block<T>>,
    tail: Vec<T>,
}

#[derive(Debug)]
struct Block<T> {
    start: usize,
    records: Arc<Vec<T>>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            tail: Vec::new(),
        }
    }
}

impl<T> Arena<T> {
    fn tail_start(&self) -> usize {
        self.blocks
            .last()
            .map_or(0, |block| block.start + block.records.len())
    }

    pub(super) fn len(&self) -> usize {
        self.tail_start() + self.tail.len()
    }

    pub(super) fn push(&mut self, record: T) {
        self.tail.push(record);
    }

    pub(super) fn freeze(&mut self) -> usize {
        if !self.tail.is_empty() {
            self.blocks.push(Block {
                start: self.tail_start(),
                records: Arc::new(mem::take(&mut self.tail)),
            });
        }
        self.blocks.len()
    }

    // Move the live tail into a read-only block. Staging can then append
    // without growing (and possibly copying) the live allocation.
    pub(super) fn isolate_tail(&mut self) {
        assert!(self.blocks.is_empty(), "GLR does not use checkpoints");
        self.freeze();
    }

    pub(super) fn restore_tail(&mut self) {
        self.tail = self.blocks.pop().map_or_else(Vec::new, |block| {
            Arc::into_inner(block.records).expect("staging borrows the live block")
        });
    }

    pub(super) fn prefix(&self, count: usize) -> Self {
        Self {
            blocks: self.blocks[..count]
                .iter()
                .map(|block| Block {
                    start: block.start,
                    records: Arc::clone(&block.records),
                })
                .collect(),
            tail: Vec::new(),
        }
    }

    pub(super) fn without_tail(self) -> Self {
        Self {
            blocks: self.blocks,
            tail: Vec::new(),
        }
    }

    pub(super) fn unshared(&mut self) -> &mut Vec<T> {
        assert!(self.blocks.is_empty(), "GLR does not use checkpoints");
        &mut self.tail
    }
}

impl<T> Index<usize> for Arena<T> {
    type Output = T;

    fn index(&self, id: usize) -> &T {
        let tail_start = self.tail_start();
        if id >= tail_start {
            &self.tail[id - tail_start]
        } else {
            let block = &self.blocks[self.blocks.partition_point(|block| block.start <= id) - 1];
            &block.records[id - block.start]
        }
    }
}
