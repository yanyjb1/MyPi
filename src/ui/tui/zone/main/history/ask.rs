//! Blocks on demand: one turn at a time, and the anchor's neighbours.

use serde_json::json;

use super::local::SlotKind;
use super::zone::HistoryZone;

use crate::ui::tui::zone::Request;

impl HistoryZone {
    /// Ask for one turn's blocks, unless they are here or already asked for.
    pub(super) fn ask_turn(&mut self, turn: i64) {
        let Some(held) = self.structure.rounds().iter().find(|t| t.id == turn) else {
            return;
        };
        for range in &held.ranges {
            if self.asking.contains(range) {
                continue;
            }
            let [lo, hi] = *range;
            let have = (lo..=hi).all(|id| self.blocks.contains_key(&id));
            if have {
                continue;
            }
            self.asking.insert(*range);
            self.outbox
                .push(Request::call("blocks", json!({"ranges": [[lo, hi]]})));
        }
    }

    /// The turn a block belongs to.
    pub(super) fn turn_of(&self, block: i64) -> Option<i64> {
        self.structure
            .rounds()
            .iter()
            .find(|turn| turn.owns(block))
            .map(|turn| turn.id)
    }

    /// Keep the anchor's own turn and its neighbours in hand.
    pub(super) fn ensure_around(&mut self) {
        let anchor = self.view.anchor(&self.local);
        let Some(block) = self.local.at(anchor).and_then(|slot| match slot.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let wanted: Vec<i64> = {
            let turns = self.structure.rounds();
            let Some(at) = turns.iter().position(|turn| turn.owns(block)) else {
                return;
            };
            [-1i64, 0, 1]
                .iter()
                .filter_map(|offset| {
                    let index = at as i64 + offset;
                    (index >= 0 && (index as usize) < turns.len()).then(|| turns[index as usize].id)
                })
                .collect()
        };
        for turn in wanted {
            self.ask_turn(turn);
        }
    }
}
