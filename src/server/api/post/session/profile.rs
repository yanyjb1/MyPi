//! `/profile` — the prompt and tool roster the session runs.

use anyhow::Result;

use crate::server::api::driver::Driver;
use crate::server::config::read::profile::{self, ResolvedProfile};

pub(crate) fn apply(driver: &Driver, session: i64, name: String) -> Result<()> {
    crate::commands::Profile::resolve(&driver.cfg, &name)?;
    driver.sessions.choose_profile(&driver.cfg, session, &name)
}

impl crate::commands::Profile {
    pub fn resolve(cfg: &crate::server::config::Config, name: &str) -> Result<ResolvedProfile> {
        let known = profile::list(cfg);
        anyhow::ensure!(
            known.iter().any(|n| n == name),
            "unknown profile: {name}. Available: {}",
            known.join(", ")
        );
        profile::resolve(cfg, name)
    }
}
