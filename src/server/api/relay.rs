//! The session stream: one buffer per session, one queue per connection.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::server::storage::decode::parse_blocks;
use crate::server::storage::{Reader, Relay as Piece, Writer};

use super::convert;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Writer,
    Watcher,
}

struct Entry {
    viewer: Option<String>,
    session: Option<i64>,
    role: Role,
    tx: Sender<Frame>,
}

#[derive(Default)]
struct State {
    conns: HashMap<u64, Entry>,
    buffers: HashMap<i64, Vec<Frame>>,
    running: HashSet<i64>,
}

/// The stream: who is on which session, what each session still holds.
#[derive(Clone)]
pub(crate) struct Relay {
    state: Arc<Mutex<State>>,
    db: Arc<PathBuf>,
}

impl Relay {
    /// Take the writer's port; every piece becomes a stream event. The returned sender is the
    /// loop's way in: live signals that never become blocks.
    pub(crate) fn start(
        db: PathBuf,
        writer: &Writer,
    ) -> Result<(Relay, Sender<crate::server::agent::r#loop::outlet::Event>)> {
        let relay = Relay {
            state: Arc::new(Mutex::new(State::default())),
            db: Arc::new(db),
        };
        let (port, pieces) = mpsc::channel::<Piece>();
        writer
            .relay(port)
            .context("cannot register the api's relay port")?;
        let me = relay.clone();
        std::thread::Builder::new()
            .name("api-relay".to_string())
            .spawn(move || {
                for piece in pieces {
                    me.piece(piece);
                }
            })
            .context("cannot start the relay thread")?;

        let (events, live) = mpsc::channel();
        let me = relay.clone();
        std::thread::Builder::new()
            .name("api-live".to_string())
            .spawn(move || {
                for event in live {
                    me.event(event);
                }
            })
            .context("cannot start the live thread")?;
        Ok((relay, events))
    }

    pub(crate) fn attach(&self, conn: u64, tx: Sender<Frame>) {
        self.state().conns.insert(
            conn,
            Entry {
                viewer: None,
                session: None,
                role: Role::Watcher,
                tx,
            },
        );
    }

    pub(crate) fn hello(&self, conn: u64, viewer: String) {
        if let Some(entry) = self.state().conns.get_mut(&conn) {
            entry.viewer = Some(viewer);
        }
    }

    pub(crate) fn detach(&self, conn: u64) {
        let mut state = self.state();
        let Some(entry) = state.conns.remove(&conn) else {
            return;
        };
        if let Some(session) = entry.session {
            drop_buffer(&mut state, session);
        }
    }

    pub(crate) fn viewer_of(&self, conn: u64) -> Option<String> {
        self.state()
            .conns
            .get(&conn)
            .and_then(|entry| entry.viewer.clone())
    }

    pub(crate) fn live(&self) -> usize {
        self.state().conns.len()
    }

    /// Declare this connection's session and role. A new writer takes the session.
    pub(crate) fn declare(&self, conn: u64, session: i64, role: Role, turn: Option<i64>) {
        let mut state = self.state();
        let old = state.conns.get(&conn).and_then(|entry| entry.session);
        if role == Role::Writer {
            for entry in state.conns.values_mut() {
                if entry.session == Some(session) && entry.role == Role::Writer {
                    entry.role = Role::Watcher;
                }
            }
        }
        if let Some(entry) = state.conns.get_mut(&conn) {
            entry.session = Some(session);
            entry.role = role;
        }
        if let Some(old) = old
            && old != session
        {
            drop_buffer(&mut state, old);
        }
    }

    /// Claim the session's running slot; `false` = a turn is already running.
    pub(crate) fn claim(&self, session: i64) -> bool {
        self.state().running.insert(session)
    }

    pub(crate) fn release(&self, session: i64) {
        self.state().running.remove(&session);
    }

    pub(crate) fn running(&self, session: i64) -> bool {
        self.state().running.contains(&session)
    }

    pub(crate) fn writer_of(&self, session: i64) -> Option<String> {
        self.state()
            .conns
            .values()
            .find(|entry| entry.session == Some(session) && entry.role == Role::Writer)
            .and_then(|entry| entry.viewer.clone())
    }

