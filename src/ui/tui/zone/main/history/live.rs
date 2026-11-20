//! What is still running: the pin above the editor, and the rows it prints.

use std::time::Instant;

use super::local::section::Idx;
use super::product::{compact, esc_row, pin_rows};
use super::row::Row;
use super::zone::HistoryZone;
use crate::ui::tui::text::glyphs as g;

/// Every block id these ranges name, in order.
fn blocks_of(ranges: &[[i64; 2]]) -> Vec<i64> {
    ranges.iter().flat_map(|[lo, hi]| *lo..=*hi).collect()
}

/// One running label: a call or a thought, and when it started.
pub(super) struct Pin {
    /// What the ESC row prints after the elapsed time.
    pub label: String,
    pub since: Instant,
    /// The call this label belongs to; `None` for a thought.
    pub call: Option<String>,
    /// The compaction's own row: no call owns it, and a thought ending leaves it alone.
    pub compact: bool,
}

/// What the last context figure was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ask {
    /// The gauge as it stood before a compaction started.
    Before,
    /// The gauge the compaction left behind.
    After,
    /// A fresh attachment: what the gauge reads now is the newest compaction's aftermath.
    Attach,
}

impl HistoryZone {
    /// The rows under the last segment: the checklist, then what is still running.
    pub(super) fn pin_rows(&mut self, width: usize) -> Vec<Row> {
        let HistoryZone {
            todo_panel,
            todo,
            todo_fold,
            pin,
            tick,
            ..
        } = self;
        let checklist = todo_panel.rows(todo, *todo_fold, width).to_vec();
        let esc = pin.first().map(|running| {
            esc_row(
                g::STAR[*tick % g::STAR.len()],
                running.since.elapsed(),
                &running.label,
                running.call.is_none() || running.compact,
            )
        });
        pin_rows(checklist, esc)
    }

    /// A thought started: its label leads until the thought ends.
    pub(super) fn thinking_started(&mut self) {
        if self.pin.iter().any(|pin| pin.call.is_none()) {
            return;
        }
        self.pin.push(Pin {
            label: "thinking".to_string(),
            since: Instant::now(),
            call: None,
            compact: false,
        });
    }

    /// The thought is over; the compaction's own row is not a thought and stays.
    pub(super) fn thinking_ended(&mut self) {
        self.pin.retain(|pin| pin.call.is_some() || pin.compact);
    }

    /// The server took a `/compact`: its row leads the pin, and its summary draws as one.
    pub(super) fn compact_started(&mut self) {
        if self.compacting.is_some() {
            return;
        }
        let now = Instant::now();
        self.compacting = Some(now);
        self.before = None;
        self.after = None;
        self.awaiting.clear();
        self.pin.retain(|pin| !pin.compact);
        self.pin.push(Pin {
            label: compact::LABEL.to_string(),
            since: now,
            call: None,
            compact: true,
        });
        // The summary has not arrived yet: the round needs a position for its banners.
        let at = self.local.running_mut().text("");
        self.local.set_compacting(true);
        self.mark(Idx::Turn(at as u32));
        self.ask = Some(Ask::Before);
        self.bill();
    }

    /// The compaction round is over. Its banner keeps the figures this process measured; a round
    /// this front end never watched start keeps only what the snapshot says.
    pub(super) fn compact_closed(&mut self, compact: bool, ranges: &[[i64; 2]]) {
        let opened = self.compacting.take();
        self.local.set_compacting(false);
        if !compact {
            return;
        }
        let blocks = blocks_of(ranges);
        let mark = compact::Mark {
            opened: opened.is_some(),
            before: self.before.take(),
            after: self.after,
        };
        self.local.mark_compact(&blocks, mark);
        self.local.drop_products();
        self.run.clear();
        if opened.is_some() {
            self.awaiting = blocks;
            self.ask = Some(Ask::After);
            self.bill();
        }
    }

    /// A snapshot names the compaction rounds it holds: give their blocks the banner.
    pub(super) fn stamp_structure(&mut self) {
        let blocks = self.structure.compact_blocks();
        if blocks.is_empty() {
            return;
        }
        self.local.mark_compact(
            &blocks,
            compact::Mark {
                opened: false,
                before: None,
                after: self.after,
            },
        );
        self.local.set_after(&blocks, self.after);
    }

    /// One call started; a running thought ends with it.
    pub(super) fn call_started(&mut self, call: &str, intent: Option<&str>) {
        self.thinking_ended();
        self.pin.retain(|pin| pin.call.as_deref() != Some(call));
        self.pin.push(Pin {
            label: intent.unwrap_or_default().to_string(),
            since: Instant::now(),
            call: Some(call.to_string()),
            compact: false,
        });
    }

    /// One call ended: the next unfinished one leads.
    pub(super) fn call_ended(&mut self, call: &str) {
        self.pin.retain(|pin| pin.call.as_deref() != Some(call));
    }
}
