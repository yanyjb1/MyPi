// One connection answered: its file, one frame and its answer, or one conversation's stream.

use std::io::{BufReader, Read};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use crate::ipc::contract::Frame;

use super::host;
use super::read::{self, Request};
use super::respond;
use super::route;
use super::rpc;
use super::ws;
use crate::ui::bridge::adapter::events;
use crate::ui::bridge::adapter::manifest::Adapter;
use crate::ui::bridge::manager::Hub;
use crate::ui::bridge::run::Options;

pub fn serve(stream: TcpStream, options: &Options, hub: &Hub) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    let mut reader = BufReader::new(stream);

    let first = match read::read_from(&mut reader) {
        Ok(request) => request,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(());
            }
            let answer = respond::error("bad_request", &e.to_string());
            return respond::respond_json(reader.get_mut(), "400 Bad Request", &answer, false);
        }
    };

    if first.is_websocket() {
        let part = reader.buffer().to_vec();
        let raw = reader.into_inner();
        return ws::serve_ws(raw, peer, &first, part, options, hub);
    }

    let mut request = first;
    loop {
        let keep_alive = request.keep_alive();
        let is_stream = handle_request(reader.get_mut(), peer, &request, options, hub, keep_alive)?;
        if is_stream || !keep_alive {
            break;
        }

        // Keep-alive idle timeout waiting for subsequent requests from the same client.
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(15)))?;
        request = match read::read_from(&mut reader) {
            Ok(req) => req,
            Err(_) => break,
        };
        reader.get_ref().set_read_timeout(None)?;
    }
    Ok(())
}

// Answer one request on the wire; returns true when the connection transitioned into a stream.
fn handle_request(
    stream: &mut TcpStream,
    peer: std::net::SocketAddr,
    request: &Request,
    options: &Options,
    hub: &Hub,
    keep_alive: bool,
) -> std::io::Result<bool> {
    if request.method == "GET" {
        let name = request.path.trim_start_matches('/');
        match options.webui.file(name) {
            Some((kind, body)) => respond::respond(stream, "200 OK", kind, &body, keep_alive)?,
            None => respond::respond_json(
                stream,
                "404 Not Found",
                &respond::error("no_route", "no file of that name in the webui directory"),
                keep_alive,
            )?,
        };
        return Ok(false);
    }
    if request.method != "POST" {
        respond::respond_json(
            stream,
            "405 Method Not Allowed",
            &respond::error(
                "no_route",
                "the bridge takes GET for a file and POST for a frame",
            ),
            false,
        )?;
        return Ok(false);
    }
    let Some(named) = request.head(route::HEADER) else {
        let answer = respond::error(
            "unknown_adapter",
            "no X-Bridge-Adapter head names this request",
        );
        respond::respond_json(stream, "403 Forbidden", &answer, false)?;
        return Ok(false);
    };
    let Some(adapter) = options.manifest.adapter(named) else {
        let answer = respond::error("unknown_adapter", "no adapter of that name in adapters.yml");
        respond::respond_json(stream, "403 Forbidden", &answer, false)?;
        return Ok(false);
    };
    if let Err(why) = route::admitted(adapter, peer) {
        let answer = respond::error("remote_refused", why);
        respond::respond_json(stream, "403 Forbidden", &answer, false)?;
        return Ok(false);
    }
    let body: serde_json::Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(e) => {
            let answer = respond::error("bad_request", &e.to_string());
            respond::respond_json(stream, "400 Bad Request", &answer, false)?;
            return Ok(false);
        }
    };
    let frame = match rpc::frame_of(adapter, &body) {
        Ok(frame) => frame,
        Err((status, answer)) => {
            respond::respond_json(stream, &status_line(status), &answer, keep_alive)?;
            return Ok(false);
        }
    };
    if frame.kind == "host" {
        if frame.name != "list" {
            let answer = respond::error("bad_request", "the door answers one host frame: `list`");
            respond::respond_json(stream, "400 Bad Request", &answer, keep_alive)?;
            return Ok(false);
        }
        respond::respond_json(stream, "200 OK", &host::list(&frame.payload), keep_alive)?;
        return Ok(false);
    }
    if rpc::is_watch(&frame) {
        follow(stream, adapter, hub, &frame)?;
        return Ok(true);
    }
    if frame.kind == "subscribe" && frame.name == "watch" {
        let answer = respond::error(
            "bad_request",
            "a frame that follows a conversation names `session`; close the connection to stop",
        );
        respond::respond_json(stream, "400 Bad Request", &answer, keep_alive)?;
        return Ok(false);
    }
    ask(stream, hub, frame, keep_alive)?;
    Ok(false)
}

fn ask(stream: &mut TcpStream, hub: &Hub, frame: Frame, keep_alive: bool) -> std::io::Result<()> {
    let held = match rpc::session_of(&frame) {
        Some(session) => hub.get(session),
        None => hub.bare(),
    };
    let held = match held {
        Ok(held) => held,
        Err(e) => {
            let answer = respond::error("upstream_closed", &e);
            return respond::respond_json(stream, "502 Bad Gateway", &answer, keep_alive);
        }
    };
    match held.ask(frame) {
        Ok(answer) => respond::respond_json(stream, "200 OK", &rpc::answer(&answer), keep_alive),
        Err(e) => {
            let answer = respond::error("upstream_silent", &e);
            respond::respond_json(stream, "504 Gateway Timeout", &answer, keep_alive)
        }
    }
}

fn follow(
    stream: &mut TcpStream,
    adapter: &Adapter,
    hub: &Hub,
    frame: &Frame,
) -> std::io::Result<()> {
    let Some(session) = rpc::session_of(frame) else {
        let answer = respond::error(
            "bad_request",
            "a frame that follows a conversation names `session`",
        );
        return respond::respond_json(stream, "400 Bad Request", &answer, false);
    };
    let held = match hub.get(session) {
        Ok(held) => held,
        Err(e) => {
            let answer = respond::error("upstream_closed", &e);
            return respond::respond_json(stream, "502 Bad Gateway", &answer, false);
        }
    };
    let (tx, frames) = mpsc::channel();
    respond::begin_stream(stream)?;
    let Some(id) = held.listen(tx) else {
        return Ok(());
    };
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
            if respond::event(stream, &piece).is_err() {
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
