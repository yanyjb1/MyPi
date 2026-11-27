// Launch argv and the wait for a DevTools port.

use anyhow::{Context as _, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::platform::proc::{Origin, Process};

use super::super::devtools::Client;
use super::Browser;
use crate::server::config::BrowserOptions;

pub(super) fn launch(cfg: &BrowserOptions, profile: &Path) -> anyhow::Result<Browser> {
    let exe = executable(cfg)?;
    std::fs::create_dir_all(profile)
        .with_context(|| format!("creating browser profile dir {}", profile.display()))?;
    let mut cmd = command(&exe, profile, cfg);
    let process = Process::spawn(Origin::Browser, &mut cmd)
        .map_err(|e| anyhow!("spawning the browser: {e}"))?;
    match wait_for_devtools(profile, &process) {
        Ok(port) => Ok(Browser {
            process: Some(process),
            port,
            attached: false,
        }),
        Err(e) => {
            process.stop();
            Err(e)
        }
    }
}

fn executable(cfg: &BrowserOptions) -> anyhow::Result<PathBuf> {
    cfg.bin
        .as_deref()
        .filter(|b| !b.as_os_str().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("browser.bin is empty; set config.server.agent.tools.browser.bin"))
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

// DevToolsActivePort appears once the WS endpoint is listening; an early exit means a bad binary or a profile another process holds.
fn wait_for_devtools(profile: &Path, process: &Process) -> anyhow::Result<u16> {
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
        if process.try_wait()?.is_some() {
            anyhow::bail!("browser exited before opening DevTools port");
        }
        if Instant::now() > deadline {
            anyhow::bail!("timeout waiting for DevToolsActivePort");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
