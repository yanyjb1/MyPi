// The server's writing half: one frame per line, until the queue closes or the socket dies.

use std::io::{BufWriter, Write};
use std::sync::mpsc::Receiver;

use crate::ipc::contract::Frame;
use crate::ipc::line;
use crate::ipc::transport::Stream;

/// Write one connection's queue until it closes.
pub(crate) fn write(mut out: BufWriter<Stream>, queue: Receiver<Frame>) {
    for frame in queue {
        let bytes = match line::encode(&frame) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("mypi: ipc write: {e}");
                break;
            }
        };
        if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}
