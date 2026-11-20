// The structural window: which blocks each round holds.

use std::collections::BTreeMap;

use serde_json::Value;

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

    pub fn owns(&self, block: i64) -> bool {
        self.ranges
            .iter()
            .any(|[lo, hi]| *lo <= block && block <= *hi)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Structure {
    // Keyed by round id: key order is round order.
    turns: BTreeMap<i64, Turn>,
}

impl Structure {
    // A named round arrives; an unnamed single block cuts to that block, everything after it going.
    pub fn update(&mut self, turn: Option<i64>, blocks: &[[i64; 2]], compact: bool) {
        let Some(id) = turn else {
            if let [block] = blocks
                && block[0] == block[1]
            {
                self.cut_to(block[0]);
            }
            return;
        };
        self.turns
            .insert(id, Turn::new(id, blocks.to_vec(), compact));
    }

    // One `call init` answer's `structure`: `[{turn, ranges, compact}, …]`, oldest first.
    pub fn of(snapshot: &Value) -> Structure {
        let mut out = Structure::default();
        for entry in snapshot.as_array().into_iter().flatten() {
            let Some(id) = entry.get("turn").and_then(Value::as_i64) else {
                continue;
            };
            let ranges = entry.get("ranges").and_then(ranges_of).unwrap_or_default();
            let compact = entry
                .get("compact")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            out.update(Some(id), &ranges, compact);
        }
        out
    }

    pub fn owns(&self, block: i64) -> bool {
        self.turns.values().any(|turn| turn.owns(block))
    }

    // Newest first.
    pub fn turns(&self) -> impl DoubleEndedIterator<Item = &Turn> {
        self.turns.values().rev()
    }
    // Every block a compaction round owns, newest round first.
    pub fn compact_blocks(&self) -> Vec<i64> {
        let mut out = Vec::new();
        for turn in self.turns.values().rev().filter(|turn| turn.compact) {
            for [lo, hi] in &turn.ranges {
                out.extend(*lo..=*hi);
            }
        }
        out
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }

    fn cut_to(&mut self, block: i64) {
        let Some((id, compact, kept)) = self
            .turns
            .iter()
            .find(|(_, turn)| turn.owns(block))
            .map(|(id, turn)| (*id, turn.compact, head(&turn.ranges, block)))
        else {
            return;
        };
        self.turns.split_off(&id);
        if !kept.is_empty() {
            self.turns.insert(id, Turn::new(id, kept, compact));
        }
    }
}

fn head(ranges: &[[i64; 2]], upto: i64) -> Vec<[i64; 2]> {
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
pub(super) fn ranges_of(value: &Value) -> Option<Vec<[i64; 2]>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        let pair = item.as_array()?;
        if pair.len() != 2 {
            return None;
        }
        out.push([pair[0].as_i64()?, pair[1].as_i64()?]);
    }
    Some(out)
}
