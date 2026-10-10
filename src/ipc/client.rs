//! The front end's half of the wire: connect, ask, watch, leave. The values
//! are the api's own (see `memo.md`, the IPC section).

use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use anyhow::{Context, Result, bail};

use crate::server::api::subscribe::Retention;
use crate::server::api::{Call, Opened, Post, PostDone, Reply, StreamItem, Subscribe, Viewer};
use crate::server::platform::ipc::{self, Cost, Stream};
use crate::server::platform::log;

use super::frame::{self, Lines};
use super::{Answer, Ask};

/// A front end's handle on a running server: one connection for questions.
pub struct Client {
    viewer: Viewer,
    cost: Cost,
    path: PathBuf,
    conn: Mutex<Conn>,
}

/// The question connection: a reader and a writer over one socket.
struct Conn {
    lines: Lines<BufReader<Stream>>,
    out: BufWriter<Stream>,
}

impl Client {
    /// Connect to the server at `path`, and register.
    pub fn connect(cost: Cost, path: &Path, viewer: Viewer) -> Result<Client> {
        let conn = open(cost, path)?;
        let client = Client {
            viewer,
            cost,
            path: path.to_path_buf(),
            conn: Mutex::new(conn),
        };
        client.send(&Ask::Hello { viewer })?;
        Ok(client)
    }

    /// One question, one answer.
    pub fn call(&self, call: Call) -> Result<Reply> {
        match self.ask(&Ask::Call { call })? {
            Answer::Reply { reply } => Ok(reply),
            _ => bail!("the server answered a question with something else"),
        }
    }

    /// One change of session state.
    pub fn post(&self, post: Post) -> Result<PostDone> {
        match self.ask(&Ask::Post { post })? {
            Answer::Done { done } => Ok(done),
            _ => bail!("the server answered a change with something else"),
        }
    }

    /// Open a round: one user message, already split into its three fields.
    pub fn submit(
        &self,
        session: i64,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> Result<Opened> {
        self.opened(Subscribe::Submit {
            session,
            prefix: prefix.map(str::to_string),
            text: text.to_string(),
            suffix: suffix.map(str::to_string),
        })
    }

    /// Open a compaction of one session's context (`/compact`).
    pub fn compact(
        &self,
        session: i64,
        retention: Retention,
        focus: Option<String>,
    ) -> Result<Opened> {
        self.opened(Subscribe::Compact {
            session,
            retention,
            focus,
        })
    }

    /// Watch one session: a connection of its own, streamed back on a channel.
    /// Dropping the channel closes it, which stops the watch.
    pub fn watch(&self, session: i64) -> Result<Receiver<StreamItem>> {
        let stream = ipc::connect(self.cost, &self.path)
            .with_context(|| format!("cannot open the stream for session {session}"))?;
        let mut out = BufWriter::new(stream.clone());
        write_line(
            &mut out,
            &Ask::Subscribe {
                subscribe: Subscribe::Watch {
                    viewer: self.viewer,
                    session,
                },
            },
        )?;

        let (tx, rx) = mpsc::channel();
        let mut lines = Lines::new(BufReader::new(stream));
        std::thread::Builder::new()
            .name(format!("ipc-stream-{session}"))
            .spawn(move || {
                while let Some(item) = read_item(&mut lines, session) {
                    if tx.send(item).is_err() {
                        // Nobody is listening: closing it stops the watch.
                        return;
                    }
                }
            })
            .context("cannot start the stream reader")?;
        Ok(rx)
    }

    /// Say that this front end is leaving. The connection ending says the same,
    /// so a front end that is killed needs no goodbye.
    pub fn bye(&self) {
        let _ = self.send(&Ask::Bye);
    }

    /// Open a stretch and take its verdict (`started` / `busy`).
    fn opened(&self, subscribe: Subscribe) -> Result<Opened> {
        match self.ask(&Ask::Subscribe { subscribe })? {
            Answer::Opened { opened } => Ok(opened),
            _ => bail!("the server answered an opening with something else"),
        }
    }

    /// Write one line and wait for the one answer it earns.
    fn ask(&self, ask: &Ask) -> Result<Answer> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        write_line(&mut conn.out, ask)?;
        let line = conn
            .lines
            .next_line()?
            .context("the server closed the connection")?;
        match frame::decode::<Answer>(line)? {
            Answer::Error { message } => bail!("{message}"),
            answer => Ok(answer),
        }
    }

    /// Write one line and do not wait for anything.
    fn send(&self, ask: &Ask) -> Result<()> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        write_line(&mut conn.out, ask)
    }
}

/// One connection to the server, ready to write one line and read one back.
fn open(cost: Cost, path: &Path) -> Result<Conn> {
    let stream =
        ipc::connect(cost, path).with_context(|| format!("no mypi server at {}", path.display()))?;
    Ok(Conn {
        lines: Lines::new(BufReader::new(stream.clone())),
        out: BufWriter::new(stream),
    })
}

/// One line out, flushed: the server reads a connection as it arrives.
fn write_line(out: &mut impl Write, ask: &Ask) -> Result<()> {
    out.write_all(&frame::encode(ask)?)?;
    out.flush()?;
    Ok(())
}

/// One piece off a stream connection. `None` = the stream is over.
fn read_item(lines: &mut Lines<BufReader<Stream>>, session: i64) -> Option<StreamItem> {
    loop {
        let line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => return None,
            Err(e) => {
                log::warn("ipc", format!("session {session} stream: {e}"));
                return None;
            }
        };
        match frame::decode::<Answer>(line) {
            Ok(Answer::Stream { item }) => return Some(item),
            Ok(Answer::Error { message }) => {
                log::warn("ipc", format!("session {session} stream: {message}"));
                return None;
            }
            // A stream connection carries the stream; anything else on it is
            // not ours to act on.
            Ok(_) => continue,
            Err(e) => {
                log::warn("ipc", format!("session {session} stream: {e}"));
                return None;
            }
        }
    }
}
