//! Watch: register the session's full stream.

use anyhow::Result;

use crate::ipc::contract::Viewer;
use crate::server::api::driver::Driver;

pub(crate) fn apply(driver: &Driver, viewer: &Viewer, session: i64) -> Result<()> {
    driver.watch(viewer, session)
}
