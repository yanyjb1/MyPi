//! Subscribe — the three requests that put one connection on one session's stream.

use anyhow::Result;
use std::sync::mpsc;

use crate::ipc::contract::subscribe::Retention;
use crate::ipc::contract::{Frame, Viewer};
use crate::server::api::driver;
use crate::server::config::Config;

pub(crate) mod compact;
pub(crate) mod submit;
pub(crate) mod watch;

/// Watch one session: the whole stream, not one turn. Replies with the stream channel.
pub fn watch(cfg: &Config, viewer: &Viewer, session: i64) -> Result<mpsc::Receiver<Frame>> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    watch::apply(&state, viewer, session)
}

/// Submit one user message and watch its round. `Ok(None)` = busy.
pub fn submit(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    prefix: Option<&str>,
    text: &str,
    suffix: Option<&str>,
) -> Result<(mpsc::Receiver<Frame>, Option<i64>)> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    submit::apply(&mut state, viewer, session, prefix, text, suffix)
}

/// Compact a session's context and watch its round. `Ok(None)` = busy.
pub fn compact(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    retention: Retention,
    focus: Option<String>,
) -> Result<(mpsc::Receiver<Frame>, Option<i64>)> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    compact::apply(&mut state, viewer, session, retention, focus)
}
