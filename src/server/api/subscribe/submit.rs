//! Submit: open a round and register the viewer on its turn.

use anyhow::Result;

use crate::ipc::contract::Viewer;
use crate::server::api::driver::Driver;

use super::Opened;

pub(crate) fn apply(
    driver: &mut Driver,
    viewer: &Viewer,
    session: i64,
    prefix: Option<&str>,
    text: &str,
    suffix: Option<&str>,
) -> Result<Opened> {
    driver.submit(viewer, session, prefix, text, suffix)
}
