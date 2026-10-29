//! One job on the writer's queue: what to do, and where the answer goes.

use std::sync::mpsc::Sender;

use anyhow::Result;

use super::super::delete::rewind::{Cut, Target};
use super::super::shape::{Landed, Structure, Submit, Table};

/// What the writer is asked to do.
pub(crate) enum What {
    /// Land something: now, or at the batch's end.
    Submit(Submit),
    /// Cut a session's list back to the picked block.
    Rewind(Target),
    /// Delete rows of one table, then sweep.
    DeleteRows(Table, Vec<i64>),
    /// Write this session's memos into the download folder.
    Export(i64),
    /// Take this port as the relay destination.
    Relay(Sender<Structure>),
}

/// What the writer answers with.
pub(crate) enum Answer {
    /// What one submission landed.
    Landed(Landed),
    /// What survived a cut.
    Cut(Cut),
    /// How many rows the delete removed.
    Rows(usize),
    /// Where the export landed (`None` = nothing to export).
    Export(Option<std::path::PathBuf>),
    /// Nothing to hand back.
    Done,
}

impl Answer {
    /// The answer's name, for the message when a caller expected another kind.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Answer::Landed(_) => "landed",
            Answer::Cut(_) => "cut",
            Answer::Rows(_) => "rows",
            Answer::Export(_) => "export",
            Answer::Done => "done",
        }
    }
}

/// One job: what to do, plus the channel the answer goes back on.
pub(crate) struct Job {
    pub(crate) what: What,
    pub(crate) back: Sender<Result<Answer>>,
}
