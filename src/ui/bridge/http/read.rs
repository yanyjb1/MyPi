// One request off the wire: its method, its path, its heads and its body.

use std::io::{BufRead, BufReader, Read};
use std::net::TcpStream;

// A body larger than this is not a question anybody asks of a local bridge.
const MAX_BODY: usize = 8 * 1024 * 1024;

// One request, as read off the wire.
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
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

    // Whether this request asked to keep the connection open.
    pub fn keep_alive(&self) -> bool {
        !matches!(self.head("connection"), Some(v) if v.eq_ignore_ascii_case("close"))
    }

    // Whether this request asks to upgrade to WebSocket.
    pub fn is_websocket(&self) -> bool {
        self.head("upgrade")
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false)
    }
}

// Read one request off the existing buffered reader.
pub fn read_from(reader: &mut BufReader<TcpStream>) -> std::io::Result<Request> {
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
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target, None),
    };
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
        query,
        heads,
        body,
    })
}
