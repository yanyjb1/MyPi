//! One round: the loop on its own thread, and the outlet that streams it. The loop lands its own
//! pieces, so nothing here writes; a tool's running output goes straight to the stream.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::server::agent::r#loop::{self as agent_loop, Live, Outlet, ToolEvent, Turn};
use crate::server::api::reply::stream::StreamItem;
use crate::server::storage::{BlockKind, NewBlock, Reader, Submit, Writer};

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
    // Kept for the check at the end: is anybody still watching this session?
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
    // Nothing runs in this session any more: the state goes back when somebody watches it.
    running.release(session);
    if relay.watched(session) {
        sessions.put(session, live);
    } else {
        sessions.drop_live(session);
    }
}

/// The loop's live channel: the reply as it streams, and what a tool prints
/// while it runs.
pub(crate) struct Preview {
    session: i64,
    stop: Arc<AtomicBool>,
    writer: Writer,
    relay: Relay,
}

impl Preview {
    pub(crate) fn new(session: i64, stop: Arc<AtomicBool>, writer: Writer, relay: Relay) -> Self {
        Self {
            session,
            stop,
            writer,
            relay,
        }
    }

    /// Hand one increment to the writer: a piece of a block still being written.
    fn piece(&self, kind: BlockKind, text: &str) {
        let block = NewBlock {
            kind,
            content: Some(text.to_string()),
            ts: Some(crate::server::platform::time::now_stamp()),
            ..NewBlock::default()
        };
        let out = self.writer.push(Submit::Blocks {
            session: self.session,
            blocks: vec![block],
            done: false,
        });
        if let Err(e) = out {
            crate::server::platform::log::warn(
                "api",
                format!(
                    "session {}: a streamed piece did not reach the writer: {e:#}",
                    self.session
                ),
            );
        }
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

impl Outlet for Preview {
    fn content(&mut self, text: &str) -> bool {
        self.piece(BlockKind::Assistant, text);
        !self.stopping()
    }

    fn reasoning(&mut self, text: &str) -> bool {
        self.piece(BlockKind::Reasoning, text);
        !self.stopping()
    }

    fn tool_output(&mut self, call_id: &str, text: &str) {
        self.relay.emit(
            self.session,
            StreamItem::ToolOutput {
                call_id: call_id.to_string(),
                text: text.to_string(),
            },
        );
    }

    fn tool_event(&mut self, _event: &ToolEvent) {
        // The three moments land as blocks of their own — the request side cuts
        // them, and storage relays those at once. Nothing to preview.
    }
}
