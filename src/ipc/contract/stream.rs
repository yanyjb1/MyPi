//! The stream's vocabulary: content, session changes, and the server's notice.

use super::{Block, NewBlock, SessionMeta};

/// One subscription request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Subscribe {
    Watch {
        session: i64,
    },
    Submit {
        session: i64,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    Compact {
        session: i64,
        retention: Retention,
        focus: Option<String>,
    },
}

/// What opening a stretch answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Opened {
    Started { turn: i64 },
    Busy,
}

/// How much of the transcript tail to keep verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Retention {
    Default,
    Tokens(usize),
    Blocks(usize),
}

/// How a notice reads on a front end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

/// One line the server says about a session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Notice {
    pub session: i64,
    pub level: NoticeLevel,
    pub content: String,
}

/// One piece on a session's stream.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Event {
    Content(ContentEvent),
    Session(SessionEvent),
    Notice(Notice),
}

/// What a round sends.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ContentEvent {
    Started {
        session: i64,
        turn: i64,
        kind: StartKind,
    },
    Live {
        session: i64,
        turn: i64,
        blocks: Vec<NewBlock>,
        done: bool,
    },
    Blocks {
        session: i64,
        turn: i64,
        blocks: Vec<Block>,
    },
    ToolOutput {
        session: i64,
        turn: i64,
        call_id: String,
        text: String,
    },
    Ended {
        session: i64,
        turn: i64,
        stop_reason: Option<String>,
        usage: Option<TurnUsage>,
    },
}

/// What a session itself says.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SessionEvent {
    Meta { session: i64, meta: SessionMeta },
    Rewound { session: i64, turn: i64, block: i64 },
    Deleted { session: i64 },
}

/// How a round began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    Submit,
    Compact,
    Wake,
}

/// One round's usage on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TurnUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
}

impl ContentEvent {
    pub fn session(&self) -> i64 {
        match self {
            ContentEvent::Started { session, .. }
            | ContentEvent::Live { session, .. }
            | ContentEvent::Blocks { session, .. }
            | ContentEvent::ToolOutput { session, .. }
            | ContentEvent::Ended { session, .. } => *session,
        }
    }

    pub fn turn(&self) -> i64 {
        match self {
            ContentEvent::Started { turn, .. }
            | ContentEvent::Live { turn, .. }
            | ContentEvent::Blocks { turn, .. }
            | ContentEvent::ToolOutput { turn, .. }
            | ContentEvent::Ended { turn, .. } => *turn,
        }
    }
}

impl SessionEvent {
    pub fn session(&self) -> i64 {
        match self {
            SessionEvent::Meta { session, .. }
            | SessionEvent::Rewound { session, .. }
            | SessionEvent::Deleted { session } => *session,
        }
    }
}

impl Event {
    pub fn session(&self) -> i64 {
        match self {
            Event::Content(event) => event.session(),
            Event::Session(event) => event.session(),
            Event::Notice(notice) => notice.session,
        }
    }

    pub fn turn(&self) -> Option<i64> {
        match self {
            Event::Content(event) => Some(event.turn()),
            Event::Session(_) | Event::Notice(_) => None,
        }
    }
}
