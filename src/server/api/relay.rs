//! The session stream: one table of connections, one current turn per session, and the buffer a
//! late watcher catches up from.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::server::agent::r#loop::{Element, Event, NoticeLevel, Stream, ToolEvent};
use crate::server::agent::request::message::Usage;
use crate::server::storage::{Structure, Writer};

/// One connection's outbound end and what it currently follows.
struct Entry {
    viewer: Option<String>,
    session: Option<i64>,
    tx: Sender<Frame>,
}

/// The turn a session is in, the door that opened it, and the viewer that did.
struct Active {
    turn: i64,
    opened_by: &'static str,
    viewer: String,
}

#[derive(Default)]
struct State {
    conns: HashMap<u64, Entry>,
    active: HashMap<i64, Active>,
    /// The viewer that put a message in the slot, waiting for the turn it opens.
    pending: HashMap<i64, String>,
    buffers: HashMap<i64, Vec<Frame>>,
}

/// The stream: who follows which session, and what this turn has already said.
#[derive(Clone)]
pub(crate) struct Relay {
    state: Arc<Mutex<State>>,
}

impl Relay {
    /// Take storage's structure port, and hand back the event sender the loop writes to.
    pub(crate) fn start(writer: &Writer) -> Result<(Relay, Sender<Event>)> {
        let relay = Relay {
            state: Arc::new(Mutex::new(State::default())),
        };
        let (port, pieces) = mpsc::channel::<Structure>();
        writer
            .relay(port)
            .context("cannot register the api's relay port")?;
        let me = relay.clone();
        std::thread::Builder::new()
            .name("api-relay".to_string())
            .spawn(move || {
                for piece in pieces {
                    me.structure(piece);
                }
            })
            .context("cannot start the relay thread")?;

        let (events, live) = mpsc::channel::<Event>();
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
        state.conns.remove(&conn);
        drop_unfollowed(&mut state);
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

    /// This connection follows this session, and hears whatever this turn already said.
    pub(crate) fn declare(&self, conn: u64, session: i64) {
        let mut state = self.state();
        if let Some(entry) = state.conns.get_mut(&conn) {
            entry.session = Some(session);
        }
        let replay = state.buffers.get(&session).cloned().unwrap_or_default();
        if let Some(tx) = state.conns.get(&conn).map(|entry| entry.tx.clone()) {
            for frame in replay {
                let _ = tx.send(frame);
            }
        }
    }

    /// Is a turn already running on this session?
    pub(crate) fn busy(&self, session: i64) -> bool {
        self.state().active.contains_key(&session)
    }

    /// Record the turn this viewer just opened on this session.
    pub(crate) fn opened(
        &self,
        conn: u64,
        session: i64,
        viewer: &str,
        turn: i64,
        door: &'static str,
    ) {
        let mut state = self.state();
        if let Some(entry) = state.conns.get_mut(&conn) {
            entry.session = Some(session);
        }
        state.pending.remove(&session);
        state.active.insert(
            session,
            Active {
                turn,
                opened_by: door,
                viewer: viewer.to_string(),
            },
        );
    }

    /// Remember who wrote the slot; the turn it opens is named after the slot, nobody else.
    pub(crate) fn note_mail(&self, conn: u64, session: i64, viewer: &str) {
        let mut state = self.state();
        if let Some(entry) = state.conns.get_mut(&conn) {
            entry.session = Some(session);
        }
        state.pending.insert(session, viewer.to_string());
    }

    /// The slot was cleared before the loop took it: nothing is owed a turn.
    pub(crate) fn drop_mail(&self, session: i64) {
        self.state().pending.remove(&session);
    }

    /// Only the viewer that opened the turn may change the session while it runs.
    pub(crate) fn writes(&self, session: i64, viewer: &str) -> bool {
        match self.state().active.get(&session) {
            Some(active) => active.viewer == viewer,
            None => true,
        }
    }

    /// One change of the api's own: everybody but the issuer hears it.
    pub(crate) fn change(&self, session: i64, action: &str, issuer: &str, extra: Value) {
        let mut payload = json!({"session": session, "action": action, "by": issuer});
        if let (Some(object), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        self.deliver(
            session,
            Frame::new("stream", "change", payload),
            Some(issuer),
        );
    }

    /// One line the api has to say, into the session's stream.
    pub(crate) fn notice(&self, session: i64, level: NoticeLevel, text: impl Into<String>) {
        let level = match level {
            NoticeLevel::Info => "info",
            NoticeLevel::Warn => "warn",
            NoticeLevel::Error => "error",
        };
        let payload = json!({"session": session, "level": level, "content": text.into()});
        self.deliver(session, Frame::new("stream", "notice", payload), None);
    }

    /// The session is gone: its buffer, its turn, and every watcher's claim on it.
    pub(crate) fn forget(&self, session: i64) {
        let mut state = self.state();
        state.buffers.remove(&session);
        state.active.remove(&session);
        state.pending.remove(&session);
        for entry in state.conns.values_mut() {
            if entry.session == Some(session) {
                entry.session = None;
            }
        }
    }

    /// One live moment from the loop.
    fn event(&self, event: Event) {
        let session = event.session;
        let element = match event.element {
            Element::Mailbox { turn } => {
                self.slot_opened(session, turn);
                return;
            }
            other => other,
        };
        let (turn, door) = self.turn_of(session);
        match element {
            Element::Stream(stream) => {
                let (kind, extra) = match stream {
                    Stream::Text { text, usage } => {
                        ("text", json!({"text": text, "usage": bill(usage)}))
                    }
                    Stream::Thinking { text, usage } => {
                        ("thinking", json!({"text": text, "usage": bill(usage)}))
                    }
                    Stream::Arguments {
                        call_id,
                        name,
                        text,
                        usage,
                    } => (
                        "arguments",
                        json!({"call_id": call_id, "name": name, "text": text, "usage": bill(usage)}),
                    ),
                    Stream::Boundary => ("boundary", json!({})),
                    Stream::Wake(text) => ("wake", json!({"text": text})),
                };
                self.stream(session, turn, door, kind, extra);
            }
            Element::Tool(tool) => {
                let (kind, extra) = match tool {
                    ToolEvent::Start {
                        call_id,
                        name,
                        args,
                        intent,
                    } => (
                        "tool_start",
                        json!({"call_id": call_id, "name": name, "args": args, "intent": intent}),
                    ),
                    ToolEvent::Progress { call_id, text } => {
                        ("tool_progress", json!({"call_id": call_id, "text": text}))
                    }
                    ToolEvent::End {
                        call_id,
                        ok,
                        result,
                        memo,
                        duration_ms,
                    } => (
                        "tool_end",
                        json!({
                            "call_id": call_id,
                            "ok": ok,
                            "result": result,
                            "memo": memo,
                            "duration_ms": duration_ms,
                        }),
                    ),
                };
                self.stream(session, turn, door, kind, extra);
            }
            Element::Notice(notice) => {
                let level = match notice.level {
                    NoticeLevel::Info => "info",
                    NoticeLevel::Warn => "warn",
                    NoticeLevel::Error => "error",
                };
                let payload = json!({"session": session, "level": level, "content": notice.text});
                self.deliver(session, Frame::new("stream", "notice", payload), None);
            }
            // Handled above: the slot opening names the turn instead of pushing a frame.
            Element::Mailbox { .. } => {}
        }
    }

    /// The slot became a turn: the writer recorded at write time owns it, the door is the slot.
    fn slot_opened(&self, session: i64, turn: i64) {
        let mut state = self.state();
        let viewer = state.pending.remove(&session).unwrap_or_default();
        state.active.insert(
            session,
            Active {
                turn,
                opened_by: "mailbox",
                viewer,
            },
        );
    }

    /// One piece of live output, labelled with the turn and the door that opened it.
    fn stream(&self, session: i64, turn: i64, door: &'static str, kind: &str, extra: Value) {
        let mut payload = json!({"session": session, "turn": turn, "kind": kind});
        if let (Some(object), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        self.deliver(session, Frame::new("stream", door, payload), None);
    }

    /// A turn closed: its ranges go out last, then the buffer and the turn go.
    fn structure(&self, piece: Structure) {
        let (_, door) = self.turn_of(piece.session);
        let payload = json!({
            "session": piece.session,
            "turn": piece.turn,
            "ranges": piece.ranges,
        });
        self.deliver(piece.session, Frame::new("stream", door, payload), None);
        let mut state = self.state();
        state.buffers.remove(&piece.session);
        state.active.remove(&piece.session);
    }

    fn turn_of(&self, session: i64) -> (i64, &'static str) {
        self.state()
            .active
            .get(&session)
            .map(|active| (active.turn, active.opened_by))
            .unwrap_or((0, "wake"))
    }

    /// One frame for every connection following this session; the issuer never reads its own write.
    fn deliver(&self, session: i64, frame: Frame, issuer: Option<&str>) {
        let mut state = self.state();
        let targets: Vec<(u64, Sender<Frame>)> = state
            .conns
            .iter()
            .filter(|(_, entry)| entry.session == Some(session))
            .filter(|(_, entry)| match (issuer, entry.viewer.as_deref()) {
                (Some(who), Some(viewer)) => who != viewer,
                _ => true,
            })
            .map(|(id, entry)| (*id, entry.tx.clone()))
            .collect();
        if targets.is_empty() {
            return;
        }
        state
            .buffers
            .entry(session)
            .or_default()
            .push(frame.clone());
        let mut dead = Vec::new();
        for (id, tx) in targets {
            if tx.send(frame.clone()).is_err() {
                dead.push(id);
            }
        }
        for id in dead {
            state.conns.remove(&id);
        }
        drop_unfollowed(&mut state);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One request's bill as a frame carries it; `null` when the piece carries none.
fn bill(usage: Option<Usage>) -> Value {
    match usage {
        Some(u) => json!({
            "prompt": u.prompt,
            "completion": u.completion,
            "cached": u.cached,
            "cache_write": u.cache_write,
            "reasoning": u.reasoning,
        }),
        None => Value::Null,
    }
}

/// A session with no watcher left keeps no buffer.
fn drop_unfollowed(state: &mut State) {
    let followed: HashSet<i64> = state
        .conns
        .values()
        .filter_map(|entry| entry.session)
        .collect();
    state
        .buffers
        .retain(|session, _| followed.contains(session));
}
