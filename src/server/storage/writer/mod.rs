//! The writer: the one thing that writes. `Writer` is a handle; one thread owns the connection,
//! so exactly one writer ever touches the database.

pub(crate) mod inlet;
pub(crate) mod job;
mod thread;

use std::path::Path;
use std::sync::mpsc::{self, Sender};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::delete::rewind::{Cut, Target};
use super::schema;
use super::shape::{Landed, Relay, Submit};
use inlet::Inlet;
use job::{Answer, Job, What};

/// What the writer thread owns: the connection, plus the write state that
/// cannot live in the database (see [`inlet`]).
pub(crate) struct Store {
    pub(crate) conn: Connection,
    pub(crate) inlet: Inlet,
}

impl Store {
    fn new(conn: Connection, batch: usize) -> Self {
        Self {
            conn,
            inlet: Inlet::new(batch),
        }
    }
}

/// The write door, as a handle.
#[derive(Clone)]
pub struct Writer {
    jobs: Sender<Job>,
}

impl Writer {
    /// Open the database and start the writer thread. `batch` is the config's `batchTools`; the
    /// layout is checked before the thread starts.
    pub fn open(path: &Path, batch: usize) -> Result<Writer> {
        let store = Store::new(schema::prepare(path)?, batch);
        let (jobs, work) = mpsc::channel();
        std::thread::Builder::new()
            .name("storage-writer".to_string())
            .spawn(move || thread::run(work, store))
            .context("cannot start the writer thread")?;
        Ok(Writer { jobs })
    }

    /// Hand one thing over, and wait for what landed.
    pub fn push(&self, submit: Submit) -> Result<Landed> {
        match self.call(What::Submit(submit))? {
            Answer::Landed(landed) => Ok(landed),
            other => anyhow::bail!("the writer answered `{}` to a submit", other.name()),
        }
    }

    /// Take this port as the relay destination: every new piece goes there.
    pub fn relay(&self, port: Sender<Relay>) -> Result<()> {
        self.call(What::Relay(port)).map(|_| ())
    }

    /// Cut a session's list back to the picked block (prune mode), then sweep
    /// what nothing references any more.
    pub fn rewind(&self, target: Target) -> Result<Cut> {
        match self.call(What::Rewind(target))? {
            Answer::Cut(cut) => Ok(cut),
            other => anyhow::bail!("the writer answered `{}` to a rewind", other.name()),
        }
    }

    /// Remove a session and everything only it referenced.
    pub fn delete_session(&self, session: i64) -> Result<()> {
        self.call(What::DeleteSession(session)).map(|_| ())
    }

    /// One round trip: hand the job over, wait for the answer.
    fn call(&self, what: What) -> Result<Answer> {
        let (back, answer) = mpsc::channel();
        self.jobs
            .send(Job { what, back })
            .context("the writer thread is gone")?;
        answer.recv().context("the writer thread is gone")?
    }
}
