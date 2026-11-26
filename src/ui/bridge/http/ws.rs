// WebSocket door: upgrades an HTTP connection to a full-duplex JSON frame stream.

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::Value;
use tungstenite::Message;
use tungstenite::protocol::{Role, WebSocket};

use super::host;
use super::read::Request;
use super::respond;
use super::route;
use super::rpc;
use crate::ipc::contract::Frame;
use crate::ui::bridge::adapter::events;
use crate::ui::bridge::adapter::manifest::Adapter;
use crate::ui::bridge::manager::{Held, Hub};
use crate::ui::bridge::run::Options;

pub fn serve_ws(
    mut stream: TcpStream,
    peer: SocketAddr,
    request: &Request,
    part: Vec<u8>,
    options: &Options,
    hub: &Hub,
) -> std::io::Result<()> {
    let Some(adapter) = find_adapter(request, options) else {
        let answer = respond::error(
            "unknown_adapter",
            "no matching adapter found in adapters.yml",
        );
        return respond::respond_json(&mut stream, "403 Forbidden", &answer, false);
    };
    if let Err(why) = route::admitted(adapter, peer) {
        let answer = respond::error("remote_refused", why);
        return respond::respond_json(&mut stream, "403 Forbidden", &answer, false);
    }
    let Some(key) = request.head("sec-websocket-key") else {
        let answer = respond::error("bad_request", "missing sec-websocket-key header");
        return respond::respond_json(&mut stream, "400 Bad Request", &answer, false);
    };
    let accept_key = tungstenite::handshake::derive_accept_key(key.as_bytes());
    let subprotocol = request.head("sec-websocket-protocol");
    let proto_hdr = match subprotocol {
        Some(p) => format!(
            "Sec-WebSocket-Protocol: {}\r\n",
            p.split(',').next().unwrap_or(p).trim()
        ),
        None => String::new(),
    };
    let handshake_resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept_key}\r\n\
         {proto_hdr}\r\n"
    );
    stream.write_all(handshake_resp.as_bytes())?;
    stream.flush()?;

    // Read timeout allows the worker loop to alternate between incoming messages and outgoing events.
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;

    let mut ws = WebSocket::from_partially_read(stream, part, Role::Server, None);
    let mut watcher: Option<(i64, u64, Arc<Held>, Receiver<Frame>)> = None;

    loop {
        if let Some((_, _, _, ref rx)) = watcher {
            while let Ok(frame) = rx.try_recv() {
                for piece in events::take(adapter, frame) {
                    let text = serde_json::to_string(&piece).unwrap_or_default();
                    if ws.send(Message::Text(text.into())).is_err() {
                        cleanup_watcher(&mut watcher);
                        return Ok(());
                    }
                }
            }
        }

        match ws.read() {
            Ok(Message::Text(text)) => {
                let body: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        let err = respond::error("bad_request", &e.to_string());
                        let _ = ws.send(Message::Text(
                            serde_json::to_string(&err).unwrap_or_default().into(),
                        ));
                        continue;
                    }
                };
                let frame = match rpc::frame_of(adapter, &body) {
                    Ok(f) => f,
                    Err((_, err)) => {
                        let _ = ws.send(Message::Text(
                            serde_json::to_string(&err).unwrap_or_default().into(),
                        ));
                        continue;
                    }
                };
                if frame.kind == "subscribe" && frame.name == "watch" {
                    cleanup_watcher(&mut watcher);
                    if let Some(session) = rpc::session_of(&frame)
                        && let Ok(held) = hub.get(session)
                    {
                        let (tx, rx) = mpsc::channel();
                        if let Some(id) = held.listen(tx) {
                            watcher = Some((session, id, held, rx));
                        }
                    }
                    continue;
                }
                if frame.kind == "host" {
                    let res = if frame.name == "list" {
                        host::list(&frame.payload)
                    } else {
                        respond::error("bad_request", "the door answers one host frame: `list`")
                    };
                    let _ = ws.send(Message::Text(
                        serde_json::to_string(&res).unwrap_or_default().into(),
                    ));
                    continue;
                }
                let held = match rpc::session_of(&frame) {
                    Some(s) => hub.get(s),
                    None => hub.bare(),
                };
                let ans = match held {
                    Ok(h) => match h.ask(frame) {
                        Ok(ans_frame) => rpc::answer(&ans_frame),
                        Err(e) => respond::error("upstream_silent", &e),
                    },
                    Err(e) => respond::error("upstream_closed", &e),
                };
                if ws
                    .send(Message::Text(
                        serde_json::to_string(&ans).unwrap_or_default().into(),
                    ))
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::Ping(data)) => {
                if ws.send(Message::Pong(data)).is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => break,
            _ => {}
        }
    }

    cleanup_watcher(&mut watcher);
    Ok(())
}

fn cleanup_watcher(watcher: &mut Option<(i64, u64, Arc<Held>, Receiver<Frame>)>) {
    if let Some((_, id, held, _)) = watcher.take() {
        held.unlisten(id);
    }
}

fn find_adapter<'a>(request: &Request, options: &'a Options) -> Option<&'a Adapter> {
    if let Some(named) = request.head(route::HEADER)
        && let Some(adapter) = options.manifest.adapter(named)
    {
        return Some(adapter);
    }
    if let Some(query) = &request.query {
        for part in query.split('&') {
            if let Some((k, v)) = part.split_once('=')
                && k == "adapter"
                && let Some(adapter) = options.manifest.adapter(v)
            {
                return Some(adapter);
            }
        }
    }
    if let Some(subproto) = request.head("sec-websocket-protocol") {
        for proto in subproto.split(',') {
            let proto = proto.trim();
            if let Some(adapter) = options.manifest.adapter(proto) {
                return Some(adapter);
            }
        }
    }
    options.manifest.adapter("webui")
}
