// The bridge's loop: bind the door, take every request, start the adapters' own programs.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ipc::endpoint::Endpoint;

use crate::ui::bridge::Manifest;
use crate::ui::bridge::adapter::web;
use crate::ui::bridge::http::serve::serve;
use crate::ui::bridge::manager::Hub;

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
