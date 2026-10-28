//! The front end's half of the wire: connect, ask, watch, leave.

use std::io::{BufReader, BufWriter, Write};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use anyhow::{Context, Result, bail};

use crate::ipc::contract::{
    Answer, Ask, Call, Event, Opened, PROTOCOL_VERSION, Post, PostDone, Reply, Retention,
    Subscribe, Viewer,
};
use crate::ipc::endpoint::Endpoint;
use crate::ipc::frame::{self, Lines};
use crate::ipc::transport::{self, Stream};

/// A front end's handle on a running server: one connection for questions.
pub struct Client {
    viewer: Viewer,
    endpoint: Endpoint,
    conn: Mutex<Conn>,
}

/// The question connection: a reader and a writer over one socket.
struct Conn {
    lines: Lines<BufReader<Stream>>,
    out: BufWriter<Stream>,
}

impl Client {
    /// Connect to the server at `endpoint`, and register.
    pub fn connect(endpoint: Endpoint, viewer: Viewer) -> Result<Client> {
        let conn = open(&endpoint)?;
        let client = Client {
            viewer: viewer.clone(),
            endpoint,
            conn: Mutex::new(conn),
        };
        client.send(&Ask::Hello {
            viewer,
            protocol: PROTOCOL_VERSION,
        })?;
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
    pub fn watch(&self, session: i64) -> Result<Receiver<Event>> {
        let stream = transport::connect(&self.endpoint)
            .with_context(|| format!("cannot open the stream for session {session}"))?;
        let mut out = BufWriter::new(stream.clone());
        write_line(
            &mut out,
            &Ask::Hello {
                viewer: self.viewer.clone(),
                protocol: PROTOCOL_VERSION,
            },
        )?;
        write_line(
            &mut out,
            &Ask::Subscribe {
                subscribe: Subscribe::Watch { session },
            },
        )?;

        let (tx, rx) = mpsc::channel();
        let mut lines = Lines::new(BufReader::new(stream));
        std::thread::Builder::new()
            .name(format!("ipc-stream-{session}"))
            .spawn(move || {
                while let Some(item) = read_item(&mut lines, session) {
                    if tx.send(item).is_err() {
                        return;
                    }
                }
            })
            .context("cannot start the stream reader")?;
        Ok(rx)
    }

    /// Say that this front end is leaving.
    pub fn bye(&self) {
        let _ = self.send(&Ask::Bye);
    }

    fn opened(&self, subscribe: Subscribe) -> Result<Opened> {
        match self.ask(&Ask::Subscribe { subscribe })? {
            Answer::Opened { opened } => Ok(opened),
            _ => bail!("the server answered an opening with something else"),
        }
    }

    fn ask(&self, ask: &Ask) -> Result<Answer> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        write_line(&mut conn.out, ask)?;
        let line = conn
            .lines
            .next_line()?
            .context("the server closed the connection")?;
        match frame::decode::<Answer>(line)? {
            Answer::Error { code, message } => bail!("{code:?}: {message}"),
            answer => Ok(answer),
        }
    }

    fn send(&self, ask: &Ask) -> Result<()> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        write_line(&mut conn.out, ask)
    }
}

fn open(endpoint: &Endpoint) -> Result<Conn> {
    let stream =
        transport::connect(endpoint).with_context(|| format!("no mypi server at {endpoint:?}"))?;
    Ok(Conn {
        lines: Lines::new(BufReader::new(stream.clone())),
        out: BufWriter::new(stream),
    })
}

fn write_line(out: &mut impl Write, ask: &Ask) -> Result<()> {
    out.write_all(&frame::encode(ask)?)?;
    out.flush()?;
    Ok(())
}

fn read_item(lines: &mut Lines<BufReader<Stream>>, session: i64) -> Option<Event> {
    loop {
        let line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => return None,
            Err(e) => {
                eprintln!("mypi: session {session} stream: {e}");
                return None;
            }
        };
        match frame::decode::<Answer>(line) {
            Ok(Answer::Stream { event }) => return Some(event),
            Ok(Answer::Error { code, message }) => {
                eprintln!("mypi: session {session} stream: {code:?}: {message}");
                return None;
            }
            Ok(_) => continue,
            Err(e) => {
                eprintln!("mypi: session {session} stream: {e}");
                return None;
            }
        }
    }
}
