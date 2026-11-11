//! Launch argv, the fork, and the wait for a DevTools port.

use anyhow::{Context as _, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::sync::mpsc::{Sender, SyncSender, channel, sync_channel};
use std::time::{Duration, Instant};

use super::super::devtools::Client;
use super::Browser;
use crate::server::config::BrowserOptions;

/// Fallback executable when neither `$MYPI_BROWSER_BIN` nor `config.yaml` names one.
const DEFAULT_BROWSER_BIN: &str = "/opt/helium/helium";

pub(super) fn launch(cfg: &BrowserOptions, profile: &Path) -> anyhow::Result<Browser> {
    std::fs::create_dir_all(profile)
        .with_context(|| format!("creating browser profile dir {}", profile.display()))?;
    let exe = executable(cfg);
    let mut cmd = command(&exe, profile, cfg);
    #[cfg(target_os = "linux")]
    crate::server::platform::proc::guard_orphan(&mut cmd);
    let mut child = fork(cmd)?;
    match wait_for_devtools(profile, &mut child) {
        Ok(port) => Ok(Browser {
            child: Some(child),
            port,
            attached: false,
        }),
        Err(e) => {
            crate::server::platform::proc::terminate_tree(&mut child);
            Err(e)
        }
    }
}

fn executable(cfg: &BrowserOptions) -> PathBuf {
    std::env::var_os("MYPI_BROWSER_BIN")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            cfg.bin
                .as_deref()
                .filter(|b| !b.as_os_str().is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BROWSER_BIN))
}

fn command(exe: &Path, profile: &Path, cfg: &BrowserOptions) -> Command {
    let mut cmd = Command::new(exe);
    cmd.args([
        &format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0",
        "--remote-debugging-address=127.0.0.1",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-blink-features=AutomationControlled",
    ]);
    if let Some(ua) = cfg.user_agent.as_deref() {
        cmd.arg(format!("--user-agent={ua}"));
    }
    cmd.args(&cfg.extra_args)
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// DevToolsActivePort appears once the WS endpoint is listening; an early exit
/// means a bad binary or a profile another process holds.
fn wait_for_devtools(profile: &Path, child: &mut Child) -> anyhow::Result<u16> {
    let port_file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(first) = std::fs::read_to_string(&port_file)
            && let Some(line) = first.lines().next()
            && let Ok(port) = line.trim().parse::<u16>()
            && Client::port_alive(port)
        {
            return Ok(port);
        }
        if child.try_wait()?.is_some() {
            anyhow::bail!("browser exited before opening DevTools port");
        }
        if Instant::now() > deadline {
            anyhow::bail!("timeout waiting for DevToolsActivePort");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Job {
    cmd: Command,
    reply: SyncSender<std::io::Result<Child>>,
}

/// Fork from a thread that never ends, so a parent-death signal binds the
/// browser to the server rather than to one connection.
fn fork(cmd: Command) -> anyhow::Result<Child> {
    static FORKER: LazyLock<Sender<Job>> = LazyLock::new(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("web-fork".to_string())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let mut cmd = job.cmd;
                    let _ = job
                        .reply
                        .send(crate::server::platform::proc::spawn_grouped(&mut cmd));
                }
            })
            .expect("cannot start the browser fork thread");
        tx
    });
    let (reply, answer) = sync_channel(1);
    FORKER
        .send(Job { cmd, reply })
        .expect("the fork thread outlives every caller");
    answer
        .recv()
        .expect("the fork thread answers every job")
        .map_err(|e| anyhow!("spawning the browser: {e}"))
}
