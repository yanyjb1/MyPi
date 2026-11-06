//! One browser process or an attachment: the handle and its group.

mod launch;

use std::path::{Path, PathBuf};
use std::process::Child;

use super::devtools::Client;
use crate::server::config::BrowserOptions;

pub(crate) fn profile_dir(cfg: &BrowserOptions) -> PathBuf {
    cfg.profile_dir.clone().unwrap_or_else(|| {
        crate::server::platform::env::data_dir()
            .join("browser")
            .join("profile")
    })
}

pub(crate) fn discover_live_browser(profile: &Path) -> Option<u16> {
    let first = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let port = first.lines().next()?.trim().parse::<u16>().ok()?;
    Client::port_alive(port).then_some(port)
}

/// A browser: one we launched (owned) or one already running (attached).
pub(crate) struct Browser {
    child: Option<Child>,
    port: u16,
    attached: bool,
}

impl Browser {
    pub(crate) fn launch(cfg: &BrowserOptions, profile: &Path) -> anyhow::Result<Browser> {
        launch::launch(cfg, profile)
    }

    pub(crate) fn attach(port: u16) -> anyhow::Result<Browser> {
        if !Client::port_alive(port) {
            anyhow::bail!("no browser listening on 127.0.0.1:{port}");
        }
        Ok(Browser {
            child: None,
            port,
            attached: true,
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn attached(&self) -> bool {
        self.attached
    }

    /// True while the port answers and an owned process has not exited.
    pub(crate) fn alive(&mut self) -> bool {
        if self.attached {
            return Client::port_alive(self.port);
        }
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => Client::port_alive(self.port),
            _ => false,
        }
    }

    pub(crate) fn stop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            crate::server::platform::proc::terminate_tree(child);
        }
        self.child = None;
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.stop();
    }
}
