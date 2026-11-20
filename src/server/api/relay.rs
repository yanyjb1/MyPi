// The session stream: the connection table, the turn each session is in, and the buffer a late watcher catches up from.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::ipc::contract::{Frame, Text};
use crate::server::agent::r#loop::{Element, Event, Stream, ToolEvent};
use crate::server::agent::request::message::Usage;

// One write's result, ready to be said on the session stream.
pub(crate) struct Signal {
    pub session: i64,
    pub name: &'static str,
    pub content: Value,
}

// One connection's outbound end and what it currently follows.
struct Entry {
    viewer: Option<String>,
    session: Option<i64>,
    tx: Sender<Frame>,
}

// The turn a session is in, the name that opened it, and the viewer that did.
struct Active {
    turn: i64,
    opened_by: &'static str,
    viewer: String,
}

#[derive(Default)]
struct State {
    conns: HashMap<u64, Entry>,
    active: HashMap<i64, Active>,
    // The viewer that put a message in the slot, waiting for the turn it opens.
    pending: HashMap<i64, String>,
    buffers: HashMap<i64, Vec<Frame>>,
    // The sessions a connection followed at the last change.
    watched: HashSet<i64>,
}

// Told, outside the state lock, that a session has no watcher left.
pub(crate) type Closer = Arc<dyn Fn(i64) + Send + Sync>;

// The stream: who follows which session, and what this turn has already said.
#[derive(Clone)]
pub(crate) struct Relay {
    state: Arc<Mutex<State>>,
    closer: Arc<Mutex<Option<Closer>>>,
}

impl Relay {
    // Hand back the one event sender the loop writes to: every frame a front end hears leaves by this queue, in the order the loop sent it.
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

    // Who to tell when a session's last watcher goes away.
    pub(crate) fn close_with(&self, closer: Closer) {
        *self.closer.lock().unwrap_or_else(|e| e.into_inner()) = Some(closer);
    }

    // Tell the closer about every session that just lost its last watcher.
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

    // This connection follows one session, or none, and hears whatever that turn already said.
    pub(crate) fn follow(&self, conn: u64, session: Option<i64>) {
        let gone = {
            let mut state = self.state();
            if let Some(entry) = state.conns.get_mut(&conn) {
                entry.session = session;
            }
            let replay = session
                .and_then(|session| state.buffers.get(&session).cloned())
                .unwrap_or_default();
            if let Some(tx) = state.conns.get(&conn).map(|entry| entry.tx.clone()) {
                for frame in replay {
                    let _ = tx.send(frame);
                }
            }
            drop_unfollowed(&mut state)
        };
        self.report(gone);
    }

    // Is a turn already running on this session?
    pub(crate) fn busy(&self, session: i64) -> bool {
        self.state().active.contains_key(&session)
    }

