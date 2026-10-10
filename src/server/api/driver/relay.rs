//! The relay: storage's one port, and the fan-out to whoever asked for a session. A subscriber is a
//! **viewer**: one session, one stream per viewer.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::server::api::reply::stream::{Relayed, StreamItem};
use crate::server::api::types::Viewer;
use crate::server::storage::{Relay as Piece, Writer};

/// The api's port and the subscribers behind it.
#[derive(Clone, Default)]
pub(crate) struct Relay {
    sinks: Arc<Mutex<HashMap<i64, Vec<(Viewer, Sender<StreamItem>)>>>>,
}

impl Relay {
    /// Register the api's port with the writer, and start handing pieces out.
    pub(crate) fn start(w: &Writer) -> Result<Relay> {
        let relay = Relay::default();
        let (port, pieces) = mpsc::channel::<Piece>();
        w.relay(port).context("cannot register the api's relay port")?;
        let handing = relay.clone();
        std::thread::Builder::new()
            .name("api-relay".to_string())
            .spawn(move || {
                for piece in pieces {
                    handing.hand(Relayed::from(piece));
                }
            })
            .context("cannot start the relay thread")?;
        Ok(relay)
    }

    /// Ask for one session's stream, as this viewer. Asking twice replaces the old stream.
    pub(crate) fn attach(&self, viewer: Viewer, session: i64) -> Receiver<StreamItem> {
        let (sink, stream) = mpsc::channel();
        let mut sinks = self.sinks();
        let list = sinks.entry(session).or_default();
        list.retain(|(v, _)| *v != viewer);
        list.push((viewer, sink));
        stream
    }

    /// Stop watching: this viewer's stream of that session is dropped.
    pub(crate) fn detach(&self, viewer: Viewer, session: i64) {
        let mut sinks = self.sinks();
        let Some(list) = sinks.get_mut(&session) else {
            return;
        };
        list.retain(|(v, _)| *v != viewer);
        if list.is_empty() {
            sinks.remove(&session);
        }
    }

    /// Is anybody still watching this session?
    pub(crate) fn watched(&self, session: i64) -> bool {
        self.sinks()
            .get(&session)
            .is_some_and(|list| !list.is_empty())
    }

    /// Hand one piece to the subscribers of its session.
    pub(crate) fn emit(&self, session: i64, item: StreamItem) {
        self.hand(Relayed { session, item });
    }

    /// One piece, to whoever asked for that session. A subscriber that dropped
    /// its receiver is forgotten here.
    fn hand(&self, piece: Relayed) {
        let Relayed { session, item } = piece;
        let mut sinks = self.sinks();
        let Some(list) = sinks.get_mut(&session) else {
            return;
        };
        list.retain(|(_, sink)| sink.send(item.clone()).is_ok());
        if list.is_empty() {
            sinks.remove(&session);
        }
    }

    fn sinks(&self) -> MutexGuard<'_, HashMap<i64, Vec<(Viewer, Sender<StreamItem>)>>> {
        self.sinks.lock().unwrap_or_else(|e| e.into_inner())
    }
}
