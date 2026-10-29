//! One job on the writer's queue: what to do, and where the answer goes.

use std::sync::mpsc::Sender;

use anyhow::Result;

use super::super::delete::rewind::{Cut, Target};
use super::super::shape::{Landed, Relay, Submit};

/// What the writer is asked to do.
pub(crate) enum What {
    /// Land something: now, or at the batch's end.
    Submit(Submit),
    /// Cut a session's list back to the picked block.
    Rewind(Target),
    /// Remove a session and everything only it referenced.
    DeleteSession(i64),
    /// Take this port as the relay destination.
    Relay(Sender<Relay>),
    /// Forward one piece at once; nothing lands.
    Stream(Relay),
}

/// What the writer answers with.
pub(crate) enum Answer {
    /// What one submission landed.
    Landed(Landed),
    /// What survived a cut.
    Cut(Cut),
    /// Nothing to hand back.
    Done,
}

impl Answer {
    /// The answer's name, for the message when a caller expected another kind.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Answer::Landed(_) => "landed",
            Answer::Cut(_) => "cut",
            Answer::Done => "done",
        }
    }
}

/// One job: what to do, plus the channel the answer goes back on.
pub(crate) struct Job {
    pub(crate) what: What,
    pub(crate) back: Sender<Result<Answer>>,
}
