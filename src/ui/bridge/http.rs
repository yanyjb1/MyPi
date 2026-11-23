// Just enough HTTP to take one JSON request and answer it, or hold one stream open.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use serde_json::Value;

use crate::ipc::contract::Frame;

// A body larger than this is not a question anybody asks of a local bridge.
const MAX_BODY: usize = 8 * 1024 * 1024;

// One request, as read off the wire.
pub struct Request {
    pub method: String,
    pub path: String,
    heads: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    // One head's value, by its lowercased name.
    pub fn head(&self, name: &str) -> Option<&str> {
        self.heads
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }
}

// Read one request head and its body; a peer that says nothing is not an error worth a reply.
pub fn read(stream: &TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut start = String::new();
    if reader.read_line(&mut start)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "closed before a request",
        ));
    }
    let mut parts = start.trim_end().split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let path = target.split('?').next().unwrap_or_default().to_string();
    let mut heads = Vec::new();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            length = value.parse().unwrap_or(0);
        }
        heads.push((name, value));
    }
    if length > MAX_BODY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "body over the limit",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        heads,
        body,
    })
}

// One whole answer: the door closes right after it.
pub fn respond(
    stream: &mut TcpStream,
    status: &str,
    ctype: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

pub fn respond_json(stream: &mut TcpStream, status: &str, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    respond(stream, status, "application/json; charset=utf-8", &body)
}

// The answer that never ends on its own: the length is the connection's own end, so none is said.
pub fn begin_stream(stream: &mut TcpStream) -> std::io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\nConnection: close\r\n\r\n";
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

pub fn event(stream: &mut TcpStream, frame: &Frame) -> std::io::Result<()> {
    let line = serde_json::to_string(frame).unwrap_or_default();
    stream.write_all(format!("data: {line}\n\n").as_bytes())?;
    stream.flush()
}

// The one error shape the door itself speaks.
pub fn error(code: &str, message: &str) -> Value {
    serde_json::json!({"error": {"code": code, "message": message}})
}
