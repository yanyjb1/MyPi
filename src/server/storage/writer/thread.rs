//! The writer thread: the only place that touches the write connection. One job at a time,
//! carried out and answered.

use std::sync::mpsc::Receiver;

use super::super::write::apply::apply;
use super::Store;
use super::job::{Answer, Job, What};

/// Serve jobs until every handle is gone.
pub(crate) fn run(work: Receiver<Job>, mut store: Store) {
    while let Ok(job) = work.recv() {
        let answer = match job.what {
            What::Submit(submit) => apply(&mut store.conn, submit).map(Answer::Landed),
            What::Rewind(target) => store.rewind(target).map(Answer::Cut),
            What::DeleteRows(table, ids) => store.delete_rows(table, &ids).map(Answer::Rows),
            What::Export { session, dir } => store.export_memos(session, &dir).map(Answer::Export),
        };
        let _ = job.back.send(answer);
    }
}
