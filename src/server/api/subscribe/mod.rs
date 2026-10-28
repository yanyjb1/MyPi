//! Subscribe — a viewer's stream, and the three things it can ask for on it.

use anyhow::Result;
use std::sync::mpsc::Receiver;

use crate::ipc::contract::Viewer;
use crate::server::api::driver;
use crate::server::config::Config;

pub(crate) mod compact;
pub mod event;
mod submit;
mod watch;

pub use event::{ContentEvent, Event, SessionEvent, StartKind};

pub use crate::ipc::contract::{Opened, Retention, Subscribe};

/// Register this viewer's one stream.
pub fn connect(cfg: &Config, viewer: &Viewer) -> Result<Receiver<Event>> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    Ok(state.connect(viewer))
}

/// Watch one session: the whole stream, not one turn.
pub fn watch(cfg: &Config, viewer: &Viewer, session: i64) -> Result<()> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    watch::apply(&state, viewer, session)
}

/// Submit one user message and watch its round.
pub fn submit(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    prefix: Option<&str>,
    text: &str,
    suffix: Option<&str>,
) -> Result<Opened> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    submit::apply(&mut state, viewer, session, prefix, text, suffix)
}

/// Compact a session's context and watch its round.
pub fn compact(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    retention: Retention,
    focus: Option<String>,
) -> Result<Opened> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    compact::apply(&mut state, viewer, session, retention, focus)
}
