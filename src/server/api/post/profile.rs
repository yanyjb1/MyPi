//! `/profile` — which system prompt (and tool roster) the session runs. An unknown name is an error:
//! silently keeping the old prompt would be a lie.

use anyhow::Result;

use crate::server::config::Config;
use crate::server::config::read::profile::{self, ResolvedProfile};

impl crate::commands::Profile {
    /// Check `name` against the roster and resolve it.
    pub fn resolve(cfg: &Config, name: &str) -> Result<ResolvedProfile> {
        let known = profile::list(cfg);
        anyhow::ensure!(
            known.iter().any(|n| n == name),
            "unknown profile: {name}. Available: {}",
            known.join(", ")
        );
        profile::resolve(cfg, name)
    }
}
