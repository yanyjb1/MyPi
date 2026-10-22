//! Row arbitration, the row broadcast, and the cursor's terminal position.

use super::MainZone;
use super::geometry;
use crate::ui::tui::zone::{Seat, SubZone, TermSize};

/// Blank row between history and the block below it.
pub const HISTORY_GAP: u16 = 1;

/// One arbitration result.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Heights {
    pub history: u16,
    pub input: u16,
    pub reserved: u16,
    pub gap: u16,
}

/// Divide the terminal: input asks, reserved freezes, history takes the rest.
pub fn arbitrate(zone: &mut MainZone) -> Heights {
    let Some(size) = zone.size else {
        return Heights::default();
    };
    let input_rows = zone.input.request_height(size).min(size.rows);
    zone.input.assign(size, input_rows);
    let reserved_rows = zone.reserved.request_height(size).min(size.rows);
    zone.reserved.assign(size, reserved_rows);
    let rest = size
        .rows
        .saturating_sub(input_rows)
        .saturating_sub(reserved_rows);
    let gap = if rest > HISTORY_GAP { HISTORY_GAP } else { 0 };
    let history_rows = rest.saturating_sub(gap);
    zone.history.assign(size, history_rows);
    let h = Heights {
        history: history_rows,
        input: input_rows,
        reserved: reserved_rows,
        gap,
    };
    zone.heights = h;
    h
}

/// Terminal coordinates of the input cursor, clamped to the input container.
pub fn cursor_position(zone: &MainZone) -> Option<(u16, u16)> {
    let size = zone.size?;
    if zone.heights.input == 0 {
        return None;
    }
    Some(geometry::cursor_position(
        (size.cols, size.rows),
        zone.heights.history + zone.heights.gap,
        zone.heights.input,
        zone.heights.reserved,
        zone.input.cursor_row,
        zone.input.cursor_col,
    ))
}

/// Settle the sub-zones and re-arbitrate; broadcast only on change.
pub fn settle(zone: &mut MainZone) {
    if zone.size.is_none() {
        return;
    }
    zone.reserved.settle();
    let before = zone.heights;
    let after = arbitrate(zone);
    if after != before {
        broadcast(zone);
    }
}

/// The row broadcast: the arbitration result, to every subscriber.
pub fn broadcast(zone: &mut MainZone) {
    let h = zone.heights;
    let _ = zone.history.on_height_changed(h.history);
    let _ = zone.input.on_height_changed(h.input);
    let _ = zone.reserved.on_height_changed(h.reserved);
}

/// Lend the reserved zone the keys it asked for while the input zone is hung.
pub fn lend(zone: &mut MainZone) {
    let claims = zone.reserved.borrowed();
    if claims.is_empty() {
        zone.keys.release(Seat::Reserved);
    } else {
        zone.keys.lend(claims, Seat::Reserved);
    }
}

/// The current size, for callers that need it without a delivery.
pub fn size(zone: &MainZone) -> Option<TermSize> {
    zone.size
}
