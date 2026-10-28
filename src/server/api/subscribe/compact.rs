//! Compact: land the fixed message, open a round, and start the summarization.

use anyhow::Result;

use crate::ipc::contract::Viewer;
use crate::server::api::driver::Driver;

use super::{Opened, Retention};

/// The user message that opens a compaction round.
pub(crate) const COMPACT_MESSAGE: &str = "/compact";

pub(crate) fn apply(
    driver: &mut Driver,
    viewer: &Viewer,
    session: i64,
    retention: Retention,
    focus: Option<String>,
) -> Result<Opened> {
    driver.compact(viewer, session, retention, focus)
}
