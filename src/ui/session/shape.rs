//! The structure cache: which blocks each round manages, newest first.
//! Design: `README.md`.

use std::collections::VecDeque;

use crate::server::api::types::TurnBlock;

/// How many rounds of structure are kept. A round is a management unit, not a block count.
const SHAPE_TURNS: usize = 8;

/// One round's blocks: the id, and the hash that claims the content.
pub struct Shape {
    pub turn: i64,
    pub blocks: Vec<TurnBlock>,
}

/// The kept rounds, newest first. Blocks may be shared by two rounds, so this holds ids only and
/// never copies content.
#[derive(Default)]
pub struct Shapes {
    turns: VecDeque<Shape>,
}

impl Shapes {
    /// Record one round: the same round replaces its old row, and the oldest falls off.
    pub fn set(&mut self, turn: i64, blocks: Vec<TurnBlock>) {
        self.turns.retain(|s| s.turn != turn);
        self.turns.push_front(Shape { turn, blocks });
        self.turns.truncate(SHAPE_TURNS);
    }

    /// Newest first.
    pub fn iter(&self) -> impl Iterator<Item = &Shape> {
        self.turns.iter()
    }

    /// The round that manages this block, when a kept round names it.
    pub fn turn_of(&self, block: i64) -> Option<i64> {
        self.turns
            .iter()
            .find(|s| s.blocks.iter().any(|b| b.id == block))
            .map(|s| s.turn)
    }

    /// The hash a kept round recorded for this block.
    pub fn hash_of(&self, block: i64) -> Option<u64> {
        self.turns
            .iter()
            .find_map(|s| s.blocks.iter().find(|b| b.id == block).map(|b| b.hash))
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }
}
