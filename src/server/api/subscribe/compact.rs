//! Compact: land the fixed message, open a round, and start the summarization.

use anyhow::Result;
use std::sync::mpsc;

use crate::ipc::contract::subscribe::Retention;
use crate::ipc::contract::{Frame, Viewer};
use crate::server::api::driver::Driver;

/// The user message that opens a compaction round.
pub(crate) const COMPACT_MESSAGE: &str = "/compact";

pub(crate) fn apply(
    driver: &mut Driver,
    viewer: &Viewer,
    session: i64,
    retention: Retention,
    focus: Option<String>,
) -> Result<(mpsc::Receiver<Frame>, Option<i64>)> {
    let (tx, rx) = mpsc::channel();
    let turn = driver.compact(viewer, session, retention, focus, tx)?;
    Ok((rx, turn))
}