    /// One event: record it for the session, then hand it to every subscriber.
    pub(crate) fn deliver(&self, session: i64, frame: Frame, issuer: Option<&str>) {
        let mut state = self.state();
        let live: Vec<(u64, Sender<Frame>)> = state
            .conns
            .iter()
            .filter(|(_, entry)| entry.session == Some(session))
            .filter(|(_, entry)| match (issuer, entry.viewer.as_deref()) {
                (Some(who), Some(viewer)) => who != viewer,
                _ => true,
            })
            .map(|(id, entry)| (*id, entry.tx.clone()))
            .collect();
        if live.is_empty() {
            return;
        }
        state
            .buffers
            .entry(session)
            .or_default()
            .push(frame.clone());
        let mut dead = Vec::new();
        for (id, tx) in live {
            if tx.send(frame.clone()).is_err() {
                dead.push(id);
            }
        }
        for id in dead {
            state.conns.remove(&id);
        }
        drop_buffer(&mut state, session);
    }

    /// A post event: the issuer never reads its own write back.
    pub(crate) fn post(&self, session: i64, action: &str, issuer: &str, extra: Value) {
        let mut payload = json!({"session": session, "action": action});
        if let (Some(object), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        self.deliver(session, frame("post", payload), Some(issuer));
    }

    /// The session is gone: its buffer, its slot, and every watcher's claim on it.
    pub(crate) fn forget(&self, session: i64) {
        let mut state = self.state();
        state.buffers.remove(&session);
        state.running.remove(&session);
        for entry in state.conns.values_mut() {
            if entry.session == Some(session) {
                entry.session = None;
                entry.role = Role::Watcher;
            }
        }
    }

    /// One live line from the loop.
    fn event(&self, event: crate::server::agent::r#loop::outlet::Event) {
        let payload = json!({
            "session": event.session,
            "level": event.level,
            "content": event.content,
        });
        self.deliver(event.session, frame("notice", payload), None);
    }

    fn piece(&self, piece: Piece) {
        match piece {
            Piece::Tail {
                session,
                turn,
                blocks,
                done,
            } => {
                let payload = json!({
                    "session": session,
                    "turn": turn,
                    "blocks": blocks.into_iter().map(convert::new_block).collect::<Vec<_>>(),
                    "done": done,
                });
                self.deliver(session, frame("stream", payload), None);
            }
            Piece::Blocks {
                session,
                turn,
                blocks,
            } => {
                let payload = json!({
                    "session": session,
                    "turn": turn,
                    "blocks": blocks.into_iter().map(convert::block).collect::<Vec<_>>(),
                });
                self.deliver(session, frame("stream", payload), None);
            }
            Piece::Tool {
                session,
                turn,
                call_id,
                text,
            } => {
                let payload = json!({
                    "session": session,
                    "turn": turn,
                    "call_id": call_id,
                    "text": text,
                });
                self.deliver(session, frame("stream", payload), None);
            }
            Piece::Ended {
                session,
                turn,
                stop_reason,
                usage,
            } => {
                let ranges = self.ranges(turn);
                let payload = json!({
                    "session": session,
                    "turn": turn,
                    "ranges": ranges,
                    "stop_reason": stop_reason,
                    "usage": usage.map(convert::usage),
                });
                self.deliver(session, frame("structure", payload), None);
                let mut state = self.state();
                state.running.remove(&session);
                state.buffers.remove(&session);
            }
        }
    }

    /// The block id ranges one round owns.
    fn ranges(&self, turn: i64) -> Vec<[i64; 2]> {
        let Ok(conn) = Reader::open(&self.db) else {
            return Vec::new();
        };
        let raw: Option<String> = conn
            .query_row("SELECT blocks FROM turns WHERE turn_id = ?1", [turn], |r| {
                r.get(0)
            })
            .ok();
        Reader::close(conn);
        raw.as_deref()
            .and_then(|raw| parse_blocks(raw).ok())
            .unwrap_or_default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn frame(name: &str, payload: Value) -> Frame {
    Frame::new("subscribe", name, payload)
}

/// A session with no subscriber left keeps no buffer.
fn drop_buffer(state: &mut State, session: i64) {
    let watched = state
        .conns
        .values()
        .any(|entry| entry.session == Some(session));
    if !watched {
        state.buffers.remove(&session);
    }
}
