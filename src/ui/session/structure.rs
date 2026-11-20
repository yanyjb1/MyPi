// The structural window: the rounds, their closed block ranges, and the wire shape they arrive in.

use serde_json::Value;

// One round: its id, the closed block id ranges it owns, and whether it is a compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: i64,
    pub ranges: Vec<[i64; 2]>,
    pub compact: bool,
}

impl Turn {
    pub fn new(id: i64, ranges: Vec<[i64; 2]>, compact: bool) -> Turn {
        Turn {
            id,
            ranges,
            compact,
        }
    }

    // Whether one block id falls in this round.
    pub fn owns(&self, block: i64) -> bool {
        self.ranges
            .iter()
            .any(|[lo, hi]| *lo <= block && block <= *hi)
    }
}

// The kept rounds, newest first. Handed down whole when a front end asks for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Structure {
    turns: Vec<Turn>,
}

impl Structure {
    // Record one round: the same round replaces its old row.
    pub fn set(&mut self, turn: Turn) {
        self.turns.retain(|held| held.id != turn.id);
        self.turns.insert(0, turn);
    }

    // Replace the whole thing; `turns` is newest first.
    pub fn replace(&mut self, turns: Vec<Turn>) {
        self.turns = turns;
    }

    // Cut the index back to one block. The request names the spot: everything newer than the round goes, and the round itself keeps the blocks up to the named one.
    pub fn cut_at(&mut self, turn: i64, block: i64) {
        let at = self
            .turns
            .iter()
            .position(|held| held.id == turn)
            .expect("rewind names a round the window holds");
        self.turns.drain(..at);
        let head = self
            .turns
            .first_mut()
            .expect("the named round is still here");
        head.ranges = kept(&head.ranges, block);
        if head.ranges.is_empty() {
            self.turns.remove(0);
        }
    }

    // Whether any round owns this block.
    pub fn owns(&self, block: i64) -> bool {
        self.turns.iter().any(|held| held.owns(block))
    }

    // Newest first.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

// The kept head of a run of ranges, up to and including `upto`.
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

// A frame's ranges: `[[lo, hi], …]`.
pub(super) fn ranges_of(value: &Value) -> Vec<[i64; 2]> {
    value
        .as_array()
        .expect("a range list is an array")
        .iter()
        .map(|item| {
            let pair = item.as_array().expect("a range is a pair");
            assert_eq!(pair.len(), 2, "a range is a pair");
            [
                pair[0].as_i64().expect("a range bound is an integer"),
                pair[1].as_i64().expect("a range bound is an integer"),
            ]
        })
        .collect()
}

// An `init` answer's `structure`: one `{turn, ranges, compact}` per round.
pub(super) fn turns_of(value: &Value) -> Vec<Turn> {
    let mut out: Vec<Turn> = value
        .as_array()
        .expect("a structure is an array")
        .iter()
        .map(|entry| {
            Turn::new(
                entry
                    .get("turn")
                    .and_then(Value::as_i64)
                    .expect("a round names its id"),
                ranges_of(entry.get("ranges").expect("a round carries its ranges")),
                entry
                    .get("compact")
                    .and_then(Value::as_bool)
                    .expect("a round marks its compaction"),
            )
        })
        .collect();
    out.reverse();
    out
}
