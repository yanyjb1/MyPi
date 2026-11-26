// The words of HTTP: one answer written whole, one stream held open, and the door's error shape.

use std::io::Write;
use std::net::TcpStream;

use serde_json::Value;

use crate::ipc::contract::Frame;

// One whole answer: sent with connection keep-alive or close depending on client request.
pub fn respond(
    stream: &mut TcpStream,
    status: &str,
    ctype: &str,
    body: &[u8],
    keep_alive: bool,
) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: {conn}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

pub fn respond_json(
    stream: &mut TcpStream,
    status: &str,
    value: &Value,
    keep_alive: bool,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    respond(
        stream,
        status,
        "application/json; charset=utf-8",
        &body,
        keep_alive,
    )
}

// The answer that never ends on its own: the length is the connection's own end, so none is said.
pub fn begin_stream(stream: &mut TcpStream) -> std::io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Cache-Control: no-store\r\nConnection: close\r\n\r\n";
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
