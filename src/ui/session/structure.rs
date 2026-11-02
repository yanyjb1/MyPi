//! The structural window: which rounds exist and the closed block ranges each owns.

/// One round: its id and the closed block id ranges it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: i64,
    pub ranges: Vec<[i64; 2]>,
}

impl Turn {
    pub fn new(id: i64, ranges: Vec<[i64; 2]>) -> Turn {
        Turn { id, ranges }
    }

    /// Whether one block id falls in this round.
    pub fn owns(&self, block: i64) -> bool {
        self.ranges
            .iter()
            .any(|[lo, hi]| *lo <= block && block <= *hi)
    }
}

/// The kept rounds, newest first. Handed down whole when a front end asks for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Structure {
    turns: Vec<Turn>,
}

impl Structure {
    /// Record one round: the same round replaces its old row.
    pub fn set(&mut self, turn: Turn) {
        self.turns.retain(|held| held.id != turn.id);
        self.turns.insert(0, turn);
    }

    /// Replace the whole thing; `turns` is newest first.
    pub fn replace(&mut self, turns: Vec<Turn>) {
        self.turns = turns;
    }

    /// Cut the index back to one block. The request names the spot, so the reply's `cut`
    /// is none of this layer's business: everything newer than the round goes, and the
    /// round itself keeps the blocks up to the named one.
    pub fn cut_at(&mut self, turn: i64, block: i64) {
        let Some(at) = self.turns.iter().position(|held| held.id == turn) else {
            return;
        };
        self.turns.drain(..at);
        let Some(head) = self.turns.first_mut() else {
            return;
        };
        head.ranges = kept(&head.ranges, block);
        if head.ranges.is_empty() {
            self.turns.remove(0);
        }
    }

    /// Whether any round owns this block.
    pub fn owns(&self, block: i64) -> bool {
        self.turns.iter().any(|held| held.owns(block))
    }

    /// Newest first.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

/// The kept head of a run of ranges, up to and including `upto`.
fn kept(ranges: &[[i64; 2]], upto: i64) -> Vec<[i64; 2]> {
    let mut out = Vec::new();
    for [lo, hi] in ranges {
        if *lo > upto {
            break;
        }
        out.push([*lo, (*hi).min(upto)]);
    }
    out
}
