//! One round: the loop on its own thread, and the outlet that streams it.

use anyhow::Result;
use std::path::PathBuf;

use crate::server::agent::r#loop::outlet::{Outlet, ToolEvent};
use crate::server::agent::r#loop::{self as agent_loop, Live, Turn};
use crate::server::agent::request::retry::RetryNotice;
use crate::server::agent::tools::jobs::Wake;
use crate::server::storage::{Reader, Writer};

use super::relay::Relay;
use super::running::Running;
use super::sessions::Sessions;

/// Run one round to its end, then give the session back.
pub(crate) fn run(
    writer: Writer,
    db: PathBuf,
    sessions: Sessions,
    running: Running,
    session: i64,
    turn: i64,
    mut live: Live,
    mut preview: Preview,
) {
    let relay = preview.relay.clone();
    let result = (|| -> Result<()> {
        let conn = Reader::open(&db)?;
        let ended = agent_loop::run(
            &writer,
            &conn,
            &mut live,
            Turn {
                session,
                turn_id: turn,
            },
            &mut preview,
        );
        Reader::close(conn);
        ended.map(|_| ())
    })();
    if let Err(e) = result {
        crate::server::platform::log::error(
            "api",
            format!("session {session}: the round died: {e:#}"),
        );
    }
    let queue = live.wakes.clone();
    let sink = live.wake_sink.clone();
    if relay.watched(session) {
        sessions.put(session, live);
        running.release(session);
        let pending = {
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            if q.is_empty() {
                None
            } else {
                Some(q.drain(..).collect::<Vec<_>>().join("\n"))
            }
        };
        if let (Some(text), Some(sink)) = (pending, sink) {
            sink.wake(Wake { session, text });
        }
    } else {
        running.release(session);
        sessions.drop_live(session);
    }
}

/// The loop's live channel: the reply as it streams, and what a tool prints.
pub(crate) struct Preview {
    session: i64,
    turn: i64,
    relay: Relay,
}

impl Preview {
    pub(crate) fn new(session: i64, turn: i64, relay: Relay) -> Self {
        Self {
            session,
            turn,
            relay,
        }
    }
}

impl Outlet for Preview {
    fn tool_event(&mut self, _event: &ToolEvent) {}

    fn tool_output(&mut self, call_id: &str, text: &str) {
        self.relay
            .tool_output(self.session, self.turn, call_id, text);
    }

    fn retry(&mut self, notice: &RetryNotice) {
        crate::server::platform::log::warn(
            "api",
            format!("session {}: {}", self.session, notice.reason),
        );
    }

    fn budget_exhausted(&mut self) {}
}
