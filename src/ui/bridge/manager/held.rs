// One held conversation: its connection, the question in flight, its listeners, its round, and whether it is booked.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ipc::client::Client;
use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

// How long a question waits for its answer before the connection is called dead.
const WAIT: Duration = Duration::from_secs(30);

// One held conversation: its connection, the question in flight, its listeners, and the round.
pub struct Held {
    pub session: Option<i64>,
    client: Client,
    state: Mutex<State>,
    // One question at a time: a reply names only kind and name, so a second would take the first's answer.
    job: Mutex<()>,
    touched: Mutex<Instant>,
    dead: Mutex<bool>,
}

struct State {
    // The question waiting for an answer that carries this kind and name.
    pending: Option<(String, String, Sender<Frame>)>,
    // Every event connection reading this session, under the id it joined with.
    listeners: Vec<(u64, Sender<Frame>)>,
    next: u64,
    // The round now running, as it was said: a listener that arrives late still sees all of it.
    round: Vec<Frame>,
    busy: bool,
    // Booked by an adapter that handed it over: nobody watching, and still not handed back.
    hosted: bool,
}

impl Held {
    pub(super) fn open(endpoint: &Endpoint, session: Option<i64>) -> Result<Held, String> {
        let viewer = viewer(session);
        let client = Client::connect(endpoint, &viewer).map_err(|e| e.to_string())?;
        let held = Held {
            session,
            client,
            state: Mutex::new(State {
                pending: None,
                listeners: Vec::new(),
                next: 0,
                round: Vec::new(),
                busy: false,
                hosted: false,
            }),
            job: Mutex::new(()),
            touched: Mutex::new(Instant::now()),
            dead: Mutex::new(false),
        };
        if let Some(session) = session {
            // Watch is fire and forget: no answer comes, and this connection now hears the conversation.
            held.tell(Frame::new(
                "subscribe",
                "watch",
                json!({"session": session}),
            ));
        }
        Ok(held)
    }

    // One question and its answer, or nothing when the connection died or the answer never came.
    pub fn ask(&self, mut frame: Frame) -> Result<Frame, String> {
        let _job = self.job.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(session), Some(object)) = (self.session, frame.payload.as_object_mut())
            && !object.contains_key("session")
        {
            object.insert("session".to_string(), json!(session));
        }
        let (tx, rx) = mpsc::channel();
        {
            let mut state = self.state();
            state.pending = Some((frame.kind.clone(), frame.name.clone(), tx));
        }
        if self.client.send(frame.clone()).is_err() {
            self.state().pending = None;
            return Err("upstream_closed".to_string());
        }
        self.touch();
        match rx.recv_timeout(WAIT) {
            Ok(answer) => Ok(answer),
            Err(_) => {
                self.state().pending = None;
                Err("upstream_silent".to_string())
            }
        }
    }

    // A frame the server answers nothing to.
    pub fn tell(&self, frame: Frame) {
        let _ = self.client.send(frame);
        self.touch();
    }

    // Nothing follows this connection any more: handing it back unbooks it, its jobs and tabs with it.
    pub fn close(&self) {
        if let Some(_session) = self.session {
            let _ = self
                .client
                .send(Frame::new("subscribe", "watch", json!({"session": null})));
        }
        let _ = self.client.send(Frame::new("register", "bye", json!({})));
        *self.dead.lock().unwrap_or_else(|e| e.into_inner()) = true;
        // The read half is asleep in `next`: closing the socket is what wakes it.
        self.client.close();
    }

    // The id this listener joined under, so a peer that leaves takes only its own stream.
    pub fn listen(&self, out: Sender<Frame>) -> Option<u64> {
        let mut state = self.state();
        // Everything the round has said so far, so a listener joining mid-turn loses nothing.
        for frame in state.round.iter() {
            if out.send(frame.clone()).is_err() {
                return None;
            }
        }
        state.next += 1;
        let id = state.next;
        state.listeners.push((id, out));
        drop(state);
        self.touch();
        Some(id)
    }

    // A peer that left stops listening: its stream goes, so a quiet conversation can be let go.
    pub fn unlisten(&self, id: u64) {
        self.state().listeners.retain(|(held, _)| *held != id);
    }

    pub fn is_dead(&self) -> bool {
        *self.dead.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Whether a round is running on this conversation, as its own frames have said.
    pub fn busy(&self) -> bool {
        self.state().busy
    }

    // Keep this connection open with nobody watching: the whole of what hosting is.
    pub fn book(&self) {
        self.state().hosted = true;
    }

    // Give it back: from here only a watcher or a running round keeps it.
    pub fn unbook(&self) {
        self.state().hosted = false;
    }

    // Whether an adapter handed this connection over and has not taken it back.
    pub fn is_hosted(&self) -> bool {
        self.state().hosted
    }

    pub(super) fn is_stale(&self, idle: Duration) -> bool {
        let state = self.state();
        if state.busy || !state.listeners.is_empty() {
            return false;
        }
        drop(state);
        self.last() >= idle
    }

    fn last(&self) -> Duration {
        self.touched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    fn touch(&self) {
        *self.touched.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // The next frame the server says, handed to the question waiting for it or to whoever listens.
    // Waiting blocks until the next frame or until the connection is gone.
    pub(super) fn read(&self) {
        loop {
            let Some(frame) = self.client.next() else {
                *self.dead.lock().unwrap_or_else(|e| e.into_inner()) = true;
                let notice = Frame::new("local", "disconnected", json!({}));
                self.fanout(&notice);
                // The stream ends here too: every peer waiting on it is let go, not kept waiting forever.
                self.state().listeners.clear();
                return;
            };
            self.absorb(frame);
        }
    }

    fn absorb(&self, frame: Frame) {
        if frame.kind == "stream" || frame.kind == "local" {
            self.fanout(&frame);
            return;
        }
        let mut state = self.state();
        let mut asked = false;
        if let Some((kind, name, _)) = &state.pending {
            asked = *kind == frame.kind && *name == frame.name;
        }
        if asked && let Some((_, _, tx)) = state.pending.take() {
            let _ = tx.send(frame);
            return;
        }
        drop(state);
        self.fanout(&frame);
    }

    // One frame: buffered as the round's own while a round runs, then handed to every listener.
    fn fanout(&self, frame: &Frame) {
        let mut state = self.state();
        if frame.kind == "stream" {
            if closes_frame(frame) {
                // The round is over: its receipt leaves last, and the round it described is gone.
                state.busy = false;
                state.round.clear();
            } else {
                state.busy = true;
                // Buffered whether or not anyone listens, so a page that reconnects loses nothing.
                state.round.push(frame.clone());
            }
        }
        let mut dead = Vec::new();
        for (at, (_, listener)) in state.listeners.iter().enumerate() {
            if listener.send(frame.clone()).is_err() {
                dead.push(at);
            }
        }
        for at in dead.into_iter().rev() {
            state.listeners.remove(at);
        }
        drop(state);
        self.touch();
    }
}

// The frame a turn closes with: the round's own ranges.
fn closes_frame(frame: &Frame) -> bool {
    frame.name != "notice" && frame.name != "signal" && frame.payload.get("ranges").is_some()
}

// This connection's name on the wire; each held session is its own front end, so no two share one.
fn viewer(session: Option<i64>) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let tag = session
        .map(|id| id.to_string())
        .unwrap_or_else(|| "bare".to_string());
    Value::String(format!("bridge-{}-{tag}-{now}", std::process::id()))
}
