//! Tree's raw block range: independent of the content window. Design: `README.md`.

use crate::server::storage::Block;

/// The blocks a tree asked for, in block-id order.
#[derive(Default)]
pub struct RangeCache {
    start: i64,
    end: i64,
    blocks: Vec<Block>,
}

impl RangeCache {
    pub fn clear(&mut self) {
        self.start = 0;
        self.end = 0;
        self.blocks.clear();
    }

    pub fn replace(&mut self, start: i64, end: i64, blocks: Vec<Block>) {
        self.start = start;
        self.end = end;
        self.blocks = blocks;
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn start(&self) -> i64 {
        self.start
    }

    pub fn end(&self) -> i64 {
        self.end
    }
}
