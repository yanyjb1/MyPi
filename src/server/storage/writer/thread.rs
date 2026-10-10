//! The writer thread: the only place that touches the write connection. One job at a time,
//! carried out and answered.

use std::sync::mpsc::Receiver;

use super::job::{Answer, Job, What};
use super::Store;

/// Serve jobs until every handle is gone.
pub(crate) fn run(work: Receiver<Job>, mut store: Store) {
    while let Ok(job) = work.recv() {
        let answer = match job.what {
            What::Submit(submit) => store.land(submit).map(Answer::Landed),
            What::Rewind(target) => store.rewind(target).map(Answer::Cut),
            What::DeleteSession(session) => store.delete_session(session).map(|()| Answer::Done),
            What::Relay(port) => {
                store.inlet.relay = Some(port);
                Ok(Answer::Done)
            }
        };
        let _ = job.back.send(answer);
    }
}
