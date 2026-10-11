//! Subscribe — a claim on a session's stream, or a request that opens a stretch (a round, a
//! compaction). Content arrives on the stream the viewer holds.

use anyhow::Result;
use std::sync::mpsc::Receiver;

use crate::server::api::driver;
use crate::server::api::reply::stream::StreamItem;
use crate::server::api::types::Viewer;
use crate::server::config::Config;

pub(crate) mod compaction;
pub(crate) mod submit;

use compaction::CompactOp;
pub use compaction::Retention;
use submit::SubmitOp;

/// What opening a stretch answers with. The content is not here: it arrives on the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Opened {
    /// The stretch is on its way. `turn` = the round it opened (`None` = the round number is not
    /// known yet and arrives with `StreamItem::Ended`; a compaction is the case).
    Started { turn: Option<i64> },
    /// Nothing started: that session is already mid-round, or there was
    /// nothing to compact. Try again when the round ends.
    Busy,
}

/// Watch one session: the stream this viewer keeps for it. Nothing has to be running; watching twice
/// as the same viewer replaces its old stream.
pub fn watch(cfg: &Config, viewer: Viewer, session: i64) -> Result<Receiver<StreamItem>> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    Ok(state.watch(viewer, session))
}

/// Open a round: one user message, already split into prefix / body / suffix. `Opened::Busy` means
/// try again once the round ends.
pub fn submit(
    cfg: &Config,
    session: i64,
    prefix: Option<&str>,
    text: &str,
    suffix: Option<&str>,
) -> Result<Opened> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.submit(session, prefix, text, suffix)
}

/// Open a compaction of one session's context (`/compact`).
pub fn compact(
    cfg: &Config,
    session: i64,
    retention: Retention,
    focus: Option<String>,
) -> Result<Opened> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.compact(session, retention, focus)
}
