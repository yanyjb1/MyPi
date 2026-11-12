// The front end's half of the wire: one connection, read and written by two sides.

use std::io::{BufReader, BufWriter, Write};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};

use crate::ipc::contract::Frame;
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

    /// Say something without waiting for anything.
    pub fn send(&self, frame: Frame) -> Result<()> {
        let bytes = line::encode(&frame)?;
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        out.write_all(&bytes)?;
        out.flush()?;
        Ok(())
    }

    /// Whether the read half still lives. A frame found while checking is kept for {@link drain}.
    pub fn alive(&self) -> bool {
        let incoming = self.incoming();
        match incoming.try_recv() {
            Ok(frame) => {
                self.skipped().push(frame);
                true
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        }
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
            // The hello reply is the server's opening word: kept, so a front end can read it.
            let opened = Frame::new("local", "opened", frame.payload);
            if tx.send(opened).is_err() {
                break;
            }
            continue;
        }
        if tx.send(frame).is_err() {
            break;
        }
    }
}
