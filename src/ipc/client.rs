//! The front end's half of the wire: connect, ask, watch, leave.

use std::io::{BufReader, BufWriter, Write};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use anyhow::{Context, Result};

use crate::ipc::contract::{Frame, Kind, Viewer};
use crate::ipc::endpoint::Endpoint;
use crate::ipc::line::{self, Lines};
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
    /// Connect to the server at `endpoint`, and say hello.
    pub fn connect(endpoint: Endpoint, viewer: Viewer) -> Result<Client> {
        let conn = open(&endpoint)?;
        let client = Client {
            viewer,
            endpoint,
            conn: Mutex::new(conn),
        };
        client.hello()?;
        Ok(client)
    }

    /// One question, one answer. The reply's kind and name echo the request.
    pub fn ask(&self, kind: Kind, name: &str, payload: serde_json::Value) -> Result<Frame> {
        let request = Frame::new(kind, name, payload);
        let reply = self.exchange(&request)?;
        Ok(reply)
    }

    /// Watch one session: a connection of its own, streamed back as frames.
    pub fn watch(&self, session: i64) -> Result<Receiver<Frame>> {
        let (tx, rx) = mpsc::channel();
        let mut stream = open(&self.endpoint)?;
        let hello = hello_frame(&self.viewer);
        write_frame(&mut stream.out, &hello)?;
        let _ = read_frame(&mut stream.lines);
        let watch = Frame::new(
            Kind::Subscribe,
            "watch",
            serde_json::json!({ "session": session }),
        );
        write_frame(&mut stream.out, &watch)?;
        let answer = read_frame(&mut stream.lines)
            .context("the watch was not answered")?
            .context("the watch was refused")?;
        refuse(&watch, &answer)?;
        std::thread::Builder::new()
            .name("ipc-stream".to_string())
            .spawn(move || {
                loop {
                    match read_frame(&mut stream.lines) {
                        Ok(Some(frame)) => {
                            if tx.send(frame).is_err() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            })
            .context("cannot start the stream thread")?;
        Ok(rx)
    }

    /// Say that this front end is leaving.
    pub fn bye(&self) {
        let _ = self.exchange(&Frame::new(Kind::Register, "bye", serde_json::json!({})));
    }

    fn hello(&self) -> Result<()> {
        let hello = hello_frame(&self.viewer);
        self.exchange(&hello)?;
        Ok(())
    }

    fn exchange(&self, request: &Frame) -> Result<Frame> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        write_frame(&mut conn.out, request)?;
        let answer = read_frame(&mut conn.lines)
            .context("the server closed the connection")?
            .context("the server did not answer")?;
        refuse(request, &answer)?;
        Ok(answer)
    }
}

fn refuse(request: &Frame, answer: &Frame) -> Result<()> {
    if let Ok(error) =
        serde_json::from_value::<crate::ipc::contract::error::ErrorPayload>(answer.payload.clone())
    {
        anyhow::bail!(
            "{} {}: {}",
            request.name,
            error.error.code,
            error.error.message
        );
    }
    Ok(())
}

fn hello_frame(viewer: &Viewer) -> Frame {
    Frame::new(
        Kind::Register,
        "hello",
        serde_json::json!({ "viewer": viewer }),
    )
}

fn open(endpoint: &Endpoint) -> Result<Conn> {
    let stream = transport::connect(endpoint).context("cannot reach the server")?;
    let reader = stream.clone();
    Ok(Conn {
        lines: Lines::new(BufReader::new(reader)),
        out: BufWriter::new(stream),
    })
}

fn write_frame(out: &mut impl Write, frame: &Frame) -> Result<()> {
    let bytes = line::encode(frame)?;
    out.write_all(&bytes)?;
    out.flush()?;
    Ok(())
}

fn read_frame(lines: &mut Lines<BufReader<Stream>>) -> Result<Option<Frame>> {
    let Some(raw) = lines.next_line()? else {
        return Ok(None);
    };
    Ok(Some(line::decode(&raw)?))
}
