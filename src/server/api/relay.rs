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
    /// The sessions a connection followed at the last change.
    watched: HashSet<i64>,
}

/// Told, outside the state lock, that a session has no watcher left.
pub(crate) type Closer = Arc<dyn Fn(i64) + Send + Sync>;

/// The stream: who follows which session, and what this turn has already said.
#[derive(Clone)]
pub(crate) struct Relay {
    state: Arc<Mutex<State>>,
    closer: Arc<Mutex<Option<Closer>>>,
}

impl Relay {
    /// Hand back the one event sender the loop writes to: every frame a front end hears leaves by
    /// this queue, in the order the loop sent it.
    pub(crate) fn start() -> Result<(Relay, Sender<Event>)> {
        let relay = Relay {
            state: Arc::new(Mutex::new(State::default())),
            closer: Arc::new(Mutex::new(None)),
        };
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

    /// Who to tell when a session's last watcher goes away.
    pub(crate) fn close_with(&self, closer: Closer) {
        *self.closer.lock().unwrap_or_else(|e| e.into_inner()) = Some(closer);
    }

    /// Tell the closer about every session that just lost its last watcher.
    fn report(&self, gone: Vec<i64>) {
        if gone.is_empty() {
            return;
        }
        let closer = self
            .closer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(closer) = closer {
            let closer: &(dyn Fn(i64) + Send + Sync) = &*closer;
            for session in gone {
                closer(session);
            }
        }
    }

    pub(crate) fn detach(&self, conn: u64) {
        let gone = {
            let mut state = self.state();
            state.conns.remove(&conn);
            drop_unfollowed(&mut state)
        };
        self.report(gone);
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
        let gone = {
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
            drop_unfollowed(&mut state)
        };
        self.report(gone);
    }

    /// This connection stops following whatever it followed.
    pub(crate) fn unwatch(&self, conn: u64) {
        let gone = {
            let mut state = self.state();
            if let Some(entry) = state.conns.get_mut(&conn) {
                entry.session = None;
            }
            drop_unfollowed(&mut state)
        };
        self.report(gone);
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
        let gone = {
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
            drop_unfollowed(&mut state)
        };
        self.report(gone);
    }

    /// Remember who wrote the slot; the turn it opens is named after the slot, nobody else.
    pub(crate) fn note_mail(&self, conn: u64, session: i64, viewer: &str) {
        let gone = {
            let mut state = self.state();
            if let Some(entry) = state.conns.get_mut(&conn) {
                entry.session = Some(session);
            }
            state.pending.insert(session, viewer.to_string());
            drop_unfollowed(&mut state)
        };
        self.report(gone);
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

    /// One change of the api's own: everybody but the issuer hears the field it changed.
    pub(crate) fn change(&self, session: i64, changed: Value, issuer: &str) {
        self.deliver(
            session,
            Frame::new("stream", "signal", changed),
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
        let gone = {
            let mut state = self.state();
            state.buffers.remove(&session);
            state.active.remove(&session);
            state.pending.remove(&session);
            for entry in state.conns.values_mut() {
                if entry.session == Some(session) {
                    entry.session = None;
                }
            }
            drop_unfollowed(&mut state)
        };
        self.report(gone);
    }

    /// One live moment from the loop.
    fn event(&self, event: Event) {
        let session = event.session;
        let element = match event.element {
            Element::Mailbox { turn } => {
                self.slot_opened(session, turn);
                self.deliver(
                    session,
                    Frame::new("stream", "signal", json!({"signal": "mailbox"})),
                    None,
                );
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
                        truncated,
                    } => (
                        "tool_end",
                        json!({
                            "call_id": call_id,
                            "ok": ok,
                            "result": result,
                            "memo": memo,
                            "duration_ms": duration_ms,
                            "truncated": truncated.map(|t| json!({"lines": t.lines, "bytes": t.bytes})),
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
            Element::Structure { turn, ranges } => self.structure(session, turn, ranges),
        }
    }

    /// The slot was taken: the writer recorded at write time owns the turn, and the door is the
    /// slot. A letter taken into a turn that is already running keeps that turn's door.
    fn slot_opened(&self, session: i64, turn: i64) {
        let mut state = self.state();
        let viewer = state.pending.remove(&session).unwrap_or_default();
        if state
            .active
            .get(&session)
            .is_some_and(|active| active.turn == turn)
        {
            return;
        }
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

    /// The turn closed: its ranges go out last, then the buffer and the turn go.
    fn structure(&self, session: i64, turn: i64, ranges: Vec<[i64; 2]>) {
        let (_, door) = self.turn_of(session);
        let payload = json!({
            "session": session,
            "turn": turn,
            "ranges": ranges,
        });
        self.deliver(session, Frame::new("stream", door, payload), None);
        let mut state = self.state();
        state.buffers.remove(&session);
        state.active.remove(&session);
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
        let gone = drop_unfollowed(&mut state);
        drop(state);
        self.report(gone);
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

/// A session with no watcher left keeps no buffer, and is handed back to be unbooked.
fn drop_unfollowed(state: &mut State) -> Vec<i64> {
    let followed: HashSet<i64> = state
        .conns
        .values()
        .filter_map(|entry| entry.session)
        .collect();
    let gone: Vec<i64> = state.watched.difference(&followed).copied().collect();
    state
        .buffers
        .retain(|session, _| followed.contains(session));
    state.watched = followed;
    gone
}
