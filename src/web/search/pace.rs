// Spacing between the requests a search makes, so the rhythm does not read as a script.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::platform::stop::Stop;

// Between two redirect lookups: one per hit that carries an engine redirect.
const RESOLVE_MIN_MS: u64 = 300;
const RESOLVE_MAX_MS: u64 = 800;

// Between two result pages.
const PAGE_MIN_MS: u64 = 1_500;
const PAGE_MAX_MS: u64 = 3_000;

// The pause before one redirect lookup; `false` means a stop woke it, and the lookup is not made.
pub(crate) fn between_lookups(stop: &Arc<Stop>) -> bool {
    stop.pause(span(RESOLVE_MIN_MS, RESOLVE_MAX_MS))
}

// The pause before the next page; `false` means a stop woke it, and the page is not fetched.
pub(crate) fn between_pages(stop: &Arc<Stop>) -> bool {
    stop.pause(span(PAGE_MIN_MS, PAGE_MAX_MS))
}

fn span(min_ms: u64, max_ms: u64) -> Duration {
    let room = max_ms.saturating_sub(min_ms);
    if room == 0 {
        return Duration::from_millis(min_ms);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as u64)
        .unwrap_or(0);
    let draw = (nanos.wrapping_mul(6_364_136_223_846_793_005) >> 33) % (room + 1);
    Duration::from_millis(min_ms + draw)
}
