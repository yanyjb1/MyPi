// The bridge's loop: bind the door, take every request, start the adapters' own programs.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

use super::events;
use super::http;
use super::hub::Hub;
use super::manifest::{Adapter, Manifest};
use super::route;
use super::rpc;
use super::web;

// How long a conversation nothing listens to and nothing runs stays held before it is let go.
const IDLE: Duration = Duration::from_secs(10 * 60);

// How often the table is swept for conversations that have gone quiet.
const SWEEP: Duration = Duration::from_secs(30);

// What the bridge was started with: the server, the adapter file, the page, and the network switch.
pub struct Options {
    pub endpoint: Endpoint,
    pub manifest: Manifest,
    pub webui: web::Webui,
    pub lan: bool,
}

pub fn run(options: Options) -> Result<()> {
    let options = Arc::new(options);
    let hub = Arc::new(Hub::new(options.endpoint.clone()));
    let stop = Arc::new(AtomicBool::new(false));
    // One connection that follows nothing keeps the server alive between requests.
    hub.bare()
        .map_err(|e| anyhow::anyhow!("cannot reach the server: {e}"))?;

    let addr = SocketAddr::new(
        if options.lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        },
        options.manifest.port,
    );
    let listener = TcpListener::bind(addr).with_context(|| format!("cannot listen on {addr}"))?;
    println!("mypi: bridge on http://{addr}/");
    println!(
        "mypi: webui at that same address (files in {})",
        options.webui.dir().display()
    );

    let children = start_adapters(&options);

    {
        let hub = Arc::clone(&hub);
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("bridge-sweep".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(SWEEP);
                    hub.sweep(IDLE);
                }
            })
            .context("cannot start the sweep")?;
    }

    {
        let hub = Arc::clone(&hub);
        let options = Arc::clone(&options);
        std::thread::Builder::new()
            .name("bridge-accept".to_string())
            .spawn(move || accept(listener, options, hub))
            .context("cannot start the accept loop")?;
    }

    // The accept loop never returns, so only a signal closes the door: programs first, then every conversation.
    wait_for_stop();
    stop.store(true, Ordering::Relaxed);
    for child in children {
        end(child);
    }
    hub.stop();
    Ok(())
}

// The door answers until the process is asked to stop.
fn accept(listener: TcpListener, options: Arc<Options>, hub: Arc<Hub>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("mypi: bridge accept: {e}");
                continue;
            }
        };
        let hub = Arc::clone(&hub);
        let options = Arc::clone(&options);
        let _ = std::thread::Builder::new()
            .name("bridge-conn".to_string())
            .spawn(move || {
                if let Err(e) = serve(stream, &options, &hub) {
                    eprintln!("mypi: bridge connection: {e}");
                }
            });
    }
}

