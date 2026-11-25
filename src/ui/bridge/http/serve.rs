// One connection answered: its file, one frame and its answer, or one conversation's stream.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc;

use crate::ipc::contract::Frame;

use super::host;
use super::read;
use super::respond;
use super::route;
use super::rpc;
use crate::ui::bridge::adapter::events;
use crate::ui::bridge::adapter::manifest::Adapter;
use crate::ui::bridge::manager::Hub;
use crate::ui::bridge::run::Options;

pub fn serve(mut stream: TcpStream, options: &Options, hub: &Hub) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    let request = match read::read(&stream) {
        Ok(request) => request,
        Err(e) => {
            let answer = respond::error("bad_request", &e.to_string());
            return respond::respond_json(&mut stream, "400 Bad Request", &answer);
        }
    };
    if request.method == "GET" {
        let name = request.path.trim_start_matches('/');
        return match options.webui.file(name) {
            Some((kind, body)) => respond::respond(&mut stream, "200 OK", kind, &body),
            None => respond::respond_json(
                &mut stream,
                "404 Not Found",
                &respond::error("no_route", "no file of that name in the webui directory"),
            ),
        };
    }
    if request.method != "POST" {
        return respond::respond_json(
            &mut stream,
            "405 Method Not Allowed",
            &respond::error(
                "no_route",
                "the bridge takes GET for a file and POST for a frame",
            ),
        );
    }
    let Some(named) = request.head(route::HEADER) else {
        let answer = respond::error(
            "unknown_adapter",
            "no X-Bridge-Adapter head names this request",
        );
        return respond::respond_json(&mut stream, "403 Forbidden", &answer);
    };
    let Some(adapter) = options.manifest.adapter(named) else {
        let answer = respond::error("unknown_adapter", "no adapter of that name in adapters.yml");
        return respond::respond_json(&mut stream, "403 Forbidden", &answer);
    };
    if let Err(why) = route::admitted(adapter, peer) {
        let answer = respond::error("remote_refused", why);
        return respond::respond_json(&mut stream, "403 Forbidden", &answer);
    }
    let body: serde_json::Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(e) => {
            let answer = respond::error("bad_request", &e.to_string());
            return respond::respond_json(&mut stream, "400 Bad Request", &answer);
        }
    };
    let frame = match rpc::frame_of(adapter, &body) {
        Ok(frame) => frame,
        Err((status, answer)) => {
            return respond::respond_json(&mut stream, &status_line(status), &answer);
        }
    };
    // The door's own kind: this machine's directories, answered here and never sent upstream.
    if frame.kind == "host" {
        if frame.name != "list" {
            let answer = respond::error("bad_request", "the door answers one host frame: `list`");
            return respond::respond_json(&mut stream, "400 Bad Request", &answer);
        }
        return respond::respond_json(&mut stream, "200 OK", &host::list(&frame.payload));
    }
    // A frame that follows a conversation turns this connection into that conversation's stream.
    if rpc::is_watch(&frame) {
        return follow(stream, adapter, hub, &frame);
    }
    // Following nothing is leaving, and leaving is closing the connection: this frame is no question.
    if frame.kind == "subscribe" && frame.name == "watch" {
        let answer = respond::error(
            "bad_request",
            "a frame that follows a conversation names `session`; close the connection to stop",
        );
        return respond::respond_json(&mut stream, "400 Bad Request", &answer);
    }
    ask(stream, hub, frame)
}

// One frame to the server, its answer back. The adapter's own rules are read before anything leaves.
fn ask(mut stream: TcpStream, hub: &Hub, frame: Frame) -> std::io::Result<()> {
    let held = match rpc::session_of(&frame) {
        Some(session) => hub.get(session),
        None => hub.bare(),
    };
    let held = match held {
        Ok(held) => held,
        Err(e) => {
            let answer = respond::error("upstream_closed", &e);
            return respond::respond_json(&mut stream, "502 Bad Gateway", &answer);
        }
    };
    match held.ask(frame) {
        Ok(answer) => respond::respond_json(&mut stream, "200 OK", &rpc::answer(&answer)),
        Err(e) => {
            let answer = respond::error("upstream_silent", &e);
            respond::respond_json(&mut stream, "504 Gateway Timeout", &answer)
        }
    }
}

// One conversation's frames, shaped by what this adapter said it wants, until the peer leaves.
fn follow(
    mut stream: TcpStream,
    adapter: &Adapter,
    hub: &Hub,
    frame: &Frame,
) -> std::io::Result<()> {
    let Some(session) = rpc::session_of(frame) else {
        let answer = respond::error(
            "bad_request",
            "a frame that follows a conversation names `session`",
        );
        return respond::respond_json(&mut stream, "400 Bad Request", &answer);
    };
    let held = match hub.get(session) {
        Ok(held) => held,
        Err(e) => {
            let answer = respond::error("upstream_closed", &e);
            return respond::respond_json(&mut stream, "502 Bad Gateway", &answer);
        }
    };
    let (tx, frames) = mpsc::channel();
    respond::begin_stream(&mut stream)?;
    let Some(id) = held.listen(tx) else {
        return Ok(());
    };
    // A peer that leaves just ends its socket: one read that blocks is the only way to hear it, and only it.
    {
        let held = Arc::clone(&held);
        let mut watch = stream.try_clone()?;
        std::thread::Builder::new()
            .name(format!("bridge-follow-{id}"))
            .spawn(move || {
                let mut sink = [0u8; 64];
                while matches!(watch.read(&mut sink), Ok(n) if n > 0) {}
                held.unlisten(id);
            })
            .map_err(std::io::Error::other)?;
    }
    for frame in frames {
        for piece in events::take(adapter, frame) {
            if respond::event(&mut stream, &piece).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn status_line(status: u16) -> String {
    match status {
        400 => "400 Bad Request",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        502 => "502 Bad Gateway",
        504 => "504 Gateway Timeout",
        _ => "500 Internal Server Error",
    }
    .to_string()
}
