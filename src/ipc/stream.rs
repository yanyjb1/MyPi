//! The server's half of one watch: frames become lines until either end gives up.

use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::ipc::contract::{Frame, Kind, Viewer};
use crate::ipc::line;
use crate::ipc::transport::Stream;
use crate::server::api::Api;

/// Write one session's stream to one connection until either end gives up.
pub(crate) fn pump(mut out: BufWriter<Stream>, rx: Receiver<Frame>, api: Arc<Api>, viewer: Viewer) {
    let mut session: Option<i64> = None;
    for frame in rx {
        if session.is_none() {
            session = frame.payload.get("session").and_then(|v| v.as_i64());
        }
        let bytes = match line::encode(&frame) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("mypi: stream: {e}");
                break;
            }
        };
        if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
    let unwatch = Frame::new(Kind::Post, "unwatch", session_json(session));
    let _ = api.receive(&viewer, unwatch);
}

fn session_json(session: Option<i64>) -> serde_json::Value {
    match session {
        Some(session) => serde_json::json!({ "session": session }),
        None => serde_json::json!({}),
    }
}
