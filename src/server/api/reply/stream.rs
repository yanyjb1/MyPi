//! The stream: what a subscriber receives while a round runs and when it ends. Nothing internal
//! leaks through this face.

use crate::server::storage::{Block, NewBlock, Relay};

/// One forwarded step of a running round, applied in order by a front end.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum StreamItem {
    /// The tail that has not landed yet: the blocks a streaming reply produced, or the api's own
    /// mid-flight preview. A front end **replaces** its preview with the latest of these.
    Live(Vec<NewBlock>),
    /// A batch landed: its blocks, ids included, in order.
    Blocks(Vec<Block>),
    /// What a tool printed while it ran. Display only: the result block carries
    /// the durable text.
    ToolOutput { call_id: String, text: String },
    /// The round ended; the reason as stored, and which round it was.
    Ended {
        turn: Option<i64>,
        stop_reason: Option<String>,
    },
}

/// One piece off storage's port: the session it belongs to, and what it says.
#[derive(Debug, Clone)]
pub struct Relayed {
    pub session: i64,
    pub item: StreamItem,
}

impl From<Relay> for Relayed {
    /// The one place storage's vocabulary becomes the surface's: what it
    /// relays, forwarded as is.
    fn from(relay: Relay) -> Self {
        match relay {
            Relay::Tail { session, blocks } => Relayed {
                session,
                item: StreamItem::Live(blocks),
            },
            Relay::Blocks { session, blocks } => Relayed {
                session,
                item: StreamItem::Blocks(blocks),
            },
            Relay::Ended {
                session,
                turn,
                stop_reason,
            } => Relayed {
                session,
                item: StreamItem::Ended { turn, stop_reason },
            },
        }
    }
}
