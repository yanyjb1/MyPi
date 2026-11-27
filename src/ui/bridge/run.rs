// The bridge's loop: bind the door, take every request, start the adapters' own programs.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ipc::endpoint::Endpoint;

use crate::platform::proc::{Origin, Process};
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

    let programs = start_adapters(&options);

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
    for program in programs {
        program.stop();
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
fn start_adapters(options: &Options) -> Vec<Process> {
    let mut out = Vec::new();
    for adapter in options.manifest.adapters() {
        let Some(command) = &adapter.command else {
            continue;
        };
        let mut shell = Command::new(SHELL);
        for flag in FLAGS {
            shell.arg(flag);
        }
        shell.stdin(Stdio::null()).arg(command);
        match Process::spawn(Origin::Frontend, &mut shell) {
            Ok(program) => out.push(program),
            Err(e) => eprintln!("mypi: bridge cannot start {}: {e}", adapter.name),
        }
    }
    out
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

// The unix shell, and the one-line shape it takes: `exec` hands it over to the program, so the leader of the tree is the program itself.
#[cfg(unix)]
const SHELL: &str = "/bin/sh";

#[cfg(unix)]
const FLAGS: &[&str] = &["-c"];

// The windows shell: `-Command` takes the line whole, where `cmd`'s `/C` would take quotes out of it first.
#[cfg(windows)]
const SHELL: &str = "pwsh";

#[cfg(windows)]
const FLAGS: &[&str] = &["-NoProfile", "-NonInteractive", "-Command"];
