// The front end's half of the wire: one connection, read and written by two sides.

use std::io::{BufReader, BufWriter, Write};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result, bail};

use crate::ipc::contract::{Frame, error};
use crate::ipc::endpoint::Endpoint;
use crate::ipc::line::{self, Lines};
use crate::ipc::transport::{self, Stream};

/// A front end's handle on a running server. Replies and pushes share one connection.
pub struct Client {
    out: Mutex<BufWriter<Stream>>,
    incoming: Mutex<Receiver<Frame>>,
    skipped: Mutex<Vec<Frame>>,
}

impl Client {
    /// Connect, say hello, and keep reading from then on.
    pub fn connect(endpoint: &Endpoint, viewer: &serde_json::Value) -> Result<Client> {
        let stream = transport::connect(endpoint).context("cannot reach the server")?;
        let reader = stream.clone();
        let out = Mutex::new(BufWriter::new(stream));
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("ipc-read".to_string())
            .spawn(move || read(Lines::new(BufReader::new(reader)), tx))
            .context("cannot start the read thread")?;
        let client = Client {
            out,
            incoming: Mutex::new(rx),
            skipped: Mutex::new(Vec::new()),
        };
        client.send(Frame::new("register", "hello", viewer.clone()))?;
        Ok(client)
    }

    /// One question, its answer. Frames that are not the answer are kept for [`drain`].
    pub fn ask(&self, kind: &str, name: &str, payload: serde_json::Value) -> Result<Frame> {
        self.send(Frame::new(kind, name, payload))?;
        let incoming = self.incoming();
        loop {
            let Ok(frame) = incoming.recv() else {
                bail!("the server closed the connection");
            };
            if frame.kind != kind || frame.name != name {
                self.skipped().push(frame);
                continue;
            }
            refuse(&frame)?;
            return Ok(frame);
        }
    }

    /// Say something without waiting for anything.
    pub fn send(&self, frame: Frame) -> Result<()> {
        let bytes = line::encode(&frame)?;
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        out.write_all(&bytes)?;
        out.flush()?;
        Ok(())
    }

    /// Everything the server pushed that no question claimed.
    pub fn drain(&self) -> Vec<Frame> {
        let mut out = std::mem::take(&mut *self.skipped());
        let incoming = self.incoming();
        while let Ok(frame) = incoming.try_recv() {
            out.push(frame);
        }
        out
    }

    fn incoming(&self) -> MutexGuard<'_, Receiver<Frame>> {
        self.incoming.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn skipped(&self) -> MutexGuard<'_, Vec<Frame>> {
        self.skipped.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The read half: every frame goes to the channel.
fn read(mut lines: Lines<BufReader<Stream>>, tx: mpsc::Sender<Frame>) {
    loop {
        let raw = match lines.next_line() {
            Ok(Some(raw)) => raw,
            _ => break,
        };
        let frame: Frame = match line::decode(&raw) {
            Ok(frame) => frame,
            Err(_) => break,
        };
        if frame.name == "opened" {
            continue;
        }
        if tx.send(frame).is_err() {
            break;
        }
    }
}

fn refuse(answer: &Frame) -> Result<()> {
    if let Ok(err) = serde_json::from_value::<error::ErrorPayload>(answer.payload.clone()) {
        bail!("{} {}: {}", answer.name, err.error.code, err.error.message);
    }
    Ok(())
}