    // Record the turn this viewer just opened, and tell the others the words that opened it.
    pub(crate) fn open(
        &self,
        conn: u64,
        session: i64,
        viewer: &str,
        turn: i64,
        opened_by: &'static str,
        opening: &str,
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
                    opened_by,
                    viewer: viewer.to_string(),
                },
            );
            drop_unfollowed(&mut state)
        };
        self.report(gone);
        if !opening.is_empty() {
            let said = json!({"signal": "user", "text": opening});
            self.deliver(session, Frame::new("stream", "signal", said), Some(viewer));
        }
    }

    // Remember who is waiting for the slot, or forget it when the slot is cleared.
    pub(crate) fn mail(&self, conn: u64, session: i64, waiting: Option<&str>) {
        let gone = {
            let mut state = self.state();
            match waiting {
                Some(viewer) => {
                    if let Some(entry) = state.conns.get_mut(&conn) {
                        entry.session = Some(session);
                    }
                    state.pending.insert(session, viewer.to_string());
                }
                None => {
                    state.pending.remove(&session);
                }
            }
            drop_unfollowed(&mut state)
        };
        self.report(gone);
    }

    // Only the viewer that opened the turn may change the session while it runs.
    pub(crate) fn writes(&self, session: i64, viewer: &str) -> bool {
        match self.state().active.get(&session) {
            Some(active) => active.viewer == viewer,
            None => true,
        }
    }

    // One write's result: everybody but the issuer hears the field it changed.
    pub(crate) fn signal(&self, signal: Signal, issuer: &str) {
        let Signal {
            session,
            name,
            mut content,
        } = signal;
        if let Some(object) = content.as_object_mut() {
            object.insert("signal".to_string(), json!(name));
        } else {
            content = json!({"signal": name, "content": content});
        }
        self.deliver(
            session,
            Frame::new("stream", "signal", content),
            Some(issuer),
        );
    }

    // One line the api has to say, into the session's stream.
    pub(crate) fn notice(&self, session: i64, text: Text) {
        let payload = json!({"session": session, "text": text});
        self.deliver(session, Frame::new("stream", "notice", payload), None);
    }

    // The session is gone: its buffer, its turn, and every watcher's claim on it.
    pub(crate) fn close(&self, session: i64) {
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

    // One live moment from the loop.
    fn event(&self, event: Event) {
        let session = event.session;
        let element = match event.element {
            Element::Mailbox { turn, text } => {
                let writer = self.slot_opened(session, turn);
                let said = match &text {
                    Some(text) => json!({"signal": "mailbox", "text": text}),
                    None => json!({"signal": "mailbox"}),
                };
                match writer {
                    // The writer echoes its own letter, so its own copy carries no words.
                    Some(writer) => {
                        self.deliver(session, Frame::new("stream", "signal", said), Some(&writer));
                        self.send_one(
                            session,
                            &writer,
                            Frame::new("stream", "signal", json!({"signal": "mailbox"})),
                        );
                    }
                    None => self.deliver(session, Frame::new("stream", "signal", said), None),
                }
                return;
            }
            other => other,
        };

        let (turn, opened_by) = self.turn_of(session);

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
                self.stream(session, turn, opened_by, kind, extra);
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
                        items,
                        result,
                        memo,
                        duration_ms,
                        truncated,
                    } => (
                        "tool_end",
                        json!({
                            "call_id": call_id,
                            "ok": items,
                            "result": result,
                            "memo": memo,
                            "duration_ms": duration_ms,
                            "truncated": truncated.map(|t| json!({"lines": t.lines, "bytes": t.bytes})),
                        }),
                    ),
                };
                self.stream(session, turn, opened_by, kind, extra);
            }
            Element::Notice(notice) => self.notice(session, notice.text),
            // Handled above: the slot opening names the turn instead of pushing a frame.
            Element::Mailbox { .. } => {}
            Element::Structure {
                turn,
                ranges,
                compact,
            } => self.structure(session, turn, ranges, compact),
        }
    }

    // The slot was taken: the writer recorded at write time owns the turn, and an idle session's opener is the slot. A letter taken into a running turn keeps that turn's opener.
    fn slot_opened(&self, session: i64, turn: i64) -> Option<String> {
        let mut state = self.state();
        let viewer = state.pending.remove(&session).filter(|who| !who.is_empty());
        if let Some(active) = state
            .active
            .get(&session)
            .filter(|active| active.turn == turn)
        {
            return Some(active.viewer.clone());
        }
        state.active.insert(
            session,
            Active {
                turn,
                opened_by: "mailbox",
                viewer: viewer.clone().unwrap_or_default(),
            },
        );
        viewer
    }

    // One piece of live output, labelled with the turn and the name that opened it.
    fn stream(&self, session: i64, turn: i64, opened_by: &'static str, kind: &str, extra: Value) {
        let mut payload = json!({"session": session, "turn": turn, "kind": kind});
        if let (Some(object), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                object.insert(key.clone(), value.clone());
            }
        }
        self.deliver(session, Frame::new("stream", opened_by, payload), None);
    }

    // The turn closed: its ranges go out last, then the buffer and the turn go.
    fn structure(&self, session: i64, turn: i64, ranges: Vec<[i64; 2]>, compact: bool) {
        let (_, opened_by) = self.turn_of(session);
        let payload = json!({
            "session": session,
            "turn": turn,
            "ranges": ranges,
            "compact": compact,
        });
        self.deliver(session, Frame::new("stream", opened_by, payload), None);
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

    // One frame for every connection following this session; the issuer never reads its own write.
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
        self.fanout(state, targets, frame);
    }

    // One frame for one viewer's connection, never buffered: the writer's own copy of a signal.
    fn send_one(&self, session: i64, viewer: &str, frame: Frame) {
        let state = self.state();
        let targets: Vec<(u64, Sender<Frame>)> = state
            .conns
            .iter()
            .filter(|(_, entry)| entry.session == Some(session))
            .filter(|(_, entry)| entry.viewer.as_deref() == Some(viewer))
            .map(|(id, entry)| (*id, entry.tx.clone()))
            .collect();
        self.fanout(state, targets, frame);
    }

    // Hand one frame to the targets, drop the dead connections, and report the sessions that just lost their last watcher.
    fn fanout(
        &self,
        mut state: MutexGuard<'_, State>,
        targets: Vec<(u64, Sender<Frame>)>,
        frame: Frame,
    ) {
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

// One request's bill as a frame carries it; `null` when the piece carries none.
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

// A session with no watcher left keeps no buffer, and is handed back to be unbooked.
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
