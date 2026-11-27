// WebSocket door: upgrades an HTTP connection to a full-duplex JSON frame stream.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tungstenite::Message;
use tungstenite::protocol::{Role, WebSocket};

use super::host;
use super::local;
use super::read::Request;
use super::respond;
use super::route;
use super::rpc;
use crate::ui::bridge::adapter::events;
use crate::ui::bridge::adapter::manifest::Adapter;
use crate::ui::bridge::manager::{Held, Hub};
use crate::ui::bridge::run::Options;

// Read-only stream wrapper so the blocking reader never writes auto-frames concurrently with the shared writer.
struct ReadOnlyStream(TcpStream);

impl Read for ReadOnlyStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for ReadOnlyStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

type WsWriter = Arc<Mutex<WebSocket<TcpStream>>>;

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
    let proto_hdr = match request.head("sec-websocket-protocol") {
        Some(p) => format!(
            "Sec-WebSocket-Protocol: {}\r\n",
            p.split(',').next().unwrap_or(p).trim()
        ),
        None => String::new(),
    };
    let handshake_resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept_key}\r\n{proto_hdr}\r\n"
    );
    stream.write_all(handshake_resp.as_bytes())?;
    stream.flush()?;

    let write_stream = stream.try_clone()?;
    let mut ws_reader =
        WebSocket::from_partially_read(ReadOnlyStream(stream), part, Role::Server, None);
    let ws_writer: WsWriter = Arc::new(Mutex::new(WebSocket::from_partially_read(
        write_stream,
        Vec::new(),
        Role::Server,
        None,
    )));
    let mut watcher: Option<(u64, Arc<Held>)> = None;

    loop {
        match ws_reader.read() {
            Ok(Message::Text(text)) => {
                if !handle_text(&text, adapter, hub, &ws_writer, &mut watcher) {
                    break;
                }
            }
            Ok(Message::Ping(data)) => {
                if send_msg(&ws_writer, Message::Pong(data)).is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => {
                if let Ok(mut w) = ws_writer.lock() {
                    let _ = w.close(None);
                }
                break;
            }
            Err(_) => break,
            _ => {}
        }
    }

    cleanup_watcher(&mut watcher);
    Ok(())
}

fn handle_text(
    text: &str,
    adapter: &Adapter,
    hub: &Hub,
    ws_writer: &WsWriter,
    watcher: &mut Option<(u64, Arc<Held>)>,
) -> bool {
    let body: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return send_json(ws_writer, respond::error("bad_request", &e.to_string())).is_ok();
        }
    };
    let id = body.get("id").filter(|v| !v.is_null()).cloned();
    let frame = match rpc::frame_of(adapter, &body) {
        Ok(f) => f,
        Err((_, err)) => return send_json(ws_writer, with_id(err, id.as_ref())).is_ok(),
    };
    if frame.kind == "subscribe" && frame.name == "watch" {
        let reply = start_watch(&frame, adapter, hub, ws_writer, watcher);
        return send_json(ws_writer, with_id(reply, id.as_ref())).is_ok();
    }
    if frame.kind == "host" {
        let res = match host::apply(hub, &frame) {
            Ok(answer) => answer,
            Err((_, answer)) => answer,
        };
        return send_json(ws_writer, with_id(res, id.as_ref())).is_ok();
    }
    if frame.kind == "local" {
        let res = if frame.name == "list" {
            local::list(&frame.payload)
        } else {
            respond::error("bad_request", "the door answers one local frame: `list`")
        };
        return send_json(ws_writer, with_id(res, id.as_ref())).is_ok();
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
    send_json(ws_writer, with_id(ans, id.as_ref())).is_ok()
}

fn start_watch(
    frame: &crate::ipc::contract::Frame,
    adapter: &Adapter,
    hub: &Hub,
    ws_writer: &WsWriter,
    watcher: &mut Option<(u64, Arc<Held>)>,
) -> Value {
    cleanup_watcher(watcher);
    let Some(session) = rpc::session_of(frame) else {
        return respond::error(
            "bad_request",
            "a frame that follows a conversation names `session`",
        );
    };
    let held = match hub.watch(session) {
        Ok(h) => h,
        Err(e) => return respond::error("upstream_closed", &e),
    };
    let (tx, rx) = mpsc::channel();
    let Some(lid) = held.listen(tx) else {
        return respond::error("upstream_closed", "failed to attach listener");
    };
    let writer = Arc::clone(ws_writer);
    let adapter = adapter.clone();
    let spawn_res = std::thread::Builder::new()
        .name(format!("bridge-ws-watch-{lid}"))
        .spawn(move || {
            for frame in rx {
                for piece in events::take(&adapter, frame) {
                    if send_json(&writer, serde_json::to_value(piece).unwrap_or(Value::Null))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
    if let Err(e) = spawn_res {
        held.unlisten(lid);
        return respond::error("internal_error", &e.to_string());
    }
    *watcher = Some((lid, held));
    json!({"ok": true})
}

fn send_json(ws_writer: &WsWriter, value: Value) -> Result<(), ()> {
    let text = serde_json::to_string(&value).unwrap_or_default();
    send_msg(ws_writer, Message::Text(text.into()))
}

fn send_msg(ws_writer: &WsWriter, msg: Message) -> Result<(), ()> {
    let mut w = ws_writer.lock().map_err(|_| ())?;
    w.send(msg).map_err(|_| ())
}

fn with_id(mut val: Value, id: Option<&Value>) -> Value {
    if let Some(id_val) = id {
        if let Value::Object(ref mut map) = val {
            map.insert("id".to_string(), id_val.clone());
        } else {
            return json!({"id": id_val, "payload": val});
        }
    }
    val
}

fn cleanup_watcher(watcher: &mut Option<(u64, Arc<Held>)>) {
    if let Some((id, held)) = watcher.take() {
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
