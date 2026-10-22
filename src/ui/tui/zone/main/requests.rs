//! Aggregate the sub-zones' requests and Main's own queue.

use super::MainZone;
use crate::ui::tui::zone::{Request, SubZone};

pub fn take(zone: &mut MainZone) -> Vec<Request> {
    let mut out = std::mem::take(&mut zone.pending_requests);
    out.extend(zone.history.take_requests());
    out.extend(zone.input.take_requests());
    out.extend(zone.reserved.take_requests());
    out
}
