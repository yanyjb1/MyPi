//! Session writes: metadata, the destructive cuts, and the viewer's own watch.

use crate::ipc::contract::Viewer;
use crate::server::api::driver::Driver;

pub(crate) mod cwd;
pub(crate) mod delete;
pub(crate) mod interrupt;
pub(crate) mod name;
pub(crate) mod profile;
pub(crate) mod rewind;
pub(crate) mod switch;
pub(crate) mod todo;
pub(crate) mod unwatch;

pub(crate) fn name(driver: &mut Driver, session: i64, value: Option<String>) -> anyhow::Result<()> {
    name::apply(driver, session, value)
}

pub(crate) fn cwd(
    driver: &mut Driver,
    session: i64,
    raw: String,
) -> anyhow::Result<std::path::PathBuf> {
    cwd::apply(driver, session, raw)
}
