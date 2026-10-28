//! The subscription table and the current-turn buffer: one stream per viewer.

use anyhow::{Result, anyhow, bail};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::ipc::contract::Viewer;
use crate::server::api::notice::Notice;
use crate::server::api::subscribe::event::{ContentEvent, Event, SessionEvent};

/// The subscription table.
#[derive(Clone, Default)]
pub(crate) struct Subscriptions {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    slots: HashMap<Viewer, Slot>,
    current: HashMap<i64, TurnBuffer>,
    pending: HashMap<i64, Vec<Notice>>,
}

struct Slot {
    sender: Sender<Event>,
    session: Option<i64>,
    turn: Option<i64>,
}

struct TurnBuffer {
    turn: i64,
    events: Vec<Event>,
}

impl Subscriptions {
    /// Register a viewer's one stream.
    pub(crate) fn connect(&self, viewer: &Viewer) -> Receiver<Event> {
        let (sender, receiver) = mpsc::channel();
        self.state().slots.insert(
            viewer.clone(),
            Slot {
                sender,
                session: None,
                turn: None,
            },
        );
        receiver
    }

    /// Point a viewer at a session, and at one turn when it asked for one.
    pub(crate) fn watch(&self, viewer: &Viewer, session: i64, turn: Option<i64>) -> Result<()> {
        let mut state = self.state();
        let State {
            slots,
            current,
            pending,
        } = &mut *state;
        let Some(slot) = slots.get_mut(viewer) else {
            bail!("viewer is not connected");
        };
        if let Some(buffer) = current.get(&session)
            && (turn.is_none() || Some(buffer.turn) == turn)
        {
            for event in &buffer.events {
                slot.sender
                    .send(event.clone())
                    .map_err(|_| anyhow!("the stream is closed"))?;
            }
        }
        slot.session = Some(session);
        slot.turn = turn;
        if let Some(list) = pending.remove(&session) {
            for notice in list {
                slot.sender
                    .send(Event::Notice(notice))
                    .map_err(|_| anyhow!("the stream is closed"))?;
            }
        }
        Ok(())
    }

    /// Drop a viewer's stream.
    pub(crate) fn detach(&self, viewer: &Viewer) {
        self.state().slots.remove(viewer);
    }

    /// Is anybody watching this session?
    pub(crate) fn watched(&self, session: i64) -> bool {
        self.state()
            .slots
            .values()
            .any(|slot| slot.session == Some(session))
    }

    /// The session a viewer is attached to.
    pub(crate) fn session_of(&self, viewer: &Viewer) -> Option<i64> {
        self.state().slots.get(viewer).and_then(|slot| slot.session)
    }

    /// Hand one event to the matching viewers.
    pub(crate) fn publish(&self, event: Event) {
        let mut state = self.state();
        let State {
            slots,
            current,
            pending,
        } = &mut *state;
        let session = event.session();
        let turn = event.turn();

        if let Some(turn) = turn {
            let buffer = current.entry(session).or_insert(TurnBuffer {
                turn,
                events: Vec::new(),
            });
            if buffer.turn != turn {
                *buffer = TurnBuffer {
                    turn,
                    events: Vec::new(),
                };
            }
            buffer.events.push(event.clone());
        }

        let mut sent = false;
        let mut dead = Vec::new();
        for (viewer, slot) in slots.iter_mut() {
            if slot.session != Some(session) {
                continue;
            }
            if let Some(turn) = turn
                && slot.turn.is_some()
                && slot.turn != Some(turn)
            {
                continue;
            }
            sent = true;
            if slot.sender.send(event.clone()).is_err() {
                dead.push(viewer.clone());
            }
        }
        for viewer in dead {
            slots.remove(&viewer);
        }

        if let Event::Content(ContentEvent::Ended { turn, .. }) = &event {
            current.remove(&session);
            slots.retain(|_, slot| !(slot.session == Some(session) && slot.turn == Some(*turn)));
        }
        if let Event::Session(SessionEvent::Deleted { session }) = &event {
            current.remove(session);
            pending.remove(session);
            slots.retain(|_, slot| slot.session != Some(*session));
        }
        if let Event::Notice(notice) = &event
            && !sent
        {
            pending.entry(session).or_default().push(notice.clone());
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
