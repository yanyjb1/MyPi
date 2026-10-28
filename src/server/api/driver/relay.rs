//! The relay: storage's one port, and the fan-out to the subscription table.

use anyhow::{Context, Result};
use std::sync::mpsc::{self, Receiver};

use crate::ipc::contract::Viewer;
use crate::server::api::convert;
use crate::server::api::notice::Notice;
use crate::server::api::subscribe::event::{
    ContentEvent, Event, SessionEvent, StartKind, TurnUsage,
};
use crate::server::storage::{Relay as Piece, Writer};

use super::subscriptions::Subscriptions;

/// The api's port and the subscribers behind it.
#[derive(Clone, Default)]
pub(crate) struct Relay {
    subs: Subscriptions,
}

impl Relay {
    /// Register the api's port with the writer, and start handing pieces out.
    pub(crate) fn start(w: &Writer) -> Result<Relay> {
        let relay = Relay::default();
        let (port, pieces) = mpsc::channel::<Piece>();
        w.relay(port)
            .context("cannot register the api's relay port")?;
        let subs = relay.subs.clone();
        std::thread::Builder::new()
            .name("api-relay".to_string())
            .spawn(move || {
                for piece in pieces {
                    subs.publish(convert(piece));
                }
            })
            .context("cannot start the relay thread")?;
        Ok(relay)
    }

    pub(crate) fn connect(&self, viewer: &Viewer) -> Receiver<Event> {
        self.subs.connect(viewer)
    }

    pub(crate) fn watch(&self, viewer: &Viewer, session: i64, turn: Option<i64>) -> Result<()> {
        self.subs.watch(viewer, session, turn)
    }

    pub(crate) fn detach(&self, viewer: &Viewer) {
        self.subs.detach(viewer);
    }

    pub(crate) fn watched(&self, session: i64) -> bool {
        self.subs.watched(session)
    }

    pub(crate) fn session_of(&self, viewer: &Viewer) -> Option<i64> {
        self.subs.session_of(viewer)
    }

    pub(crate) fn started(&self, session: i64, turn: i64, kind: StartKind) {
        self.subs.publish(Event::Content(ContentEvent::Started {
            session,
            turn,
            kind,
        }));
    }

    pub(crate) fn tool_output(&self, session: i64, turn: i64, call_id: &str, text: &str) {
        self.subs.publish(Event::Content(ContentEvent::ToolOutput {
            session,
            turn,
            call_id: call_id.to_string(),
            text: text.to_string(),
        }));
    }

    pub(crate) fn ended(
        &self,
        session: i64,
        turn: i64,
        stop_reason: Option<String>,
        usage: Option<TurnUsage>,
    ) {
        self.subs.publish(Event::Content(ContentEvent::Ended {
            session,
            turn,
            stop_reason,
            usage,
        }));
    }

    pub(crate) fn session_meta(&self, session: i64, meta: crate::server::storage::SessionMeta) {
        self.subs.publish(Event::Session(SessionEvent::Meta {
            session,
            meta: convert::session_meta(meta),
        }));
    }

    pub(crate) fn rewound(&self, session: i64, turn: i64, block: i64) {
        self.subs.publish(Event::Session(SessionEvent::Rewound {
            session,
            turn,
            block,
        }));
    }

    pub(crate) fn deleted(&self, session: i64) {
        self.subs
            .publish(Event::Session(SessionEvent::Deleted { session }));
    }

    pub(crate) fn notice(&self, notice: Notice) {
        self.subs.publish(Event::Notice(notice));
    }
}

fn convert(piece: Piece) -> Event {
    match piece {
        Piece::Tail {
            session,
            turn,
            blocks,
            done,
        } => Event::Content(ContentEvent::Live {
            session,
            turn,
            blocks: blocks.into_iter().map(convert::new_block).collect(),
            done,
        }),
        Piece::Blocks {
            session,
            turn,
            blocks,
        } => Event::Content(ContentEvent::Blocks {
            session,
            turn,
            blocks: blocks.into_iter().map(convert::block).collect(),
        }),
        Piece::Ended {
            session,
            turn,
            stop_reason,
            usage,
        } => Event::Content(ContentEvent::Ended {
            session,
            turn,
            stop_reason,
            usage: usage.map(convert::turn_usage),
        }),
    }
}
