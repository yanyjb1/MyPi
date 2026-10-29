//! The write door and what the writer thread owns.

use std::path::Path;
use std::sync::mpsc::{self, Sender};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::super::delete::rewind::{Cut, Target};
use super::super::schema;
use super::super::shape::{Landed, Relay, Submit, Table};
use super::job::{Answer, Job, What};
use super::thread;

/// What the writer thread owns: the connection and the relay port.
pub(crate) struct Store {
    pub(crate) conn: Connection,
    pub(crate) relay: Option<Sender<Relay>>,
}

impl Store {
    fn new(conn: Connection) -> Self {
        Self { conn, relay: None }
    }

    /// Hand one piece to the relay port, if anybody listens.
    pub(crate) fn relay_out(&mut self, piece: Relay) {
        let dead = match self.relay.as_ref() {
            Some(port) => port.send(piece).is_err(),
            None => false,
        };
        if dead {
            // The api's receiver is gone: stop relaying to it.
            self.relay = None;
        }
    }
}

/// The write door, as a handle.
#[derive(Clone)]
pub struct Writer {
    jobs: Sender<Job>,
    path: Option<std::path::PathBuf>,
}

impl Writer {
    /// Open the database and start the writer thread. The layout is checked before the thread
    /// starts.
    pub fn open(path: &Path) -> Result<Writer> {
        let store = Store::new(schema::prepare(path)?);
        let (jobs, work) = mpsc::channel();
        std::thread::Builder::new()
            .name("storage-writer".to_string())
            .spawn(move || thread::run(work, store))
            .context("cannot start the writer thread")?;
        Ok(Writer {
            jobs,
            path: Some(path.to_path_buf()),
        })
    }

    /// The database path, for readers that open their own connections.
    pub fn path(&self) -> std::path::PathBuf {
        self.path.clone().unwrap_or_default()
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

    /// Forward one piece at once without landing anything.
    pub fn stream(&self, piece: Relay) -> Result<()> {
        self.call(What::Stream(piece)).map(|_| ())
    }

    /// Cut a session's list back to the picked block, then sweep what nothing references.
    pub fn rewind(&self, target: Target) -> Result<Cut> {
        match self.call(What::Rewind(target))? {
            Answer::Cut(cut) => Ok(cut),
            other => anyhow::bail!("the writer answered `{}` to a rewind", other.name()),
        }
    }

    /// Delete rows of one table, with whatever only they held; returns how many went.
    pub fn delete_rows(&self, table: Table, ids: &[i64]) -> Result<usize> {
        match self.call(What::DeleteRows(table, ids.to_vec()))? {
            Answer::Rows(n) => Ok(n),
            other => anyhow::bail!("the writer answered `{}` to a row delete", other.name()),
        }
    }

    /// Write this session's memos into the download folder; `None` = nothing to export.
    pub fn export_memos(&self, session: i64) -> Result<Option<std::path::PathBuf>> {
        match self.call(What::Export(session))? {
            Answer::Export(path) => Ok(path),
            other => anyhow::bail!("the writer answered `{}` to an export", other.name()),
        }
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
