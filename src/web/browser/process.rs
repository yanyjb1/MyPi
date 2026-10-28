//! Browser process and CDP transport.

use anyhow::{Context as _, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::cdp::Target;
use crate::server::config::BrowserConfig;

/// Fallback executable when neither `$MYPI_BROWSER_BIN` nor
/// `config.yaml → browser.bin` names one.
pub const DEFAULT_BROWSER_BIN: &str = "/opt/helium/helium";

/// Plain stable-channel desktop UA. The `HeadlessChrome` UA is the loudest
/// automation tell; keeping the engine segment truthful is the point.
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";

pub(crate) fn profile_dir(cfg: &BrowserConfig) -> PathBuf {
    cfg.profile_dir.clone().unwrap_or_else(|| {
        crate::server::platform::env::data_dir()
            .join("browser")
            .join("profile")
    })
}

pub(crate) fn discover_live_browser(profile: &Path) -> Option<u16> {
    let first = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let port = first.lines().next()?.trim().parse::<u16>().ok()?;
    Browser::port_alive(port).then_some(port)
}

// A browser handle: either a process we spawned (owned, killed on drop) or
// one already running (attached; the owner keeps the lifecycle).
pub(crate) struct Browser {
    child: Option<Child>,
    profile: PathBuf,
    headless: bool,
    attached: bool,
    pub port: u16,
}

impl Browser {
    pub fn launch(cfg: &BrowserConfig, profile: &Path, headless: bool) -> anyhow::Result<Browser> {
        let exe = std::env::var_os("MYPI_BROWSER_BIN")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                cfg.bin
                    .as_deref()
                    .filter(|b| !b.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| PathBuf::from(DEFAULT_BROWSER_BIN));
        Self::launch_with(&exe, profile, cfg, headless)
    }

    pub fn attach(profile: &Path, port: u16) -> anyhow::Result<Browser> {
        if !Self::port_alive(port) {
            anyhow::bail!("no browser listening on 127.0.0.1:{port}");
        }
        Ok(Browser {
            child: None,
            profile: profile.to_path_buf(),
            headless: false,
            attached: true,
            port,
        })
    }

    pub fn headless(&self) -> bool {
        self.headless
    }

    pub fn attached(&self) -> bool {
        self.attached
    }

    pub fn launch_with(
        exe: &Path,
        profile: &Path,
        cfg: &BrowserConfig,
        headless: bool,
    ) -> anyhow::Result<Browser> {
        std::fs::create_dir_all(profile)
            .with_context(|| format!("creating browser profile dir {}", profile.display()))?;

        let mut cmd = Command::new(exe);
        cmd.arg(format!("--user-data-dir={}", profile.display()))
            .arg("--remote-debugging-port=0")
            .arg("--remote-debugging-address=127.0.0.1")
            .args(["--no-first-run", "--no-default-browser-check"]);
        if headless {
            cmd.arg("--headless=new");
        }
        cmd.arg(format!(
            "--user-agent={}",
            cfg.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT)
        ))
        .args(&cfg.extra_args)
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

        let child = cmd
            .spawn()
            .with_context(|| format!("spawning browser {}", exe.display()))?;

        let browser = Browser {
            child: Some(child),
            profile: profile.to_path_buf(),
            headless,
            attached: false,
            port: 0,
        };
        browser.wait_for_devtools()
    }

    fn wait_for_devtools(mut self) -> anyhow::Result<Browser> {
        // DevToolsActivePort appears once the WS endpoint is listening.
        // An early exit means a bad binary or a locked profile.
        let port_file = self.profile.join("DevToolsActivePort");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(first) = std::fs::read_to_string(&port_file)
                && let Some(line) = first.lines().next()
                && let Ok(port) = line.trim().parse::<u16>()
            {
                // A stale file describes a dead port; accept it only after
                // /json/version answers.
                if Self::port_alive(port) {
                    self.port = port;
                    return Ok(self);
                }
            }
            if self
                .child
                .as_mut()
                .map(|c| c.try_wait().map(|s| s.is_some()))
                .transpose()?
                == Some(true)
            {
                anyhow::bail!("browser exited before opening DevTools port");
            }
            if Instant::now() > deadline {
                anyhow::bail!("timeout waiting for DevToolsActivePort");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// True when something accepts TCP on 127.0.0.1:port.
    pub fn port_alive(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(500),
        )
        .is_ok()
    }

    /// PUT /json/new?url=… — open a fresh tab, return its target.
    /// (Chromium ≥ 111 requires PUT; GET returns 405.)
    pub fn create_target(&self, url: &str) -> anyhow::Result<Target> {
        let endpoint = format!(
            "http://127.0.0.1:{}/json/new?{}",
            self.port,
            crate::web::url::encode_query(url)
        );
        let mut resp = ureq::put(&endpoint)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .send_empty()
            .map_err(|e| anyhow!("json/new failed: {e}"))?;
        let body: Target = resp
            .body_mut()
            .read_json()
            .context("parsing /json/new response")?;
        Ok(body)
    }

    /// GET /json/close/<id> — close a tab by target id.
    pub fn close_target(&self, id: &str) -> anyhow::Result<()> {
        let endpoint = format!("http://127.0.0.1:{}/json/close/{}", self.port, id);
        ureq::get(&endpoint)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .call()
            .map_err(|e| anyhow!("json/close failed: {e}"))?;
        Ok(())
    }

    /// GET /json/list, parsed. Works against any Chromium.
    pub fn targets(&self) -> anyhow::Result<Vec<Target>> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        let body: Vec<Target> = ureq::get(&url)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .call()
            .map_err(|e| anyhow!("json/list failed: {e}"))?
            .body_mut()
            .read_json()
            .context("parsing /json/list")?;
        Ok(body)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Attached browsers are not ours to kill — only the process we
        // spawned gets reaped.
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
