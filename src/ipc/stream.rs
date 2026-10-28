//! The server's half of one watch: a session's stream becomes lines.

use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::ipc::contract::{Answer, Event, Post, SessionWrite, Viewer};
use crate::ipc::frame::encode;
use crate::server::api::Api;

/// Write one session's stream to one connection until either end gives up.
pub(crate) fn pump(
    mut out: Box<dyn Write + Send>,
    rx: Receiver<Event>,
    api: Arc<Api>,
    viewer: Viewer,
    session: i64,
) {
    for item in rx {
        let line = match encode(&Answer::Stream { event: item }) {
            Ok(line) => line,
            Err(e) => {
                eprintln!("mypi: session {session} stream: {e}");
                break;
            }
        };
        if let Err(e) = out.write_all(&line).and_then(|()| out.flush()) {
            eprintln!("mypi: session {session} stream closed: {e}");
            break;
        }
    }
    let post = Post::Session {
        session,
        write: SessionWrite::Unwatch,
    };
    if let Err(e) = api.post(&viewer, post) {
        eprintln!("mypi: session {session} unwatch failed: {e:#}");
    }
}
