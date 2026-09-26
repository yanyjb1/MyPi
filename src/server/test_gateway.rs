//! Test support: a scripted HTTP gateway.
//!
//! The server's real behavior is "send this to a gateway, decode the stream,
//! persist a round". Testing that honestly needs a gateway — a socket that
//! answers with canned SSE and captures what it was asked. Everything here is
//! `#[cfg(test)]`: a fake that no production path can reach.

use std::io::{Read, Write};

/// `data:` lines carrying content chunks, then `finish_reason: stop`, then a
/// standalone usage chunk (the `stream_options.include_usage` shape).
pub fn sse(chunks: &[&str]) -> String {
    let mut b = String::new();
    for c in chunks {
        let quoted = serde_json::to_string(c).unwrap();
        b.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{quoted}}}}}]}}\n\n"
        ));
    }
    b.push_str("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
    b.push_str(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n",
    );
    b.push_str("data: [DONE]\n\n");
    b
}

/// One response that asks for a tool call, then one that finishes the round.
pub fn tool_script(args: &str) -> Vec<String> {
    tool_script_named("read", args)
}

/// As [`tool_script`], for a specific tool.
pub fn tool_script_named(name: &str, args: &str) -> Vec<String> {
    let first = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"我先看看\",\"tool_calls\":[{{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{{\"name\":{},\"arguments\":{}}}}}]}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n",
        serde_json::to_string(name).unwrap(),
        serde_json::to_string(args).unwrap()
    );
    vec![first, sse(&["读完了"])]
}

/// A gateway that answers `responses` in order (one connection each) and
/// returns the base URL plus a handle yielding every request body it saw.
pub fn fake_gateway_status(
    responses: Vec<(u16, String)>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in responses {
            let Ok((mut sock, _)) = listener.accept() else {
                break;
            };
            seen.push(read_request(&mut sock));
            let reason = if status == 200 { "OK" } else { "ERR" };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.flush();
        }
        seen
    });
    (format!("http://{addr}/v1"), h)
}

/// All-200 convenience wrapper.
pub fn fake_gateway(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    fake_gateway_status(responses.into_iter().map(|b| (200, b)).collect())
}

/// Read one HTTP request (headers + `Content-Length` body) off the socket.
pub fn read_request(sock: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    while let Ok(n) = sock.read(&mut tmp) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        while buf.len() < pos + 4 + len {
            let Ok(n) = sock.read(&mut tmp) else { break };
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        return String::from_utf8_lossy(&buf[pos + 4..]).to_string();
    }
    String::new()
}
