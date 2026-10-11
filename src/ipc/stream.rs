//! The server's half of one watch: a session's stream becomes lines, at the
//! granularity `config.server.request.streaming` asks for.

use std::io::Write;
use std::sync::mpsc::Receiver;

use crate::server::api::{Post, StreamItem, Viewer};
use crate::server::config::{Config, StreamMode};
use crate::server::platform::log;

use super::frame::encode;
use super::Answer;

/// Write one session's stream to one connection until either end gives up.
/// The watch is dropped on the way out.
pub(crate) fn pump(
    mut out: Box<dyn Write + Send>,
    rx: Receiver<StreamItem>,
    cfg: Config,
    viewer: Viewer,
    session: i64,
) {
    let mode = cfg.app.server.request.streaming;
    for item in rx {
        // `buffered` hands out no half-written text: the durable version
        // arrives as blocks in either setting.
        if mode == StreamMode::Buffered
            && matches!(
                item,
                StreamItem::Live { .. } | StreamItem::ToolOutput { .. }
            )
        {
            continue;
        }
        let line = match encode(&Answer::Stream { item }) {
            Ok(line) => line,
            Err(e) => {
                log::warn("ipc", format!("session {session} stream: {e}"));
                break;
            }
        };
        if let Err(e) = out.write_all(&line).and_then(|()| out.flush()) {
            // The front end is gone; that is the answer, not an error.
            log::info("ipc", format!("session {session} stream closed: {e}"));
            break;
        }
    }
    if let Err(e) = crate::server::api::post::apply(&cfg, Post::Unwatch { viewer, session }) {
        log::warn("ipc", format!("session {session} unwatch failed: {e:#}"));
    }
}
