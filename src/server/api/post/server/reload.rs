//! Reload: replace the config, but only while nothing runs.

use anyhow::Result;

use crate::server::api::driver::Driver;
use crate::server::config::Config;

pub(crate) fn apply(driver: &mut Driver) -> Result<()> {
    driver.cfg = Config::load()?;
    driver.reload_mcp();
    driver.sessions.invalidate();
    Ok(())
}
