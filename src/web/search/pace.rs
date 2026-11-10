//! Spacing between the requests a search makes, so the rhythm does not read as a script.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Between two redirect lookups: one per hit that carries an engine redirect.
const RESOLVE_MIN_MS: u64 = 300;
const RESOLVE_MAX_MS: u64 = 800;

/// Between two result pages.
const PAGE_MIN_MS: u64 = 1_500;
const PAGE_MAX_MS: u64 = 3_000;

pub(crate) fn between_lookups() {
    pause(RESOLVE_MIN_MS, RESOLVE_MAX_MS);
}

pub(crate) fn between_pages() {
    pause(PAGE_MIN_MS, PAGE_MAX_MS);
}

fn pause(min_ms: u64, max_ms: u64) {
    let span = max_ms.saturating_sub(min_ms);
    if span == 0 {
        std::thread::sleep(Duration::from_millis(min_ms));
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as u64)
        .unwrap_or(0);
    let draw = (nanos.wrapping_mul(6_364_136_223_846_793_005) >> 33) % (span + 1);
    std::thread::sleep(Duration::from_millis(min_ms + draw));
}
