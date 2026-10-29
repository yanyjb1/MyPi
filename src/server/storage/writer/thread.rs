//! The writer thread: the only place that touches the write connection. One job at a time,
//! carried out and answered.

use std::sync::mpsc::Receiver;

use super::super::write;
use super::Store;
use super::job::{Answer, Job, What};

/// Serve jobs until every handle is gone.
pub(crate) fn run(work: Receiver<Job>, mut store: Store) {
    while let Ok(job) = work.recv() {
        let answer = match job.what {
            What::Submit(submit) => match write::apply(&mut store.conn, submit) {
                Ok((landed, pieces)) => {
                    for piece in pieces {
                        store.relay_out(piece);
                    }
                    Ok(Answer::Landed(landed))
                }
                Err(e) => Err(e),
            },
            What::Rewind(target) => store.rewind(target).map(Answer::Cut),
            What::DeleteRows(table, ids) => store.delete_rows(table, &ids).map(Answer::Rows),
            What::Export(session) => store.export_memos(session).map(Answer::Export),
            What::Relay(port) => {
                store.relay = Some(port);
                Ok(Answer::Done)
            }
            What::Stream(piece) => {
                store.relay_out(piece);
                Ok(Answer::Done)
            }
        };
        let _ = job.back.send(answer);
    }
}