// One request: a file, one frame and its answer, or one conversation's stream.
fn serve(mut stream: TcpStream, options: &Options, hub: &Hub) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    let request = match http::read(&stream) {
        Ok(request) => request,
        Err(e) => {
            let answer = http::error("bad_request", &e.to_string());
            return http::respond_json(&mut stream, "400 Bad Request", &answer);
        }
    };
    if request.method == "GET" {
        let name = request.path.trim_start_matches('/');
        return match options.webui.file(name) {
            Some((kind, body)) => http::respond(&mut stream, "200 OK", kind, &body),
            None => http::respond_json(
                &mut stream,
                "404 Not Found",
                &http::error("no_route", "no file of that name in the webui directory"),
            ),
        };
    }
    if request.method != "POST" {
        return http::respond_json(
            &mut stream,
            "405 Method Not Allowed",
            &http::error(
                "no_route",
                "the bridge takes GET for a file and POST for a frame",
            ),
        );
    }
    let Some(named) = request.head(route::HEADER) else {
        let answer = http::error(
            "unknown_adapter",
            "no X-Bridge-Adapter head names this request",
        );
        return http::respond_json(&mut stream, "403 Forbidden", &answer);
    };
    let Some(adapter) = options.manifest.adapter(named) else {
        let answer = http::error("unknown_adapter", "no adapter of that name in adapters.yml");
        return http::respond_json(&mut stream, "403 Forbidden", &answer);
    };
    if let Err(why) = route::admitted(adapter, peer) {
        let answer = http::error("remote_refused", why);
        return http::respond_json(&mut stream, "403 Forbidden", &answer);
    }
    let body: serde_json::Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(e) => {
            let answer = http::error("bad_request", &e.to_string());
            return http::respond_json(&mut stream, "400 Bad Request", &answer);
        }
    };
    let frame = match rpc::frame_of(adapter, &body) {
        Ok(frame) => frame,
        Err((status, answer)) => {
            return http::respond_json(&mut stream, &status_line(status), &answer);
        }
    };
    // A frame that follows a conversation turns this connection into that conversation's stream.
    if rpc::is_watch(&frame) {
        return follow(stream, adapter, hub, &frame);
    }
    // Following nothing is leaving, and leaving is closing the connection: this frame is no question.
    if frame.kind == "subscribe" && frame.name == "watch" {
        let answer = http::error(
            "bad_request",
            "a frame that follows a conversation names `session`; close the connection to stop",
        );
        return http::respond_json(&mut stream, "400 Bad Request", &answer);
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
            let answer = http::error("upstream_closed", &e);
            return http::respond_json(&mut stream, "502 Bad Gateway", &answer);
        }
    };
    match held.ask(frame) {
        Ok(answer) => http::respond_json(&mut stream, "200 OK", &rpc::answer(&answer)),
        Err(e) => {
            let answer = http::error("upstream_silent", &e);
            http::respond_json(&mut stream, "504 Gateway Timeout", &answer)
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
        let answer = http::error(
            "bad_request",
            "a frame that follows a conversation names `session`",
        );
        return http::respond_json(&mut stream, "400 Bad Request", &answer);
    };
    let held = match hub.get(session) {
        Ok(held) => held,
        Err(e) => {
            let answer = http::error("upstream_closed", &e);
            return http::respond_json(&mut stream, "502 Bad Gateway", &answer);
        }
    };
    let (tx, frames) = mpsc::channel();
    http::begin_stream(&mut stream)?;
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
            if http::event(&mut stream, &piece).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

// The adapters' own programs: started beside the door, stopped with it.
fn start_adapters(options: &Options) -> Vec<std::process::Child> {
    let mut out = Vec::new();
    for adapter in options.manifest.adapters() {
        let Some(line) = &adapter.command else {
            continue;
        };
        let launched = std::process::Command::new(shell())
            .arg(flag())
            // `exec` hands the process to the program itself, so the handle the bridge holds is that program.
            .arg(format!("exec {line}"))
            .stdin(std::process::Stdio::null())
            .spawn();
        match launched {
            Ok(child) => out.push(child),
            Err(e) => eprintln!("mypi: bridge cannot start {}: {e}", adapter.name),
        }
    }
    out
}

// A program the bridge started is ended with the door that started it.
fn end(mut child: std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

// The stop signals every front end answers to: the process leaves politely, with its programs.
fn wait_for_stop() {
    let seen = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
        for signal in [SIGINT, SIGTERM, SIGQUIT, SIGHUP] {
            let _ = signal_hook::flag::register(signal, Arc::clone(&seen));
        }
    }
    #[cfg(windows)]
    {
        use signal_hook::consts::{SIGBREAK, SIGINT, SIGTERM};
        for signal in [SIGINT, SIGTERM, SIGBREAK] {
            let _ = signal_hook::flag::register(signal, Arc::clone(&seen));
        }
    }
    while !seen.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn shell() -> &'static str {
    "/bin/sh"
}

#[cfg(windows)]
fn shell() -> &'static str {
    "cmd"
}

#[cfg(unix)]
fn flag() -> &'static str {
    "-c"
}

#[cfg(windows)]
fn flag() -> &'static str {
    "/C"
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
