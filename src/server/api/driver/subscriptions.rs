//! The subscription table: one stream per session, many subscribers on it.

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::ipc::contract::subscribe::{
    Blocks, Deleted, Live, Meta, Rewound, RoundEnded, RoundStarted, ToolOutput,
};
use crate::ipc::contract::{Frame, Kind, Viewer};

/// The stream table.
#[derive(Clone, Default)]
pub(crate) struct Subscriptions {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    streams: HashMap<i64, SessionStream>,
    attached: HashMap<Viewer, i64>,
}

struct SessionStream {
    turn: Option<TurnBuffer>,
    subscribers: Vec<Subscriber>,
}

struct Subscriber {
    viewer: Viewer,
    role: Role,
    turn: Option<i64>,
    tx: Sender<Frame>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Writer,
    Watcher,
}

struct TurnBuffer {
    turn: i64,
    frames: Vec<Frame>,
}

impl Subscriptions {
    /// Attach one subscriber to a session's stream; it first gets the buffered turn.
    pub(crate) fn attach(
        &self,
        viewer: &Viewer,
        session: i64,
        role: Role,
        turn: Option<i64>,
        tx: Sender<Frame>,
    ) {
        let mut state = self.state();
        let stream = state
            .streams
            .entry(session)
            .or_insert_with(|| SessionStream {
                turn: None,
                subscribers: Vec::new(),
            });
        if let Some(buffer) = &stream.turn
            && turn.is_none_or(|t| t == buffer.turn)
        {
            for frame in &buffer.frames {
                if tx.send(frame.clone()).is_err() {
                    return;
                }
            }
        }
        stream.subscribers.push(Subscriber {
            viewer: viewer.clone(),
            role,
            turn,
            tx,
        });
        state.attached.insert(viewer.clone(), session);
    }

    /// Drop every subscription this viewer holds.
    pub(crate) fn detach(&self, viewer: &Viewer) {
        let mut state = self.state();
        let Some(session) = state.attached.remove(viewer) else {
            return;
        };
        if let Some(stream) = state.streams.get_mut(&session) {
            stream.subscribers.retain(|s| s.viewer != *viewer);
        }
    }

    /// Is anybody on this session's stream?
    pub(crate) fn watched(&self, session: i64) -> bool {
        self.state()
            .streams
            .get(&session)
            .is_some_and(|s| !s.subscribers.is_empty())
    }

    /// The session this viewer is attached to.
    pub(crate) fn session_of(&self, viewer: &Viewer) -> Option<i64> {
        self.state().attached.get(viewer).copied()
    }

    /// Deliver one frame to the stream named by the frame's own session.
    pub(crate) fn deliver(&self, frame: Frame) {
        let Some(session) = frame.payload.get("session").and_then(|v| v.as_i64()) else {
            return;
        };
        let mut state = self.state();
        buffer_turn(&mut state.streams, session, &frame);
        let Some(stream) = state.streams.get_mut(&session) else {
            return;
        };

        let name = frame.name.as_str();
        let turn = frame.payload.get("turn").and_then(|v| v.as_i64());
        let mut dead = Vec::new();
        for sub in &mut stream.subscribers {
            if sub.role == Role::Writer && is_post_metadata(name) {
                continue;
            }
            if let (Some(want), Some(now)) = (sub.turn, turn)
                && want != now
            {
                continue;
            }
            if sub.tx.send(frame.clone()).is_err() {
                dead.push(sub.viewer.clone());
            }
        }
        let stream = state.streams.get_mut(&session).expect("checked above");
        stream.subscribers.retain(|s| !dead.contains(&s.viewer));

        if name == "round_ended" {
            if let Some(turn) = turn {
                state.streams.get_mut(&session).map(|s| {
                    s.turn = None;
                    s.subscribers.retain(|sub| sub.turn != Some(turn));
                });
            }
        }
        if name == "deleted" {
            state.streams.remove(&session);
            state.attached.retain(|_, attached| *attached != session);
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Only a round's own frames are buffered, and only one round at a time.
fn buffer_turn(streams: &mut HashMap<i64, SessionStream>, session: i64, frame: &Frame) {
    let turn = match frame.name.as_str() {
        "round_started" | "live" | "blocks" | "tool_output" | "round_ended" => {
            frame.payload.get("turn").and_then(|v| v.as_i64())
        }
        _ => None,
    };
    let Some(turn) = turn else { return };
    let stream = streams.entry(session).or_insert_with(|| SessionStream {
        turn: None,
        subscribers: Vec::new(),
    });
    match &mut stream.turn {
        Some(buffer) if buffer.turn == turn => buffer.frames.push(frame.clone()),
        Some(buffer) => {
            *buffer = TurnBuffer {
                turn,
                frames: vec![frame.clone()],
            }
        }
        None => {
            stream.turn = Some(TurnBuffer {
                turn,
                frames: vec![frame.clone()],
            })
        }
    }
}

/// Metadata writes the writer itself caused; it does not need them echoed back.
fn is_post_metadata(name: &str) -> bool {
    matches!(name, "meta" | "rewound" | "deleted")
}

/// Frame builders for the eight stream names.
pub(crate) mod frames {
    use super::*;

    pub(crate) fn round_started(v: &RoundStarted) -> Frame {
        frame("round_started", v)
    }

    pub(crate) fn notice(v: &crate::ipc::contract::Notice) -> Frame {
        Frame::new(
            Kind::Notice,
            "notice",
            serde_json::to_value(v).unwrap_or_else(|_| serde_json::json!({})),
        )
    }

    pub(crate) fn live(v: &Live) -> Frame {
        frame("live", v)
    }

    pub(crate) fn blocks(v: &Blocks) -> Frame {
        frame("blocks", v)
    }

    pub(crate) fn tool_output(v: &ToolOutput) -> Frame {
        frame("tool_output", v)
    }

    pub(crate) fn round_ended(v: &RoundEnded) -> Frame {
        frame("round_ended", v)
    }

    pub(crate) fn meta(v: &Meta) -> Frame {
        frame("meta", v)
    }

    pub(crate) fn rewound(v: &Rewound) -> Frame {
        frame("rewound", v)
    }

    pub(crate) fn deleted(v: &Deleted) -> Frame {
        frame("deleted", v)
    }

    fn frame(name: &str, payload: impl serde::Serialize) -> Frame {
        Frame::new(
            Kind::Subscribe,
            name,
            serde_json::to_value(payload).unwrap_or_else(|_| serde_json::json!({})),
        )
    }
}
